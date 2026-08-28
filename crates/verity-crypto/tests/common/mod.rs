//! Loading `leansig-test-keys` material, shared by the two test binaries that need it.
//!
//! Rust compiles each file under `tests/` as its own crate, so `xmss_signing` and
//! `aggregation` cannot see each other's helpers; a `common` module included by both is how
//! the workspace already shares fixture plumbing (`crates/verity-chain/tests/common/mod.rs`).
//!
//! Everything here is gated on `VERITY_TEST_KEYS`. Callers that get `None` must return: a
//! production-scheme secret key is 33.5 MB, and there is no smaller one that still exercises
//! the parameters Verity runs.

#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use verity_crypto::SecretKey;
use verity_crypto::containers::PublicKey;

/// One key file, as leanSpec's tooling writes it: both halves as hex of their canonical SSZ.
///
/// The files carry attestation-role keys only, which is all these tests need — the roles
/// differ in which duty signs with them, not in the cryptography.
#[derive(Debug, Deserialize)]
struct KeyFile {
    attestation_keypair: KeyPair,
}

#[derive(Debug, Deserialize)]
struct KeyPair {
    public_key: String,
    secret_key: String,
}

/// A validator's key pair, parsed into this crate's own types.
pub struct TestKey {
    pub public: PublicKey,
    pub secret: SecretKey,
}

/// Loads the first `count` key files in `VERITY_TEST_KEYS`, or `None` when it is unset.
///
/// # Panics
///
/// When the variable is set but the directory is missing, holds fewer than `count` key files,
/// or holds one that does not parse. A gate that is switched on and then silently covers
/// nothing is worse than no gate.
pub fn test_keys(count: usize) -> Option<Vec<TestKey>> {
    let dir = PathBuf::from(std::env::var_os("VERITY_TEST_KEYS")?);
    assert!(
        dir.is_dir(),
        "VERITY_TEST_KEYS is set but {} is not a directory",
        dir.display()
    );

    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();

    assert!(
        files.len() >= count,
        "{} holds {} key files, needed {count}",
        dir.display(),
        files.len()
    );

    Some(files.iter().take(count).map(read_key).collect())
}

fn read_key(path: &PathBuf) -> TestKey {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let parsed: KeyFile = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not a key file: {error}", path.display()));

    let public: [u8; 52] = from_hex(&parsed.attestation_keypair.public_key)
        .try_into()
        .expect("public key is not 52 bytes");

    TestKey {
        public: PublicKey::from_bytes52(&public).expect("public key does not parse"),
        secret: SecretKey::from_ssz_bytes(&from_hex(&parsed.attestation_keypair.secret_key))
            .expect("secret key does not parse"),
    }
}

/// Decodes hex with an optional `0x` prefix, panicking on anything else.
///
/// Test-only, so it panics where `keystore`'s own decoder returns `None`: a malformed fixture
/// is a broken test setup, not an input to handle.
pub fn from_hex(text: &str) -> Vec<u8> {
    let text = text.strip_prefix("0x").unwrap_or(text);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("invalid hex"))
        .collect()
}
