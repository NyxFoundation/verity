//! Conformance against leanSpec SSZ vectors (`ssz_test` format).
//!
//! Gated on `VERITY_FIXTURES` pointing at an extracted `fixtures-prod-scheme`
//! tree. The fast `cargo test` gate leaves this unset and the test returns.
//! CI's fixtures job always sets it, and fails if no vector matched.
//!
//! Source: leanSpec `tests/consensus/lstar/ssz/` filled into the
//! `fixtures-prod-scheme.tar.gz` release asset, sha256-pinned in
//! `crates/verity-types/fixtures.sha256`. leanSpec `main` @
//! `0588c2d215a955a516378677a92db2a5666802f3`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use libssz::{SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha2Hasher};
use serde::Deserialize;
use verity_types::GenesisConfig;
use verity_types::aggregation::{MultiMessageAggregate, SingleMessageAggregate};
use verity_types::attestation::{AggregatedAttestation, Attestation, SignedAggregatedAttestation};
use verity_types::block::{Block, BlockBody, BlockHeader, SignedBlock};
use verity_types::checkpoint::{AttestationData, Checkpoint};
use verity_types::primitives::{Bytes32, Bytes52, Interval, Slot, SubnetId, ValidatorIndex};
use verity_types::state::State;
use verity_types::validator::Validator;

/// Types whose signature field is an XMSS container. They land with `verity-crypto`.
const SKIPPED_TYPES: &[&str] = &["SignedAttestation"];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureCase {
    type_name: String,
    serialized: String,
    #[serde(default)]
    root: String,
    #[serde(default)]
    rejection_reason: Option<String>,
}

enum Outcome {
    Matched,
    Skipped,
}

#[test]
fn should_match_leanspec_ssz_vectors_when_fixtures_are_present() {
    let Some(root) = fixtures_dir() else {
        eprintln!("skipping: set VERITY_FIXTURES to run leanSpec SSZ vectors");
        return;
    };
    assert!(
        root.is_dir(),
        "VERITY_FIXTURES is set but {} is not a directory",
        root.display()
    );

    let mut files = Vec::new();
    collect_ssz_json(&root, &mut files);
    assert!(
        !files.is_empty(),
        "no SSZ JSON under {} (expected **/test_consensus_containers/*.json)",
        root.display()
    );

    let mut matched = 0usize;
    let mut skipped = 0usize;
    let mut failures = Vec::new();
    for path in &files {
        run_file(path, &mut matched, &mut skipped, &mut failures);
    }

    eprintln!("matched {matched} leanSpec SSZ vectors, skipped {skipped}");
    assert!(
        failures.is_empty(),
        "{} SSZ vector(s) disagreed with leanSpec:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(matched > 0, "no SSZ vector matched; {skipped} skipped");
}

fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("VERITY_FIXTURES").map(PathBuf::from)
}

fn collect_ssz_json(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ssz_json(&path, out);
            continue;
        }
        let is_json = path.extension().is_some_and(|ext| ext == "json");
        let is_container_suite = path
            .components()
            .any(|c| c.as_os_str() == "test_consensus_containers");
        if is_json && is_container_suite {
            out.push(path);
        }
    }
}

fn run_file(path: &Path, matched: &mut usize, skipped: &mut usize, failures: &mut Vec<String>) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            failures.push(format!("{}: read error: {error}", path.display()));
            return;
        }
    };
    let cases: BTreeMap<String, FixtureCase> = match serde_json::from_str(&text) {
        Ok(cases) => cases,
        Err(error) => {
            failures.push(format!("{}: json: {error}", path.display()));
            return;
        }
    };
    for (id, case) in cases {
        match run_case(&case) {
            Ok(Outcome::Matched) => *matched += 1,
            Ok(Outcome::Skipped) => *skipped += 1,
            Err(error) => failures.push(format!("{} ({id}): {error}", path.display())),
        }
    }
}

