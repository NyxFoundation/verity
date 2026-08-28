//! Conformance against leanSpec's SSZ vectors for the XMSS containers.
//!
//! `verity-types` runs the same suites and skips exactly these five type names, because they
//! carry key material or a raw XMSS signature and are defined here instead. This harness is
//! the other half of that split: it claims those five and skips everything `verity-types`
//! already covers, so between the two runs no vector goes unchecked.
//!
//! Gated on `VERITY_FIXTURES` pointing at an extracted `fixtures-prod-scheme` tree. The fast
//! `cargo test` gate leaves it unset and this returns; CI's fixtures job always sets it, and
//! fails if no vector matched.
//!
//! Source: leanSpec `tests/consensus/lstar/ssz/`, filled into the
//! `fixtures-prod-scheme.tar.gz` release asset, sha256-pinned in
//! `crates/verity-types/fixtures.sha256`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use libssz::{SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha2Hasher};
use serde::Deserialize;
use verity_crypto::containers::{
    HashTreeLayer, HashTreeOpening, PublicKey, Signature, SignedAttestation,
};

/// The five type names this crate owns. Anything else in these suites belongs to
/// `verity-types` and is counted as skipped rather than treated as an unknown type.
const OWNED_TYPES: &[&str] = &[
    "PublicKey",
    "Signature",
    "HashTreeOpening",
    "HashTreeLayer",
    "SignedAttestation",
];

/// Fixture suites read. `test_xmss_containers` holds the key and signature shapes;
/// `SignedAttestation` ships with the consensus containers because that is what it is.
const SUITES: &[&str] = &["test_consensus_containers", "test_xmss_containers"];

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
fn should_match_leanspec_xmss_vectors_when_fixtures_are_present() {
    let Some(root) = fixtures_dir() else {
        eprintln!("skipping: set VERITY_FIXTURES to run leanSpec XMSS SSZ vectors");
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
        "no SSZ JSON under {} (expected **/{{{}}}/*.json)",
        root.display(),
        SUITES.join(",")
    );

    let mut matched = 0usize;
    let mut skipped = 0usize;
    let mut failures = Vec::new();
    let mut seen_types = BTreeMap::new();
    for path in &files {
        run_file(
            path,
            &mut matched,
            &mut skipped,
            &mut failures,
            &mut seen_types,
        );
    }

    eprintln!("matched {matched} leanSpec XMSS SSZ vectors, skipped {skipped}");
    assert!(
        failures.is_empty(),
        "{} SSZ vector(s) disagreed with leanSpec:\n{}",
        failures.len(),
        failures.join("\n")
    );

    // Every owned type must have been exercised. A suite that stopped shipping one of them
    // would otherwise leave this passing on the remaining four.
    let missing: Vec<&&str> = OWNED_TYPES
        .iter()
        .filter(|name| !seen_types.contains_key(**name))
        .collect();
    assert!(
        missing.is_empty(),
        "no vector covered {missing:?}; the suites shipped {seen_types:?}"
    );
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
            .any(|c| SUITES.iter().any(|suite| c.as_os_str() == *suite));
        if is_json && is_container_suite {
            out.push(path);
        }
    }
}

fn run_file(
    path: &Path,
    matched: &mut usize,
    skipped: &mut usize,
    failures: &mut Vec<String>,
    seen_types: &mut BTreeMap<String, usize>,
) {
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
            Ok(Outcome::Matched) => {
                *matched += 1;
                *seen_types.entry(case.type_name.clone()).or_default() += 1;
            }
            Ok(Outcome::Skipped) => *skipped += 1,
            Err(error) => failures.push(format!("{} ({id}): {error}", path.display())),
        }
    }
}

fn run_case(case: &FixtureCase) -> Result<Outcome, String> {
    if !OWNED_TYPES.contains(&case.type_name.as_str()) {
        return Ok(Outcome::Skipped);
    }

    let bytes = from_hex(&case.serialized)?;
    let reject = case.rejection_reason.is_some();
    let root = if reject {
        Vec::new()
    } else if case.root.is_empty() {
        return Err("valid vector carries no root".into());
    } else {
        from_hex(&case.root)?
    };

    match case.type_name.as_str() {
        "PublicKey" => apply::<PublicKey>(&bytes, &root, reject),
        "Signature" => apply::<Signature>(&bytes, &root, reject),
        "HashTreeOpening" => apply::<HashTreeOpening>(&bytes, &root, reject),
        "HashTreeLayer" => apply::<HashTreeLayer>(&bytes, &root, reject),
        "SignedAttestation" => apply::<SignedAttestation>(&bytes, &root, reject),
        other => Err(format!("unhandled SSZ type {other}")),
    }
}

fn apply<T>(bytes: &[u8], root: &[u8], reject: bool) -> Result<Outcome, String>
where
    T: SszDecode + SszEncode + HashTreeRoot + PartialEq + std::fmt::Debug,
{
    if reject {
        match T::from_ssz_bytes(bytes) {
            Err(_) => Ok(Outcome::Matched),
            Ok(_) => Err("decode succeeded; leanSpec expects rejection".into()),
        }
    } else {
        check_roundtrip::<T>(bytes, root).map(|()| Outcome::Matched)
    }
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
