//! Conformance against leanSpec's state-transition vectors (`state_transition_test` format).
//!
//! Every case is a pre-state and a list of blocks. An accepting case carries `postStateRoot`,
//! which pins the entire post-state in one comparison, plus a `post` block of partial
//! assertions the Python harness uses for readable failures — those are checked too, so a
//! mismatch says which field moved rather than only that a root differs. A rejecting case
//! carries `rejectionReason` and no post-state.
//!
//! The JSON containers this reads are mirrored in `common` rather than derived on the
//! `verity-types` shapes — see that module for why.
//!
//! Source: leanSpec `tests/consensus/lstar/state_transition/`, filled into the
//! `fixtures-prod-scheme.tar.gz` release asset that `crates/verity-types/fixtures.sha256`
//! pins. leanSpec `main` @ `0588c2d215a955a516378677a92db2a5666802f3`.

mod common;

use common::{BlockJson, DataList, StateJson, ValidatorJson, compare, flags, hex};
use serde::Deserialize;
use verity_chain::{generate_genesis, hash_tree_root, process_block, state_transition};
use verity_types::State;

/// Every suite under leanSpec's `state_transition` fixture directory.
const SUITES: &[&str] = &[
    "test_aggregation_bits",
    "test_attestation_chain_binding",
    "test_attestation_data_limits",
    "test_block_processing",
    "test_empty_validator_registry",
    "test_finalization",
    "test_genesis",
    "test_justification",
    "test_justification_accounting",
    "test_justification_votes_length_mismatch",
    "test_justified_slot_out_of_range",
    "test_skipped_slot_history",
    "test_slot_monotonicity",
    "test_small_validator_quorums",
    "test_zero_hash_justification_root",
];

/// Cases the generator applied with `process_block` rather than `state_transition`.
///
/// leanSpec's filler picks the entry point from `BlockSpec.skip_slot_processing`, which it
/// does not serialize — see `packages/testing/src/consensus_testing/test_fixtures/`
/// `state_transition.py`, the block loop's `elif`. Both cases here exercise a guard inside
/// header validation that `process_slots` makes unreachable from the transition's own entry
/// point, so the entry point has to be carried here. It cannot be inferred from the vector:
/// a zero `stateRoot` marks the failing block of any rejection case, not just these.
const PROCESS_BLOCK_ONLY: &[&str] = &[
    "test_block_with_wrong_slot",
    "test_block_at_parent_slot_rejected_when_slot_processing_skipped",
];

/// Cases that carry no block at all, and so cannot be replayed by any client.
///
/// The filler builds a block before applying it, and a rejection raised during construction
/// is recorded with an empty `blocks` list. What this one records — proposer selection
/// against an empty registry — is covered by a unit test on `proposer_for_slot` instead.
const NO_BLOCK_TO_REPLAY: &[&str] = &["test_proposer_scheduling_on_empty_registry_rejected"];

/// Which entry point a case is replayed through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// The full transition, including slot advancement and the post-state-root commitment.
    Transition,
    /// Header and body only, against a state the vector already advanced.
    BlockOnly,
}

