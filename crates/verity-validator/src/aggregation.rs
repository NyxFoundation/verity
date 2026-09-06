//! The aggregator's interval-2 round: raw votes become proofs.
//!
//! A validator signs its vote and gossips the raw XMSS signature. That signature cannot sway
//! fork choice or enter a block until it is folded into a proof, and folding is what happens
//! here, once per slot, on the aggregator nodes.
//!
//! # Three pools feed a round
//!
//! - **Gossip signatures** — individual votes that arrived this slot.
//! - **New payloads** — proofs built this slot, not yet counted.
//! - **Known payloads** — proofs already counted, reusable as building blocks.
//!
//! Only the first two can start a round: re-aggregating counted proofs adds nothing. The
//! third is consulted for coverage once fresh evidence exists, because reusing an existing
//! proof keeps the proof tree shallow.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/aggregation.py`, read at commit
//! `8603fa63`.

use std::collections::HashSet;

use libssz::SszDecode;
use verity_chain::{ChainView, hash_tree_root, select_proofs_for_coverage};
use verity_crypto::aggregate::aggregate_single_message;
use verity_crypto::containers::{PublicKey, Signature};
use verity_types::{AttestationData, SignedAggregatedAttestation, Validators};

use crate::error::DutyError;
use crate::proofs;

/// Folds everything the view holds for one slot into one proof per vote.
///
/// Blocking and expensive — this is where the seconds of zk proving are spent — so the caller
/// runs it through the [`crate::Prover`], never on a runtime worker.
///
/// A vote whose round fails is logged and skipped rather than taking the whole round down:
/// its raw signatures stay in the pool and the next round tries again.
#[must_use = "the aggregates have to reach the chain task, or the round was for nothing"]
pub fn aggregate(view: &ChainView) -> Vec<SignedAggregatedAttestation> {
    let Some(validators) = view.head_state().map(|state| &state.validators) else {
        tracing::warn!("skipping aggregation: the chain view holds no state for its head");
        return Vec::new();
    };

    votes_with_fresh_evidence(view)
        .into_iter()
        .filter_map(|data| match aggregate_one(view, validators, data) {
            Ok(aggregate) => aggregate,
            Err(error) => {
                tracing::warn!(slot = data.slot.0, %error, "cannot aggregate a vote this round");
                None
            }
        })
        .collect()
}

/// The votes a round may start from, in a deterministic order.
///
/// Ordering is by the vote's own root: the pools are hash maps, and an order that varied per
/// process would make two runs of the same node produce different aggregates from the same
/// evidence.
fn votes_with_fresh_evidence(view: &ChainView) -> Vec<AttestationData> {
    let mut votes: Vec<AttestationData> = view
        .new_aggregated_payloads()
        .keys()
        .chain(view.attestation_signatures().keys())
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    votes.sort_by_key(hash_tree_root);
    votes
}

/// One vote's round: select existing proofs, fill the gaps with raw signatures, prove.
fn aggregate_one(
    view: &ChainView,
    validators: &Validators,
    data: AttestationData,
) -> Result<Option<SignedAggregatedAttestation>, DutyError> {
    // New payloads outrank known ones, so uncommitted work is reused before counted proofs.
    let (children, covered) = select_proofs_for_coverage(
        view.new_aggregated_payloads().get(&data),
        view.known_aggregated_payloads().get(&data),
    );

    // Every validator the children do not already cover still needs its own raw vote. Sorting
    // by index keeps the result independent of the order the votes arrived in.
    let mut entries: Vec<_> = view
        .attestation_signatures()
        .get(&data)
        .into_iter()
        .flatten()
        .filter(|entry| !covered.contains(&entry.validator_index))
        .collect();
    entries.sort_by_key(|entry| entry.validator_index);

    let raw: Vec<(PublicKey, Signature)> = entries
        .into_iter()
        .map(|entry| {
            let key = proofs::attestation_key(validators, entry.validator_index)?;
            let signature = Signature::from_ssz_bytes(&entry.signature.0).map_err(|_| {
                DutyError::Aggregation(verity_crypto::AggregationError::MalformedProof)
            })?;
            Ok((key, signature))
        })
        .collect::<Result<_, DutyError>>()?;

    // Fresh material is one raw signature, or two children to merge. A lone child proof is
    // already valid, so re-proving it would cost seconds and change nothing.
    if raw.is_empty() && children.len() < 2 {
        return Ok(None);
    }

    let children = children
        .iter()
        .map(|child| proofs::decode(child, validators))
        .collect::<Result<Vec<_>, DutyError>>()?;

    let proof = aggregate_single_message(children, &raw, &hash_tree_root(&data), data.slot)?;
    let participants = proof_participants(&proof, validators)?;

    Ok(Some(SignedAggregatedAttestation {
        data,
        proof: proofs::to_single_container(&proof, participants)?,
    }))
}

/// The bitfield naming the validators a freshly built proof covers.
///
/// leanVM reports the participants as keys, in its own sorted order; the container carries
/// them as registry indices, so each key is resolved back through the registry.
fn proof_participants(
    proof: &verity_crypto::aggregate::SingleMessageProof,
    validators: &Validators,
) -> Result<verity_types::AggregationBits, DutyError> {
    let covered: HashSet<Vec<u8>> = proof
        .participants()?
        .iter()
        .map(|key| key.to_bytes52().to_vec())
        .collect();

    let bits: Vec<bool> = validators
        .iter()
        .map(|validator| covered.contains(validator.attestation_public_key.as_slice()))
        .collect();

    verity_types::AggregationBits::try_from(bits).map_err(|_| {
        DutyError::Aggregation(verity_crypto::AggregationError::InvalidRequest {
            reason: "the registry is larger than a participation bitfield can name".to_string(),
        })
    })
}
