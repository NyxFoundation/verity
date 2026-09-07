//! Conformance against leanSpec's sync vectors.
//!
//! Source: leanSpec `tests/consensus/lstar/sync/`, filled into the
//! `fixtures-prod-scheme.tar.gz` release asset that `crates/verity-types/fixtures.sha256`
//! pins. The `sync_test` format is the only conformance coverage the sync layer has — the
//! state machine, batching, peer selection and scoring are all unspecified — and what it
//! pins is exactly one function: the structural verdict on a downloaded checkpoint state.
//!
//! The state arrives as SSZ hex rather than as a JSON object, which is the point: the vector
//! asks a client to decode the same bytes and reach the same verdict.

use std::fs;
use std::path::{Path, PathBuf};

use libssz::SszDecode;
use serde::Deserialize;
use verity_node::sync::checkpoint::verify_checkpoint_state;
use verity_types::State;

/// The leanSpec suites that emit `sync_test` vectors.
const SUITES: [&str; 2] = ["test_checkpoint_verify", "test_checkpoint_verify_advanced"];

#[derive(Debug, Deserialize)]
struct Case {
    operation: Operation,
    output: Output,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum Operation {
    VerifyCheckpoint {
        num_validators: u64,
        #[serde(default)]
        anchor_slot: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    /// The verdict `verify_checkpoint_state` must reproduce.
    valid: bool,
    /// The state the verdict was reached on, SSZ, `0x`-prefixed.
    state_bytes: String,
    validator_count: u64,
}

#[test]
fn should_match_leanspec_checkpoint_verification_vectors_when_fixtures_are_present() {
    let Some(root) = std::env::var_os("VERITY_FIXTURES").map(PathBuf::from) else {
        eprintln!("skipping: set VERITY_FIXTURES to run leanSpec sync vectors");
        return;
    };

    let files: Vec<PathBuf> = SUITES
        .iter()
        .flat_map(|suite| collect_suite_json(&root, suite))
        .collect();
    assert!(
        !files.is_empty(),
        "no JSON under {} (expected **/{{{}}}/*.json)",
        root.display(),
        SUITES.join(",")
    );

    let mut failures = Vec::new();
    let mut checked = 0usize;

    for path in &files {
        let text = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{}: read error: {error}", path.display()));
        let cases: std::collections::BTreeMap<String, Case> = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{}: json: {error}", path.display()));

        for (id, case) in cases {
            let bytes = unhex(&case.output.state_bytes)
                .unwrap_or_else(|error| panic!("{id}: state bytes: {error}"));
            let state = State::from_ssz_bytes(&bytes).unwrap_or_else(|error| {
                panic!("{id}: the vector's state does not decode: {error:?}")
            });

            // The vector echoes both inputs; checking them makes a mismatched or re-ordered
            // vector fail here rather than as an unexplained verdict disagreement.
            let Operation::VerifyCheckpoint {
                num_validators,
                anchor_slot,
            } = case.operation;
            assert_eq!(num_validators, case.output.validator_count, "{id}: inputs");
            assert_eq!(state.slot.0, anchor_slot, "{id}: anchor slot");
            assert_eq!(
                state.validators.len() as u64,
                case.output.validator_count,
                "{id}: registry size"
            );

            let verdict = verify_checkpoint_state(&state);
            if verdict != case.output.valid {
                failures.push(format!(
                    "{id}: got {verdict}, expected {}",
                    case.output.valid
                ));
            }
            checked += 1;
        }
    }

    assert!(
        failures.is_empty(),
        "{} case(s) disagree:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(checked > 0, "the suites held no cases");
}

/// Every `*.json` under a directory named `suite`, anywhere in the tree.
fn collect_suite_json(root: &Path, suite: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, suite, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, suite: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, suite, out);
            continue;
        }
        let is_json = path.extension().is_some_and(|ext| ext == "json");
        let in_suite = path.components().any(|part| part.as_os_str() == suite);
        if is_json && in_suite {
            out.push(path);
        }
    }
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    let body = text
        .strip_prefix("0x")
        .ok_or_else(|| format!("{text}: missing 0x prefix"))?;
    (0..body.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&body[index..index + 2], 16)
                .map_err(|error| format!("byte {index}: {error}"))
        })
        .collect()
}
