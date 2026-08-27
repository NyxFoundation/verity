//! Admitting gossiped votes into the store's pools.
//!
//! leanSpec's `on_gossip_attestation` and `on_gossip_aggregated_attestation` each do three
//! things in a row: validate the vote, verify its signature, then record it. Verity splits
//! the signature verification out. It is the one step that needs a cryptographic library,
//! and this crate has none by design (see the crate docs); the composed entry points land
//! with `verity-crypto`, which supplies the missing middle.
//!
//! What that leaves here is every check that is a decision about the *vote* rather than
//! about the bytes signing it — which is all but two of the rejections leanSpec's gossip
//! path can produce.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/fork_choice.py`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`.

use verity_types::config::{GOSSIP_DISPARITY_INTERVALS, INTERVALS_PER_SLOT};
use verity_types::{AttestationData, Checkpoint, SignedAggregatedAttestation, ValidatorIndex};

use crate::error::RejectionReason;
use crate::fork_choice::store::{AttestationSignature, AttestationSignatureEntry, Store};
use crate::fork_choice::weights::participants;

/// Whether a vote is admissible against the store's current view.
///
/// The vote must name blocks the store knows, order them the way history allows, agree with
/// those blocks' actual slots, lie on one chain, and belong to a slot that has already
/// started locally. The head must also descend from the finalized block: fork choice only
/// ever walks down from there, so an orphaned head could never carry weight, and admitting
/// one would let a stale vote re-enter after pruning dropped it.
///
/// # Errors
///
/// One of the ten gossip-validation [`RejectionReason`]s, named on each check below.
pub fn validate_attestation(store: &Store, data: &AttestationData) -> Result<(), RejectionReason> {
    validate_availability(store, data)?;
    validate_topology(store, data)?;
    validate_ancestry(store, data)?;
    validate_timing(store, data)
}

/// Every block the vote names must already be in the local view.
fn validate_availability(store: &Store, data: &AttestationData) -> Result<(), RejectionReason> {
    if !store.blocks.contains_key(&data.source.root) {
        return Err(RejectionReason::UnknownSourceBlock);
    }
    if !store.blocks.contains_key(&data.target.root) {
        return Err(RejectionReason::UnknownTargetBlock);
    }
    if !store.blocks.contains_key(&data.head.root) {
        return Err(RejectionReason::UnknownHeadBlock);
    }
    Ok(())
}

/// History is linear: source at or before target, target at or before head — and each
/// checkpoint's slot must be the slot of the block it names.
fn validate_topology(store: &Store, data: &AttestationData) -> Result<(), RejectionReason> {
    if data.source.slot.0 > data.target.slot.0 {
        return Err(RejectionReason::SourceAfterTarget);
    }
    if data.head.slot.0 < data.target.slot.0 {
        return Err(RejectionReason::HeadOlderThanTarget);
    }

    let checks = [
        (data.source, RejectionReason::SourceSlotMismatch),
        (data.target, RejectionReason::TargetSlotMismatch),
        (data.head, RejectionReason::HeadSlotMismatch),
    ];
    for (checkpoint, reason) in checks {
        // Availability ran first, so every root here resolves.
        if store.blocks.get(&checkpoint.root).map(|block| block.slot) != Some(checkpoint.slot) {
            return Err(reason);
        }
    }
    Ok(())
}

/// The three checkpoints must lie on one chain, rooted under the finalized block.
///
/// Weight accrues to every ancestor of the attested head, so a head on a sibling branch
/// would steer that weight onto a chain the vote never meant to support.
fn validate_ancestry(store: &Store, data: &AttestationData) -> Result<(), RejectionReason> {
    if !store.is_ancestor(data.source, data.target) {
        return Err(RejectionReason::SourceNotAncestorOfTarget);
    }
    if !store.is_ancestor(data.target, data.head) {
        return Err(RejectionReason::TargetNotAncestorOfHead);
    }
    if !store.is_ancestor(store.latest_finalized, data.head) {
        return Err(RejectionReason::HeadNotDescendantOfFinalized);
    }
    Ok(())
}

