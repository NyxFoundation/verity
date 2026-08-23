//! Conformance against leanSpec's justifiability vectors (`justifiability_test` format).
//!
//! Source: leanSpec `tests/consensus/lstar/state_transition/test_justifiability.py`, filled
//! into the `fixtures-prod-scheme.tar.gz` release asset that
//! `crates/verity-types/fixtures.sha256` pins.

mod common;

use serde::Deserialize;
use verity_chain::{is_justifiable_after, justified_index_after};
use verity_types::Slot;

const SUITE: &str = "test_justifiability";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    slot: u64,
    finalized_slot: u64,
    output: Output,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Output {
    /// Signed on purpose: leanSpec emits a negative delta for a slot behind the boundary.
    delta: i64,
    is_justifiable: bool,
}

#[test]
fn should_match_leanspec_justifiability_vectors_when_fixtures_are_present() {
    let Some(root) = common::fixtures_dir() else {
        eprintln!("skipping: set VERITY_FIXTURES to run leanSpec justifiability vectors");
        return;
    };
    let files = common::collect_suite_json(&root, SUITE);
    assert!(
        !files.is_empty(),
        "no JSON under {} (expected **/{SUITE}/*.json)",
        root.display()
    );

    let cases: Vec<(String, Case)> = common::read_cases(&files);
    let mut failures = Vec::new();
    for (id, case) in &cases {
        if let Err(error) = check(case) {
            failures.push(format!("{id}: {error}"));
        }
    }

    eprintln!("matched {} leanSpec justifiability vectors", cases.len());
    assert!(
        failures.is_empty(),
        "{} justifiability vector(s) disagreed with leanSpec:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(!cases.is_empty(), "no justifiability vector matched");
}

fn check(case: &Case) -> Result<(), String> {
    // The fixture's own delta is checked first. It is the spec's statement of how far apart
    // the two slots are, so a disagreement here means the vector was read wrong and every
    // verdict below it would be judged against the wrong question.
    let delta = i128::from(case.slot) - i128::from(case.finalized_slot);
    if delta != i128::from(case.output.delta) {
        return Err(format!(
            "delta mismatch: computed {delta}, fixture says {}",
            case.output.delta
        ));
    }

    let slot = Slot(case.slot);
    let finalized = Slot(case.finalized_slot);

    let justifiable = is_justifiable_after(slot, finalized);
    if justifiable != case.output.is_justifiable {
        return Err(format!(
            "slot {} after finalized {}: got is_justifiable={justifiable}, want {}",
            case.slot, case.finalized_slot, case.output.is_justifiable
        ));
    }

    // The bitfield index is not part of this fixture format, but it is defined over the same
    // pair and must stay consistent with the delta the vector states.
    let expected_index = (delta > 0).then(|| (delta - 1) as usize);
    let index = justified_index_after(slot, finalized);
    if index != expected_index {
        return Err(format!(
            "slot {} after finalized {}: got index {index:?}, want {expected_index:?}",
            case.slot, case.finalized_slot
        ));
    }

    Ok(())
}
