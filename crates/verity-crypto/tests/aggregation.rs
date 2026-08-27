//! Aggregation and aggregate-proof verification, end to end, over leanVM.
//!
//! Gated on `VERITY_TEST_KEYS` like the signing tests, and for the same reason: proving needs
//! real production-scheme signatures, and there is no smaller key that produces one.
//!
//! # One test function, on purpose
//!
//! leanVM's prover allocates from one arena per process, so two proofs generated at once in
//! one process corrupt each other. libtest runs the tests inside a binary on parallel
//! threads, so everything that proves lives in this single function; cargo runs test binaries
//! one at a time, which keeps it the only prover in the run. Splitting this into two
//! `#[test]`s would reintroduce exactly the race the arena cannot survive.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use verity_crypto::aggregate::{MAX_XMSS_AGGREGATED, init_prover};
use verity_crypto::containers::PublicKey;
use verity_crypto::{SecretKey, aggregate_single_message, merge_single_message_proofs, sign};
use verity_types::Slot;

#[derive(Debug, Deserialize)]
struct KeyFile {
    attestation_keypair: KeyPair,
}

#[derive(Debug, Deserialize)]
struct KeyPair {
    public_key: String,
    secret_key: String,
}

fn test_keys() -> Option<Vec<(PublicKey, SecretKey)>> {
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
    assert!(files.len() >= 2, "aggregation needs at least two keys");

    Some(
        files
            .iter()
            .take(2)
            .map(|path| {
                let parsed: KeyFile =
                    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
                let public: [u8; 52] = from_hex(&parsed.attestation_keypair.public_key)
                    .try_into()
                    .expect("public key is not 52 bytes");
                (
                    PublicKey::from_bytes52(&public).expect("public key does not parse"),
                    SecretKey::from_ssz_bytes(&from_hex(&parsed.attestation_keypair.secret_key))
                        .expect("secret key does not parse"),
                )
            })
            .collect(),
    )
}

fn from_hex(text: &str) -> Vec<u8> {
    let text = text.strip_prefix("0x").unwrap_or(text);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("invalid hex"))
        .collect()
}

#[test]
fn should_prove_and_verify_when_real_signatures_are_aggregated() {
    let Some(keys) = test_keys() else {
        eprintln!("skipping: set VERITY_TEST_KEYS to run aggregation tests");
        return;
    };

    init_prover();

    // A slot both keys are prepared for. They are generated together, so the windows agree,
    // but taking the later start rather than assuming so keeps the test honest.
    let slot = Slot(
        keys.iter()
            .map(|(_, secret)| secret.prepared_interval().start)
            .max()
            .unwrap(),
    );
    let message = [0x5au8; 32];

    let signatures: Vec<(PublicKey, _)> = keys
        .iter()
        .map(|(public, secret)| {
            (
                public.clone(),
                sign(secret, slot, &message).expect("signing failed"),
            )
        })
        .collect();

    // --- Type-1: many validators, one message ------------------------------------------
    let proof = aggregate_single_message(Vec::new(), &signatures, &message, slot)
        .expect("single-message aggregation failed");

    assert_eq!(proof.verify(), Ok(()));
    assert_eq!(proof.message(), message);
    assert_eq!(proof.slot(), slot);
    assert_eq!(
        proof.participants().unwrap().len(),
        signatures.len(),
        "every signer must appear in the proof"
    );

    // The wire form drops the keys; a verifier resupplies them from its own registry.
    let wire = proof.to_wire();
    let participants: Vec<PublicKey> = keys.iter().map(|(public, _)| public.clone()).collect();
    let decoded = verity_crypto::SingleMessageProof::from_wire(&wire, &participants)
        .expect("proof does not survive the wire form");
    assert_eq!(decoded.verify(), Ok(()));
    assert_eq!(decoded.message(), message);

    // A key set that does not match the proof cannot rescue it.
    let wrong = verity_crypto::SingleMessageProof::from_wire(&wire, &participants[..1]);
    assert!(
        wrong.is_err() || wrong.unwrap().verify().is_err(),
        "a proof must not verify against the wrong participant set"
    );

    // --- Type-2: components over distinct messages, merged into a block proof -----------
    //
    // The second component signs at the *next* slot, not a second message at `slot`. These
    // are throwaway keys and nothing here would notice, but a test that signs two different
    // messages under one (key, slot) writes the one thing that breaks an XMSS key into the
    // repository as an example. In a real block the second message is the block root, signed
    // with the proposal key, which is a different key rather than a different slot.
    let other_slot = Slot(slot.0 + 1);
    let other_message = [0xa5u8; 32];
    let other_signatures: Vec<(PublicKey, _)> = keys
        .iter()
        .take(1)
        .map(|(public, secret)| {
            (
                public.clone(),
                sign(secret, other_slot, &other_message).expect("signing failed"),
            )
        })
        .collect();
    let other_proof =
        aggregate_single_message(Vec::new(), &other_signatures, &other_message, other_slot)
            .expect("single-message aggregation failed");

    let block_proof = merge_single_message_proofs(vec![proof, other_proof]).expect("merge failed");
    assert_eq!(block_proof.verify(), Ok(()));

    let block_wire = block_proof.to_wire();
    let per_component = vec![participants.clone(), participants[..1].to_vec()];
    let decoded_block = verity_crypto::MultiMessageProof::from_wire(&block_wire, &per_component)
        .expect("block proof does not survive the wire form");
    assert_eq!(decoded_block.verify(), Ok(()));

    assert!(
        signatures.len() <= MAX_XMSS_AGGREGATED,
        "the test must stay inside the circuit's own bound"
    );
}
