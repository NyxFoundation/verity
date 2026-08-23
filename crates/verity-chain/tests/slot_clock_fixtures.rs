//! Conformance against leanSpec's slot-clock vectors.
//!
//! Source: leanSpec `tests/consensus/lstar/chain/test_slot_clock.py`, filled into the
//! `fixtures-prod-scheme.tar.gz` release asset that `crates/verity-types/fixtures.sha256`
//! pins.

mod common;

use serde::Deserialize;
use verity_chain::{SlotClock, intervals_at_slot_start};
use verity_types::Slot;
use verity_types::config::{
    INTERVALS_PER_SLOT, MILLISECONDS_PER_INTERVAL, MILLISECONDS_PER_SLOT, SECONDS_PER_SLOT,
};

const SUITE: &str = "test_slot_clock";

#[derive(Debug, Deserialize)]
struct Case {
    operation: Operation,
    config: Config,
    output: Output,
}

/// The timing constants the vector was generated under.
///
/// Checked rather than ignored: the constants live in `verity-types` as compile-time values,
/// so a spec change to slot timing would otherwise surface as a pile of arithmetic mismatches
/// instead of the one fact that actually moved.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    seconds_per_slot: u64,
    intervals_per_slot: u64,
    milliseconds_per_interval: u64,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum Operation {
    FromUnixTime {
        genesis_time: u64,
        unix_seconds: f64,
    },
    TotalIntervals {
        genesis_time: u64,
        current_time_milliseconds: f64,
    },
    CurrentSlot {
        genesis_time: u64,
        current_time_milliseconds: f64,
    },
    CurrentInterval {
        genesis_time: u64,
        current_time_milliseconds: f64,
    },
    FromSlot {
        slot: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Output {
    #[serde(default)]
    slot: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    total_intervals: Option<u64>,
}

#[test]
fn should_match_leanspec_slot_clock_vectors_when_fixtures_are_present() {
    let Some(root) = common::fixtures_dir() else {
        eprintln!("skipping: set VERITY_FIXTURES to run leanSpec slot-clock vectors");
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

    eprintln!("matched {} leanSpec slot-clock vectors", cases.len());
    assert!(
        failures.is_empty(),
        "{} slot-clock vector(s) disagreed with leanSpec:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(!cases.is_empty(), "no slot-clock vector matched");
}

fn check(case: &Case) -> Result<(), String> {
    check_config(&case.config)?;
    match case.operation {
        // A Unix instant in seconds counts the same intervals as the millisecond form; the
        // spec reaches both through one accessor over a common millisecond time base.
        Operation::FromUnixTime {
            genesis_time,
            unix_seconds,
        } => expect(
            "interval",
            SlotClock::new(genesis_time)
                .total_intervals(to_milliseconds(unix_seconds * 1000.0))
                .0,
            case.output.interval,
        ),
        Operation::TotalIntervals {
            genesis_time,
            current_time_milliseconds,
        } => expect(
            "totalIntervals",
            SlotClock::new(genesis_time)
                .total_intervals(to_milliseconds(current_time_milliseconds))
                .0,
            case.output.total_intervals,
        ),
        Operation::CurrentSlot {
            genesis_time,
            current_time_milliseconds,
        } => expect(
            "slot",
            SlotClock::new(genesis_time)
                .current_slot(to_milliseconds(current_time_milliseconds))
                .0,
            case.output.slot,
        ),
        Operation::CurrentInterval {
            genesis_time,
            current_time_milliseconds,
        } => expect(
            "interval",
            SlotClock::new(genesis_time)
                .current_interval(to_milliseconds(current_time_milliseconds))
                .0,
            case.output.interval,
        ),
        Operation::FromSlot { slot } => expect(
            "interval",
            intervals_at_slot_start(Slot(slot)).0,
            case.output.interval,
        ),
    }
}

fn check_config(config: &Config) -> Result<(), String> {
    let expected = [
        ("secondsPerSlot", config.seconds_per_slot, SECONDS_PER_SLOT),
        (
            "intervalsPerSlot",
            config.intervals_per_slot,
            INTERVALS_PER_SLOT,
        ),
        (
            "millisecondsPerInterval",
            config.milliseconds_per_interval,
            MILLISECONDS_PER_INTERVAL,
        ),
    ];
    for (name, fixture, ours) in expected {
        if fixture != ours {
            return Err(format!(
                "{name}: fixture generated with {fixture}, this build uses {ours}"
            ));
        }
    }
    // Not a fixture field, but the relation the other three rest on.
    if MILLISECONDS_PER_SLOT != config.seconds_per_slot * 1000 {
        return Err(format!(
            "MILLISECONDS_PER_SLOT {MILLISECONDS_PER_SLOT} disagrees with secondsPerSlot {}",
            config.seconds_per_slot
        ));
    }
    Ok(())
}

fn expect(field: &str, got: u64, want: Option<u64>) -> Result<(), String> {
    let want = want.ok_or_else(|| format!("vector carries no `{field}` output"))?;
    if got == want {
        return Ok(());
    }
    Err(format!("{field}: got {got}, want {want}"))
}

/// leanSpec reads its time source as a float and truncates to whole milliseconds.
fn to_milliseconds(value: f64) -> u64 {
    value.trunc().max(0.0) as u64
}
