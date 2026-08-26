//! Conformance against leanSpec's fork-choice vectors (`fork_choice_test` format).
//!
//! Unlike the other suites, a case here is a **state machine**, not a single call. It opens
//! with a trusted anchor and then applies a list of steps — a block arriving, the clock
//! ticking, a vote or an aggregate reaching the node over gossip — carrying one store from
//! each step to the next.
//!
//! Every step, accepted or rejected, carries a `storeSnapshot`, and this replays against all
//! ten of its fields rather than the shallower `checks` block. leanSpec says why in
//! `StoreSnapshot`'s own docs: full block membership is what makes over- and under-pruning
//! observable, and the weights must agree "even where two clients agree on the head" — a
//! client whose weights are wrong but whose head happens to land right is the failure this
//! catches and `headSlot` does not. `checks` is asserted too, for what it adds on top:
//! symbolic block labels, the validator's attestation target, and the block body's own
//! contents.
//!
//! Two things the generator does are the harness's job here rather than the crate's, because
//! leanSpec puts them outside `Store`:
//!
//! - **Signature verification.** `verity-chain` splits it out of all three verifying entry
//!   points (see that module's docs), so this replays the pure halves. Two steps in the whole
//!   suite turn on a signature actually being wrong; they are listed in [`NEEDS_VERIFICATION`].
//! - **Seeding the counted pool from a block's own votes.** The generator hands its store the
//!   proofs the proposer had aggregated before it applies the block, so the snapshot expects
//!   them. A replaying client only ever sees the block, and recovers the same coverage from
//!   the body's aggregation bits — the proof bytes are not in the snapshot. `ethlambda`'s
//!   runner does the same thing at the same point.
//!
//! Source: leanSpec `tests/consensus/lstar/fork_choice/`, filled into the
//! `fixtures-prod-scheme.tar.gz` release asset that `crates/verity-types/fixtures.sha256`
//! pins. leanSpec `main` @ `0588c2d215a955a516378677a92db2a5666802f3`.

mod common;
mod fork_choice;

use std::collections::HashSet;

use common::bitlist;
use fork_choice::labels::Labels;
use fork_choice::shapes::{Case, Step};
use verity_chain::{
    AttestationSignature, RejectionReason, Store, hash_tree_root, on_block, on_tick,
    record_aggregated_payload, record_attestation_signature,
};
use verity_types::config::INTERVALS_PER_SLOT;
use verity_types::{
    AggregationBits, ByteList512KiB, Interval, SignedAggregatedAttestation, SingleMessageAggregate,
    ValidatorIndex,
};

/// Every suite under leanSpec's `fork_choice` fixture directory.
const SUITES: &[&str] = &[
    "test_ancestry_branches",
    "test_attestation_source_divergence",
    "test_attestation_target_selection",
    "test_block_attestation_limits",
    "test_block_future_horizon",
    "test_block_genesis_self_vote",
    "test_block_production",
    "test_block_unknown_parent",
    "test_checkpoint_sync",
    "test_checkpoint_sync_window",
    "test_duplicate_attestation_data",
    "test_early_block_arrival",
    "test_equivocation",
    "test_fallback_pool_set_cover",
    "test_finalization_mid_processing",
    "test_finalized_safety",
    "test_fork_choice_head",
    "test_fork_choice_reorgs",
    "test_gossip_aggregated_attestation_validation",
    "test_gossip_aggregated_empty_participants",
    "test_gossip_aggregated_registry_and_signature",
    "test_gossip_attestation_validation",
    "test_head_movement",
    "test_lexicographic_tiebreaker",
    "test_lmd_latest_message",
    "test_prune_finalized_orphaned_branch",
    "test_safe_target",
    "test_safe_target_supermajority",
    "test_signature_aggregation",
    "test_store_pruning",
    "test_tick_acceptance_branches",
    "test_tick_system",
];

/// Cases whose rejection is the signature itself being wrong.
///
/// `verity-chain` cannot produce `INVALID_SIGNATURE`: verification is the caller's, above
/// this crate (see `fork_choice`'s module docs). Both cases replay their earlier steps
/// normally; only the failing step is not asserted. They land with `verity-crypto`, which is
/// what supplies the missing check.
const NEEDS_VERIFICATION: &[&str] = &[
    "test_gossip_attestation_with_invalid_signature",
    "test_aggregated_attestation_proof_verification_failure_rejected",
];