fn run_case(case: &FixtureCase) -> Result<Outcome, String> {
    let bytes = from_hex(&case.serialized)?;
    let reject = case.rejection_reason.is_some() || case.root.is_empty();
    let root = if reject {
        Vec::new()
    } else {
        from_hex(&case.root)?
    };
    dispatch(&case.type_name, &bytes, &root, reject)
}

fn dispatch(type_name: &str, bytes: &[u8], root: &[u8], reject: bool) -> Result<Outcome, String> {
    if SKIPPED_TYPES.contains(&type_name) {
        return Ok(Outcome::Skipped);
    }
    match type_name {
        "Checkpoint" => apply::<Checkpoint>(bytes, root, reject),
        "AttestationData" => apply::<AttestationData>(bytes, root, reject),
        "Attestation" => apply::<Attestation>(bytes, root, reject),
        "AggregatedAttestation" => apply::<AggregatedAttestation>(bytes, root, reject),
        "SignedAggregatedAttestation" => apply::<SignedAggregatedAttestation>(bytes, root, reject),
        "BlockBody" => apply::<BlockBody>(bytes, root, reject),
        "BlockHeader" => apply::<BlockHeader>(bytes, root, reject),
        "Block" => apply::<Block>(bytes, root, reject),
        "SignedBlock" => apply::<SignedBlock>(bytes, root, reject),
        "Config" | "GenesisConfig" => apply::<GenesisConfig>(bytes, root, reject),
        "Validator" => apply::<Validator>(bytes, root, reject),
        "State" => apply::<State>(bytes, root, reject),
        "SingleMessageAggregate" => apply::<SingleMessageAggregate>(bytes, root, reject),
        "MultiMessageAggregate" => apply::<MultiMessageAggregate>(bytes, root, reject),
        "Bytes32" => apply::<Bytes32>(bytes, root, reject),
        "Bytes52" => apply::<Bytes52>(bytes, root, reject),
        "Slot" => apply::<Slot>(bytes, root, reject),
        "ValidatorIndex" => apply::<ValidatorIndex>(bytes, root, reject),
        "SubnetId" => apply::<SubnetId>(bytes, root, reject),
        "Interval" => apply::<Interval>(bytes, root, reject),
        "Boolean" => apply::<bool>(bytes, root, reject),
        "Uint64" => apply::<u64>(bytes, root, reject),
        other => Err(format!("unhandled SSZ type {other}")),
    }
}

fn apply<T>(bytes: &[u8], root: &[u8], reject: bool) -> Result<Outcome, String>
where
    T: SszDecode + SszEncode + HashTreeRoot + PartialEq + std::fmt::Debug,
{
    if reject {
        check_rejection::<T>(bytes)?;
    } else {
        check_roundtrip::<T>(bytes, root)?;
    }
    Ok(Outcome::Matched)
}

fn check_roundtrip<T>(bytes: &[u8], expected_root: &[u8]) -> Result<(), String>
where
    T: SszDecode + SszEncode + HashTreeRoot + PartialEq + std::fmt::Debug,
{
    let decoded = T::from_ssz_bytes(bytes).map_err(|error| format!("decode: {error:?}"))?;
    let encoded = decoded.to_ssz();
    if encoded != bytes {
        return Err(format!(
            "encode mismatch: got {} bytes, want {}",
            encoded.len(),
            bytes.len()
        ));
    }
    let computed = decoded.hash_tree_root(&Sha2Hasher);
    if computed.as_slice() != expected_root {
        return Err(format!(
            "root mismatch: got 0x{}, want 0x{}",
            hex_encode(&computed),
            hex_encode(expected_root)
        ));
    }
    Ok(())
}

fn check_rejection<T: SszDecode>(bytes: &[u8]) -> Result<(), String> {
    match T::from_ssz_bytes(bytes) {
        Err(_) => Ok(()),
        Ok(_) => Err("decode succeeded; leanSpec expects rejection".into()),
    }
}

fn from_hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.strip_prefix("0x").unwrap_or(text);
    if !text.len().is_multiple_of(2) {
        return Err(format!("odd-length hex ({})", text.len()));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| format!("invalid hex at {i}")))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
