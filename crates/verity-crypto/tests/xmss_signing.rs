//! Sign, verify, and aggregate with real production-scheme key material.
//!
//! Gated on `VERITY_TEST_KEYS` pointing at a directory of `leansig-test-keys` `prod_scheme`
//! JSON files. The fast `cargo test` gate leaves it unset and every test here returns,
//! because a production-scheme secret key is 33.5 MB and there is no smaller one that still
//! exercises the parameters Verity actually runs.
//!
//! The keys are attestation-role only, which is all these tests need: the roles differ in
//! which duty signs with them, not in the cryptography.
//!
//! Source: <https://github.com/leanEthereum/leansig-test-keys>, `prod_scheme.tar.gz`,
//! sha256-pinned in `crates/verity-crypto/test-keys.sha256`.

use std::fs;
use std::path::PathBuf;

use libssz::{SszDecode, SszEncode};
use serde::Deserialize;
use verity_crypto::containers::{PublicKey, Signature};
use verity_crypto::error::SignatureError;
use verity_crypto::scheme::SIGNATURE_BYTES;
use verity_crypto::{SecretKey, sign, verify};
use verity_types::Slot;

/// One key file: leanSpec's tooling writes both halves as hex of their canonical SSZ.
#[derive(Debug, Deserialize)]
struct KeyFile {
    attestation_keypair: KeyPair,
}

#[derive(Debug, Deserialize)]
struct KeyPair {
    public_key: String,
    secret_key: String,
}

struct TestKey {
    public: PublicKey,
    secret: SecretKey,
}

/// Loads the `n`th key file in the directory, or `None` when the gate is off.
fn test_key(nth: usize) -> Option<TestKey> {
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

    let path = files.get(nth).unwrap_or_else(|| {
        panic!(
            "{} holds {} key files, needed at least {}",
            dir.display(),
            files.len(),
            nth + 1
        )
    });

    let text = fs::read_to_string(path).unwrap();
    let parsed: KeyFile = serde_json::from_str(&text).unwrap();

    let public_bytes: [u8; 52] = from_hex(&parsed.attestation_keypair.public_key)
        .try_into()
        .expect("public key is not 52 bytes");

    Some(TestKey {
        public: PublicKey::from_bytes52(&public_bytes).expect("public key does not parse"),
        secret: SecretKey::from_ssz_bytes(&from_hex(&parsed.attestation_keypair.secret_key))
            .expect("secret key does not parse"),
    })
}

/// A slot the key is prepared to sign, taken from the key itself rather than assumed.
fn signable_slot(key: &TestKey) -> Slot {
    Slot(key.secret.prepared_interval().start)
}

fn message(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn from_hex(text: &str) -> Vec<u8> {
    let text = text.strip_prefix("0x").unwrap_or(text);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("invalid hex"))
        .collect()
}

#[test]
fn should_verify_when_a_signature_is_checked_against_the_key_that_made_it() {
    let Some(key) = test_key(0) else {
        eprintln!("skipping: set VERITY_TEST_KEYS to run signing tests");
        return;
    };
    let slot = signable_slot(&key);

    let signature = sign(&key.secret, slot, &message(1)).expect("signing failed");
    assert_eq!(verify(&key.public, slot, &message(1), &signature), Ok(()));
}

#[test]
fn should_survive_its_own_encoding_when_a_real_signature_is_round_tripped() {
    let Some(key) = test_key(0) else {
        return;
    };
    let slot = signable_slot(&key);

    let signature = sign(&key.secret, slot, &message(2)).expect("signing failed");
    let encoded = signature.to_ssz();

    assert_eq!(encoded.len(), SIGNATURE_BYTES);
    let decoded = Signature::from_ssz_bytes(&encoded).expect("real signature does not decode");
    assert_eq!(decoded, signature);
    assert_eq!(verify(&key.public, slot, &message(2), &decoded), Ok(()));
}

/// leanSig derandomizes signing, so the same triple yields the identical signature. That is
/// what makes an idempotent re-sign harmless — and why the watermark refuses *equality*
/// rather than relying on it.
#[test]
fn should_produce_identical_bytes_when_the_same_message_is_signed_twice_at_one_slot() {
    let Some(key) = test_key(0) else {
        return;
    };
    let slot = signable_slot(&key);

    let first = sign(&key.secret, slot, &message(3)).expect("signing failed");
    let second = sign(&key.secret, slot, &message(3)).expect("signing failed");
    assert_eq!(first, second);
}

#[test]
fn should_refuse_when_the_message_the_slot_or_the_key_is_not_the_one_signed() {
    let Some(key) = test_key(0) else {
        return;
    };
    let Some(other) = test_key(1) else {
        return;
    };
    let slot = signable_slot(&key);

    let signature = sign(&key.secret, slot, &message(4)).expect("signing failed");

    assert_eq!(
        verify(&key.public, slot, &message(5), &signature),
        Err(SignatureError::InvalidSignature),
        "a different message must not verify"
    );
    assert_eq!(
        verify(&key.public, Slot(slot.0 + 1), &message(4), &signature),
        Err(SignatureError::InvalidSignature),
        "a different slot must not verify"
    );
    assert_eq!(
        verify(&other.public, slot, &message(4), &signature),
        Err(SignatureError::InvalidSignature),
        "a different key must not verify"
    );
}

/// leanSig asserts on both of these. The wrapper has to reach them first, or the node dies
/// on a slot it should merely have refused.
#[test]
fn should_return_a_typed_error_rather_than_panic_when_the_slot_is_unsignable() {
    let Some(key) = test_key(0) else {
        return;
    };

    let activation = key.secret.activation_interval();
    let prepared = key.secret.prepared_interval();

    // Inside the activation range but past the prepared window: recoverable by advancing.
    if prepared.end < activation.end {
        assert!(matches!(
            sign(&key.secret, Slot(prepared.end), &message(6)),
            Err(SignatureError::KeyNotPrepared { .. })
        ));
    }

    // Past the activation range entirely: no amount of preparation helps.
    assert!(matches!(
        sign(&key.secret, Slot(activation.end), &message(6)),
        Err(SignatureError::KeyNotActive { .. })
    ));
}

#[test]
fn should_reproduce_the_key_when_a_secret_key_is_duplicated_for_an_advance() {
    let Some(key) = test_key(0) else {
        return;
    };
    let slot = signable_slot(&key);

    let copy = key
        .secret
        .duplicate()
        .expect("secret key does not round trip");
    assert_eq!(copy.activation_interval(), key.secret.activation_interval());
    assert_eq!(copy.prepared_interval(), key.secret.prepared_interval());

    // The copy is the same key, so it signs identically — which is exactly why no-reuse
    // cannot rest on which copy signed.
    assert_eq!(
        sign(&copy, slot, &message(7)).unwrap(),
        sign(&key.secret, slot, &message(7)).unwrap()
    );
}