/// A vote cannot predate the head it claims, nor arrive before its own slot has started.
///
/// The clock-skew margin is one interval, not a whole slot. With five intervals per slot,
/// slot 10 begins at interval 50: interval 49 is admitted as skew, interval 45 would admit a
/// vote a full slot early and let an adversary pre-publish next-slot aggregates.
///
/// The comparison stays in slot units. Multiplying a near-`u64::MAX` wire slot up into
/// intervals would overflow before it could be rejected.
fn validate_timing(store: &Store, data: &AttestationData) -> Result<(), RejectionReason> {
    if data.slot.0 < data.head.slot.0 {
        return Err(RejectionReason::AttestationSlotBeforeHead);
    }

    let admission_horizon = store.time.0.saturating_add(GOSSIP_DISPARITY_INTERVALS);
    if data.slot.0 > admission_horizon / INTERVALS_PER_SLOT {
        return Err(RejectionReason::AttestationTooFarInFuture);
    }
    Ok(())
}

/// Whether a gossiped vote is admissible *and* names a validator the target block knew.
///
/// This is the admission decision every node makes, aggregator or not. leanSpec reaches the
/// registry check on the way to resolving the signer's public key, so a node that only
/// validates and relays applies it too.
///
/// # Errors
///
/// Any [`RejectionReason`] from [`validate_attestation`], or
/// [`RejectionReason::ValidatorNotInState`] when the signer is outside the target's registry.
pub fn validate_attestation_signer(
    store: &Store,
    validator_index: ValidatorIndex,
    data: &AttestationData,
) -> Result<(), RejectionReason> {
    validate_attestation(store, data)?;
    validate_signers(store, data.target, [validator_index])
}

/// Whether the target block's post-state registry holds every named validator.
///
/// The registry is read from the target's post-state rather than the head's: that is the
/// state a verifier would resolve the signers' keys from, so an index outside it names a
/// validator the vote's own target never knew.
///
/// # Errors
///
/// [`RejectionReason::ValidatorNotInState`] for the first index outside the registry.
fn validate_signers(
    store: &Store,
    target: Checkpoint,
    signers: impl IntoIterator<Item = ValidatorIndex>,
) -> Result<(), RejectionReason> {
    // Validation ran first, so the target's post-state is present whenever this is reached.
    let registry_size = store
        .states
        .get(&target.root)
        .map_or(0, |state| state.validators.len() as u64);

    for validator_index in signers {
        if validator_index.0 >= registry_size {
            return Err(RejectionReason::ValidatorNotInState);
        }
    }
    Ok(())
}

/// Records one validator's signature in the aggregator's pool.
///
/// **The caller must have verified `signature` against the validator's key first.** This
/// crate cannot: see the module docs. Every other admission check leanSpec performs on the
/// gossip path runs here, in leanSpec's order, before anything is written.
///
/// A node that does not aggregate has no reason to call this. leanSpec validates and relays
/// such a vote without keeping it, which is [`validate_attestation`] on its own.
///
/// # Errors
///
/// Any [`RejectionReason`] from [`validate_attestation`], or
/// [`RejectionReason::ValidatorNotInState`] when the signer is outside the target's registry.
/// The store is left untouched on every one of them.
pub fn record_attestation_signature(
    store: &mut Store,
    validator_index: ValidatorIndex,
    data: AttestationData,
    signature: AttestationSignature,
) -> Result<(), RejectionReason> {
    validate_attestation_signer(store, validator_index, &data)?;

    store
        .attestation_signatures
        .entry(data)
        .or_default()
        .insert(AttestationSignatureEntry {
            validator_index,
            signature,
        });
    Ok(())
}

/// Records a gossiped aggregate proof in the pending pool.
///
/// **The caller must have verified the proof against its participants' keys first**, for the
/// same reason as above. The proof carries no weight until an acceptance tick promotes it —
/// see [`super::timeline::accept_new_attestations`].
///
/// # Errors
///
/// Any [`RejectionReason`] from [`validate_attestation`],
/// [`RejectionReason::EmptyAggregationBits`] when the proof names nobody, or
/// [`RejectionReason::ValidatorNotInState`] when a participant is outside the target's
/// registry. The store is left untouched on every one of them.
pub fn record_aggregated_payload(
    store: &mut Store,
    attestation: &SignedAggregatedAttestation,
) -> Result<(), RejectionReason> {
    validate_attestation(store, &attestation.data)?;

    let signers: Vec<ValidatorIndex> = participants(&attestation.proof.participants).collect();
    if signers.is_empty() {
        return Err(RejectionReason::EmptyAggregationBits);
    }
    validate_signers(store, attestation.data.target, signers)?;

    store
        .latest_new_aggregated_payloads
        .entry(attestation.data)
        .or_default()
        .insert(attestation.proof.clone());
    Ok(())
}