#[test]
fn should_match_leanspec_state_transition_vectors_when_fixtures_are_present() {
    let Some(root) = common::fixtures_dir() else {
        eprintln!("skipping: set VERITY_FIXTURES to run leanSpec state-transition vectors");
        return;
    };

    let mut failures = Vec::new();
    let mut matched: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut checked = 0usize;
    let mut skipped = 0usize;
    for suite in SUITES {
        let files = common::collect_suite_json(&root, suite);
        assert!(
            !files.is_empty(),
            "no JSON under {} (expected **/{suite}/*.json)",
            root.display()
        );
        for (id, case) in common::read_cases::<Case>(&files) {
            if let Some(name) = listed(&id, NO_BLOCK_TO_REPLAY) {
                matched.insert(name);
                skipped += 1;
                continue;
            }
            checked += 1;
            let entry = match listed(&id, PROCESS_BLOCK_ONLY) {
                Some(name) => {
                    matched.insert(name);
                    Entry::BlockOnly
                }
                None => Entry::Transition,
            };
            if let Err(error) = check(&case, entry) {
                failures.push(format!("{id}: {error}"));
            }
        }
    }

    // A name that stopped matching is a rename upstream, not a case that quietly passes.
    for name in PROCESS_BLOCK_ONLY.iter().chain(NO_BLOCK_TO_REPLAY) {
        assert!(
            matched.contains(*name),
            "{name} matched no vector; leanSpec renamed or dropped it"
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} state-transition vectors failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    eprintln!("state transition: {checked} vectors matched, {skipped} carried no block");
}

/// The listed test function the leanSpec test id names, if any.
fn listed(id: &str, names: &[&'static str]) -> Option<&'static str> {
    names.iter().copied().find(|name| id.contains(name))
}

/// Runs one case: apply every block in order, then check what the case asserts.
fn check(case: &Case, entry: Entry) -> Result<(), String> {
    let mut state = case.pre.build()?;
    check_generated_genesis(&state)?;

    for (index, block) in case.blocks.iter().enumerate() {
        let block = block.build()?;
        let applied = match entry {
            Entry::Transition => state_transition(&state, &block),
            Entry::BlockOnly => process_block(&state, &block),
        };
        match applied {
            Ok(post) => state = post,
            Err(reason) => {
                let expected = case.rejection_reason.as_deref().ok_or_else(|| {
                    format!("block {index} rejected as {reason}, expected accept")
                })?;
                if reason.as_str() != expected {
                    return Err(format!(
                        "block {index} rejected as {reason}, expected {expected}"
                    ));
                }
                return Ok(());
            }
        }
    }

    if let Some(expected) = &case.rejection_reason {
        return Err(format!(
            "every block applied, expected rejection {expected}"
        ));
    }
    check_post_state_root(case, &state)?;
    case.post
        .as_ref()
        .map_or(Ok(()), |assertions| assertions.check(&state))
}

/// Cross-checks [`generate_genesis`] against any pre-state that is one.
///
/// No vector calls the generator directly, but 68 of them start from its output. A pre-state
/// with an empty history at slot 0 is a genesis state, and everything the generator has to get
/// right beyond its two arguments — the zero checkpoints, and the empty body's root in the
/// genesis header — is asserted here against a value leanSpec produced.
fn check_generated_genesis(pre: &State) -> Result<(), String> {
    let untouched = pre.slot.0 == 0
        && pre.historical_block_hashes.is_empty()
        && pre.justified_slots.is_empty()
        && pre.justifications_roots.is_empty()
        && pre.justifications_validators.is_empty();
    if !untouched {
        return Ok(());
    }

    let generated = generate_genesis(pre.config.genesis_time, pre.validators.clone());
    if generated == *pre {
        return Ok(());
    }
    Err(format!(
        "generate_genesis root {}, expected {}",
        hex(&hash_tree_root(&generated)),
        hex(&hash_tree_root(pre))
    ))
}

fn check_post_state_root(case: &Case, state: &State) -> Result<(), String> {
    let expected = case
        .post_state_root
        .as_deref()
        .ok_or("accepting case carries no postStateRoot")?;
    let actual = hex(&hash_tree_root(state));
    if actual == expected {
        return Ok(());
    }
    Err(format!("post state root {actual}, expected {expected}"))
}

// ---------------------------------------------------------------------------------------
// Fixture shapes
// ---------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    #[allow(dead_code)]
    network: String,
    #[allow(dead_code)]
    lean_env: String,
    #[allow(dead_code)]
    proof_setting: u8,
    pre: StateJson,
    blocks: Vec<BlockJson>,
    #[serde(default)]
    post: Option<PostAssertions>,
    #[serde(default)]
    post_state_root: Option<String>,
    #[serde(default)]
    rejection_reason: Option<String>,
    #[serde(rename = "_info")]
    #[allow(dead_code)]
    info: serde_json::Value,
}

// ---------------------------------------------------------------------------------------
// Partial post-state assertions
// ---------------------------------------------------------------------------------------

/// The `post` block a case may carry.
///
/// `deny_unknown_fields` is what keeps this honest: a field leanSpec adds fails the run
/// instead of being skipped. The three `*Label` fields are the deliberate exception — they
/// carry symbolic block names such as `"block_2"`, resolvable only against the generator's own
/// labelling, and `postStateRoot` already pins every value they describe.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct PostAssertions {
    slot: Option<u64>,
    config_genesis_time: Option<u64>,
    latest_justified_slot: Option<u64>,
    latest_justified_root: Option<String>,
    latest_finalized_slot: Option<u64>,
    latest_finalized_root: Option<String>,
    latest_block_header_slot: Option<u64>,
    latest_block_header_proposer_index: Option<u64>,
    latest_block_header_parent_root: Option<String>,
    latest_block_header_state_root: Option<String>,
    latest_block_header_body_root: Option<String>,
    historical_block_hashes: Option<DataList<String>>,
    historical_block_hashes_count: Option<usize>,
    justified_slots: Option<DataList<bool>>,
    justifications_roots: Option<DataList<String>>,
    justifications_roots_count: Option<usize>,
    justifications_validators: Option<DataList<bool>>,
    justifications_validators_count: Option<usize>,
    validators: Option<DataList<ValidatorJson>>,
    validator_count: Option<usize>,
    latest_justified_root_label: Option<String>,
    latest_finalized_root_label: Option<String>,
    justifications_roots_labels: Option<Vec<String>>,
}

impl PostAssertions {
    fn check(&self, state: &State) -> Result<(), String> {
        let mut failures = Vec::new();
        self.check_scalars(state, &mut failures);
        self.check_roots(state, &mut failures);
        self.check_collections(state, &mut failures);
        if failures.is_empty() {
            return Ok(());
        }
        Err(failures.join("; "))
    }

    fn check_scalars(&self, state: &State, failures: &mut Vec<String>) {
        let header = &state.latest_block_header;
        compare(&mut *failures, "slot", self.slot, Some(state.slot.0));
        compare(
            failures,
            "configGenesisTime",
            self.config_genesis_time,
            Some(state.config.genesis_time),
        );
        compare(
            failures,
            "latestJustifiedSlot",
            self.latest_justified_slot,
            Some(state.latest_justified.slot.0),
        );
        compare(
            failures,
            "latestFinalizedSlot",
            self.latest_finalized_slot,
            Some(state.latest_finalized.slot.0),
        );
        compare(
            failures,
            "latestBlockHeaderSlot",
            self.latest_block_header_slot,
            Some(header.slot.0),
        );
        compare(
            failures,
            "latestBlockHeaderProposerIndex",
            self.latest_block_header_proposer_index,
            Some(header.proposer_index.0),
        );
    }

    fn check_roots(&self, state: &State, failures: &mut Vec<String>) {
        let header = &state.latest_block_header;
        for (name, expected, actual) in [
            (
                "latestJustifiedRoot",
                &self.latest_justified_root,
                state.latest_justified.root,
            ),
            (
                "latestFinalizedRoot",
                &self.latest_finalized_root,
                state.latest_finalized.root,
            ),
            (
                "latestBlockHeaderParentRoot",
                &self.latest_block_header_parent_root,
                header.parent_root,
            ),
            (
                "latestBlockHeaderStateRoot",
                &self.latest_block_header_state_root,
                header.state_root,
            ),
            (
                "latestBlockHeaderBodyRoot",
                &self.latest_block_header_body_root,
                header.body_root,
            ),
        ] {
            compare(failures, name, expected.clone(), Some(hex(&actual)));
        }
    }

    fn check_collections(&self, state: &State, failures: &mut Vec<String>) {
        let hashes: Vec<String> = state
            .historical_block_hashes
            .iter()
            .map(|root| hex(root))
            .collect();
        compare(
            failures,
            "historicalBlockHashes",
            self.historical_block_hashes.as_ref().map(|list| &list.data),
            Some(&hashes),
        );
        compare(
            failures,
            "historicalBlockHashesCount",
            self.historical_block_hashes_count,
            Some(state.historical_block_hashes.len()),
        );

        let tracked: Vec<String> = state
            .justifications_roots
            .iter()
            .map(|root| hex(root))
            .collect();
        compare(
            failures,
            "justificationsRoots",
            self.justifications_roots.as_ref().map(|list| &list.data),
            Some(&tracked),
        );
        compare(
            failures,
            "justificationsRootsCount",
            self.justifications_roots_count,
            Some(state.justifications_roots.len()),
        );
        compare(
            failures,
            "justificationsValidatorsCount",
            self.justifications_validators_count,
            Some(state.justifications_validators.len()),
        );
        compare(
            failures,
            "justifiedSlots",
            self.justified_slots.as_ref().map(|list| &list.data),
            Some(&flags(state.justified_slots.len(), |index| {
                state.justified_slots.get(index)
            })),
        );
        compare(
            failures,
            "justificationsValidators",
            self.justifications_validators
                .as_ref()
                .map(|list| &list.data),
            Some(&flags(state.justifications_validators.len(), |index| {
                state.justifications_validators.get(index)
            })),
        );
        compare(
            failures,
            "validatorCount",
            self.validator_count,
            Some(state.validators.len()),
        );
        if let Some(expected) = &self.validators {
            let actual: Vec<(String, String, u64)> = state
                .validators
                .iter()
                .map(|validator| {
                    (
                        hex(&validator.attestation_public_key),
                        hex(&validator.proposal_public_key),
                        validator.index.0,
                    )
                })
                .collect();
            let wanted: Vec<(String, String, u64)> = expected
                .data
                .iter()
                .map(|validator| {
                    (
                        validator.attestation_public_key.clone(),
                        validator.proposal_public_key.clone(),
                        validator.index,
                    )
                })
                .collect();
            compare(failures, "validators", Some(wanted), Some(actual));
        }
    }
}