/// Vectors whose recorded vote pools hold more than the wire ever carried.
///
/// The generator builds a block by aggregating the votes its proposer held, and merges that
/// pool into its store *before* applying the block. A replaying client only ever sees the
/// block, and a block body carries at most `MAX_ATTESTATIONS_DATA` distinct votes — in one of
/// these it carries none at all while the proposer's pool held three. Where the two diverge
/// the snapshot records local state that never reached the wire, and no client can reproduce
/// it. Everything else in those vectors is still asserted; see [`POOL_DERIVED`].
const GENERATOR_POOL_EXCEEDS_WIRE: &[&str] = &[
    "test_attestation_target_justifiable_constraint",
    "test_block_builder_fixed_point_advances_justification",
    "test_block_builder_recovers_finality_after_non_zero_boundary_stall",
    "test_produce_block_enforces_max_attestations_data_limit",
    "test_produce_block_includes_pending_attestations",
    "test_post_anchor_votes_can_finalize_above_anchor",
    "test_fork_above_finalized_wins_at_or_below_loses",
    "test_heavier_fork_below_finalized_slot_never_wins",
];

/// Vectors that turn on the aggregator duty at interval 2.
///
/// Folding a slot's pooled signatures into proofs is a cryptographic operation and belongs to
/// `verity-crypto` (see `fork_choice::timeline`). Its absence shows up in the pools alone —
/// the signatures it would have drained, and the proofs it would have produced. It lands with
/// that crate, and these become ordinary vectors then.
const NEEDS_AGGREGATION: &[&str] = &[
    "test_interval_2_aggregator_aggregates_raw_signatures",
    "test_aggregate_covers_union_of_priority_and_fallback_pools",
    "test_tick_interval_0_skips_acceptance_when_not_proposer",
];

/// The observables that follow from the vote pools, and so from what a client was told.
///
/// A vector on either list above is not asserted on these, and is asserted on everything
/// else: its head, checkpoints, block membership, clock, and block bodies all still have to
/// match. The safe target is here because it is weighed from the pending pool.
const POOL_DERIVED: &[&str] = &[
    "attestationSignatures",
    "attestationSignatureTargetSlots",
    "attestationChecks",
    "blockWeights",
    "knownAggregatedPayloads",
    "latestKnownAggregatedTargetSlots",
    "latestNewAggregatedTargetSlots",
    "newAggregatedPayloads",
    "newPoolProofParticipants",
    "safeTargetRoot",
    "safeTargetRootLabel",
    "safeTargetSlot",
];

