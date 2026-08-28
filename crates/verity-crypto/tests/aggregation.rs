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

mod common;

use verity_crypto::aggregate::{MAX_XMSS_AGGREGATED, init_prover};
use verity_crypto::containers::PublicKey;
use verity_crypto::{aggregate_single_message, merge_single_message_proofs, sign};
use verity_types::Slot;

#[test]
fn should_prove_and_verify_when_real_signatures_are_aggregated() {
    let Some(keys) = common::test_keys(2) else {
        eprintln!("skipping: set VERITY_TEST_KEYS to run aggregation tests");
        return;
    };

    init_prover();

    // A slot both keys are prepared for. They are generated together, so the windows agree,
    // but taking the later start rather than assuming so keeps the test honest.
    let slot = Slot(
        keys.iter()
            .map(|key| key.secret.prepared_interval().start)
            .max()
            .unwrap(),
    );
    let message = [0x5au8; 32];

    let signatures: Vec<(PublicKey, _)> = keys
        .iter()
        .map(|key| {
            (
                key.public.clone(),
                sign(&key.secret, slot, &message).expect("signing failed"),
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

    // The serialized form drops the keys; a verifier resupplies them from its own registry.
    let proof_bytes = proof.to_proof_bytes();
    let participants: Vec<PublicKey> = keys.iter().map(|key| key.public.clone()).collect();
    let decoded = verity_crypto::SingleMessageProof::from_proof_bytes(&proof_bytes, &participants)
        .expect("proof does not survive its serialized form");
    assert_eq!(decoded.verify(), Ok(()));
    assert_eq!(decoded.message(), message);

    // A key set that does not match the proof cannot rescue it.
    let wrong =
        verity_crypto::SingleMessageProof::from_proof_bytes(&proof_bytes, &participants[..1]);
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
        .map(|key| {
            (
                key.public.clone(),
                sign(&key.secret, other_slot, &other_message).expect("signing failed"),
            )
        })
        .collect();
    let other_proof =
        aggregate_single_message(Vec::new(), &other_signatures, &other_message, other_slot)
            .expect("single-message aggregation failed");

    let block_proof = merge_single_message_proofs(vec![proof, other_proof]).expect("merge failed");
    assert_eq!(block_proof.verify(), Ok(()));

    let block_proof_bytes = block_proof.to_proof_bytes();
    let per_component = vec![participants.clone(), participants[..1].to_vec()];
    let decoded_block =
        verity_crypto::MultiMessageProof::from_proof_bytes(&block_proof_bytes, &per_component)
            .expect("block proof does not survive its serialized form");
    assert_eq!(decoded_block.verify(), Ok(()));

    assert!(
        signatures.len() <= MAX_XMSS_AGGREGATED,
        "the test must stay inside the circuit's own bound"
    );
}