#[test]
fn should_match_leanspec_fork_choice_vectors_when_fixtures_are_present() {
    let Some(root) = common::fixtures_dir() else {
        eprintln!("skipping: set VERITY_FIXTURES to run leanSpec fork-choice vectors");
        return;
    };

    let mut failures = Vec::new();
    let mut matched: HashSet<&str> = HashSet::new();
    let mut relaxed_and_diverged: HashSet<&str> = HashSet::new();
    let mut checked = 0usize;
    let mut steps = 0usize;
    for suite in SUITES {
        let files = common::collect_suite_json(&root, suite);
        assert!(
            !files.is_empty(),
            "no JSON under {} (expected **/{suite}/*.json)",
            root.display()
        );
        for (id, case) in common::read_cases::<Case>(&files) {
            let skip_last = listed(&id, NEEDS_VERIFICATION);
            if let Some(name) = skip_last {
                matched.insert(name);
            }
            let relaxed =
                listed(&id, GENERATOR_POOL_EXCEEDS_WIRE).or(listed(&id, NEEDS_AGGREGATION));
            if let Some(name) = relaxed {
                matched.insert(name);
            }
            checked += 1;
            steps += case.steps.len();
            match replay(&case, skip_last.is_some()) {
                Ok(()) => {}
                Err(problems) => {
                    let (kept, dropped) = split_relaxed(problems, relaxed.is_some());
                    if let (Some(name), true) = (relaxed, dropped) {
                        relaxed_and_diverged.insert(name);
                    }
                    if !kept.is_empty() {
                        failures.push(format!("{id}: {}", kept.join("\n  ")));
                    }
                }
            }
        }
    }

    for name in NEEDS_VERIFICATION
        .iter()
        .chain(GENERATOR_POOL_EXCEEDS_WIRE)
        .chain(NEEDS_AGGREGATION)
    {
        assert!(
            matched.contains(*name),
            "{name} matched no vector; leanSpec renamed or dropped it"
        );
    }
    // A listed vector that stopped diverging is one the list no longer has a reason to hold.
    for name in GENERATOR_POOL_EXCEEDS_WIRE.iter().chain(NEEDS_AGGREGATION) {
        assert!(
            relaxed_and_diverged.contains(*name),
            "{name} no longer diverges on its pools; take it off the list"
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} fork-choice vectors failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    let relaxed_count = GENERATOR_POOL_EXCEEDS_WIRE.len() + NEEDS_AGGREGATION.len();
    eprintln!(
        "fork choice: {checked} vectors matched, {steps} steps replayed, \
         {relaxed_count} not asserted on their vote pools"
    );
}

/// The listed test function the leanSpec test id names, if any.
fn listed(id: &str, names: &[&'static str]) -> Option<&'static str> {
    names.iter().copied().find(|name| id.contains(name))
}

/// Splits a vector's failures into those that stand and those [`POOL_DERIVED`] excuses.
fn split_relaxed(problems: Vec<String>, relaxed: bool) -> (Vec<String>, bool) {
    if !relaxed {
        return (problems, false);
    }
    let mut kept = Vec::new();
    let mut dropped = false;
    for problem in problems {
        let field = problem
            .rsplit("): ")
            .next()
            .unwrap_or(&problem)
            .split(['[', ':', '.'])
            .next()
            .unwrap_or_default();
        if POOL_DERIVED.contains(&field) {
            dropped = true;
        } else {
            kept.push(problem);
        }
    }
    (kept, dropped)
}

/// Runs one case: build the anchor store, then apply every step in order.
///
/// `skip_signature_step` drops the one step whose rejection this crate cannot produce; the
/// steps before it still run, since they build the store that step is posed against.
fn replay(case: &Case, skip_signature_step: bool) -> Result<(), Vec<String>> {
    let anchor_state = case.anchor_state.build().map_err(one)?;
    let anchor_block = case.anchor_block.build().map_err(one)?;
    let anchor = Store::new(&anchor_state, &anchor_block, Some(ValidatorIndex(0)));

    // A case may exist only to reject its own anchor, and then carries no steps at all.
    if let Some(expected) = &case.rejection_reason {
        return match anchor {
            Err(reason) if reason.as_str() == expected => Ok(()),
            Err(reason) => Err(one(format!(
                "anchor rejected as {reason}, expected {expected}"
            ))),
            Ok(_) => Err(one(format!(
                "anchor accepted, expected rejection {expected}"
            ))),
        };
    }
    let mut store = anchor.map_err(|reason| one(format!("anchor rejected as {reason}")))?;

    let mut labels = Labels::new(hash_tree_root(&anchor_block));
    let mut failures = Vec::new();

    for (index, step) in case.steps.iter().enumerate() {
        if skip_signature_step && step.rejection_reason.as_deref() == Some("INVALID_SIGNATURE") {
            continue;
        }
        let previous_head = store.head;
        let outcome = apply(&mut store, step, &mut labels).map_err(one)?;

        let mut step_failures = Vec::new();
        check_outcome(&mut step_failures, step, outcome);
        step.store_snapshot.check(&mut step_failures, &store);
        if let Some(checks) = &step.checks {
            checks
                .check(&mut step_failures, &store, step, &labels, previous_head)
                .map_err(one)?;
        }
        for failure in step_failures {
            failures.push(format!("step {index} ({}): {failure}", step.step_type));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// A single harness-level problem, in the shape the caller collects.
fn one(problem: String) -> Vec<String> {
    vec![problem]
}

/// What applying a step produced: nothing, or the reason it was refused.
type Outcome = Option<RejectionReason>;

/// Confirms a step was accepted or refused exactly as the vector says.
fn check_outcome(failures: &mut Vec<String>, step: &Step, outcome: Outcome) {
    match (step.valid, outcome) {
        (true, None) => {}
        (true, Some(reason)) => failures.push(format!("rejected as {reason}, expected accept")),
        (false, None) => {
            let expected = step.rejection_reason.as_deref().unwrap_or("a rejection");
            failures.push(format!("accepted, expected rejection {expected}"));
        }
        (false, Some(reason)) => {
            if let Some(expected) = step.rejection_reason.as_deref()
                && reason.as_str() != expected
            {
                failures.push(format!("rejected as {reason}, expected {expected}"));
            }
        }
    }
}

/// Applies one step to the store.
///
/// A `Err` return is a defect in the vector or in this harness; a rejection by the spec comes
/// back as `Ok(Some(reason))` and is the vector's business, not this function's.
fn apply(store: &mut Store, step: &Step, labels: &mut Labels) -> Result<Outcome, String> {
    match step.step_type.as_str() {
        "block" => apply_block(store, step, labels),
        "tick" => {
            on_tick(store, tick_target(store, step)?, step.has_proposal);
            Ok(None)
        }
        "attestation" => apply_attestation(store, step),
        "gossipAggregatedAttestation" => apply_aggregated(store, step),
        other => Err(format!("unknown step type {other}")),
    }
}

/// Ticks to the block's slot when the vector says to, imports it, then seeds its votes.
///
/// The seeding is the second of the two harness-side jobs described in the module docs: the
/// block's aggregation bits name exactly the validators the proposer's proofs covered, which
/// is all the snapshot records of them. `on_block` files each vote with no proof behind it,
/// per leanSpec; the entry below adds the coverage on top.
fn apply_block(store: &mut Store, step: &Step, labels: &mut Labels) -> Result<Outcome, String> {
    let json = step.block.as_ref().ok_or("block step carries no block")?;
    let block = json.build()?;

    if step.tick_to_slot {
        on_tick(store, Interval(block.slot.0 * INTERVALS_PER_SLOT), true);
    }
    if let Err(reason) = on_block(store, &block) {
        return Ok(Some(reason));
    }

    for attestation in block.body.attestations.iter() {
        store
            .latest_known_aggregated_payloads
            .entry(attestation.data)
            .or_default()
            .insert(SingleMessageAggregate {
                participants: attestation.aggregation_bits.clone(),
                proof: ByteList512KiB::default(),
            });
    }
    verity_chain::update_head(store);

    if let Some(label) = &json.block_root_label {
        labels.insert(label.clone(), hash_tree_root(&block));
    }
    Ok(None)
}

fn apply_attestation(store: &mut Store, step: &Step) -> Result<Outcome, String> {
    let json = step
        .attestation
        .as_ref()
        .ok_or("attestation step is empty")?;
    let data = json.data.build()?;
    let validator_index = json
        .validator_index
        .ok_or("attestation step carries no validatorIndex")?;
    let signature = json
        .signature
        .as_deref()
        .ok_or("attestation step carries no signature")?;

    // leanSpec validates and verifies for every node, and records only for an aggregator.
    // A non-aggregator's admission decision is the validation alone.
    let outcome = if step.is_aggregator {
        record_attestation_signature(
            store,
            ValidatorIndex(validator_index),
            data,
            AttestationSignature(common::unhex(signature)?),
        )
    } else {
        verity_chain::validate_attestation_signer(store, ValidatorIndex(validator_index), &data)
    };
    Ok(outcome.err())
}

fn apply_aggregated(store: &mut Store, step: &Step) -> Result<Outcome, String> {
    let json = step.attestation.as_ref().ok_or("aggregate step is empty")?;
    let proof = json
        .proof
        .as_ref()
        .ok_or("aggregate step carries no proof")?;

    let attestation = SignedAggregatedAttestation {
        data: json.data.build()?,
        proof: SingleMessageAggregate {
            participants: bitlist::<AggregationBits>(&proof.participants.data)?,
            proof: ByteList512KiB::try_from(common::unhex(&proof.proof.data)?)
                .map_err(|error| format!("proof bytes: {error:?}"))?,
        },
    };
    Ok(record_aggregated_payload(store, &attestation).err())
}

/// The interval a tick step advances to, from either the wire form it carries.
fn tick_target(store: &Store, step: &Step) -> Result<Interval, String> {
    if let Some(interval) = step.interval {
        return Ok(Interval(interval));
    }
    let time = step
        .time
        .ok_or("tick step carries neither time nor interval")?;
    let clock = verity_chain::SlotClock::new(store.config.genesis_time);
    Ok(clock.total_intervals(time * 1000))
}
