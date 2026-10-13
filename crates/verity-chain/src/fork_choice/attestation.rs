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
use verity_types::{
    AttestationData, Checkpoint, SignedAggregatedAttestation, Slot, ValidatorIndex,
};

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

/// History is linear: source at or before target, then target at or before head.
const fn validate_slot_order(
    source: Slot,
    target: Slot,
    head: Slot,
) -> Result<(), RejectionReason> {
    if source.0 > target.0 {
        return Err(RejectionReason::SourceAfterTarget);
    }
    if head.0 < target.0 {
        return Err(RejectionReason::HeadOlderThanTarget);
    }
    Ok(())
}

/// Each checkpoint's slot must be ordered and equal the slot of the block it names.
fn validate_topology(store: &Store, data: &AttestationData) -> Result<(), RejectionReason> {
    validate_slot_order(data.source.slot, data.target.slot, data.head.slot)?;

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

#[cfg(test)]
mod tests {
    use verity_types::{
        AggregationBits, AttestationData, Checkpoint, Interval, SignedAggregatedAttestation,
        SingleMessageAggregate, Slot, ValidatorIndex,
    };

    use crate::fork_choice::testing::{anchored_on_genesis, import_at};
    use crate::merkle::hash_tree_root;

    use super::{
        AttestationSignature, RejectionReason, record_aggregated_payload,
        record_attestation_signature, validate_attestation, validate_attestation_signer,
    };

    struct Chain {
        store: crate::fork_choice::Store,
        genesis: Checkpoint,
        first: Checkpoint,
        second: Checkpoint,
    }

    fn chain() -> Chain {
        let (mut store, genesis_state) = anchored_on_genesis(2);
        let genesis = store.latest_finalized;
        let first_block = import_at(&mut store, &genesis_state, 1);
        let first = Checkpoint {
            root: hash_tree_root(&first_block),
            slot: Slot(1),
        };
        let first_state = store.states[&first.root].clone();
        let second_block = import_at(&mut store, &first_state, 2);
        let second = Checkpoint {
            root: hash_tree_root(&second_block),
            slot: Slot(2),
        };
        store.time = Interval(10);
        Chain {
            store,
            genesis,
            first,
            second,
        }
    }

    fn vote(chain: &Chain) -> AttestationData {
        AttestationData {
            slot: Slot(2),
            source: chain.genesis,
            target: chain.first,
            head: chain.second,
        }
    }

    fn proof(bits: &[bool]) -> SingleMessageAggregate {
        SingleMessageAggregate {
            participants: AggregationBits::try_from(bits.to_vec()).expect("small bitfield"),
            ..SingleMessageAggregate::default()
        }
    }

    #[test]
    fn should_accept_a_vote_whose_blocks_slots_ancestry_and_time_are_valid() {
        let chain = chain();
        assert_eq!(validate_attestation(&chain.store, &vote(&chain)), Ok(()));
    }

    fn assert_rejected(chain: &Chain, data: AttestationData, expected: RejectionReason) {
        assert_eq!(validate_attestation(&chain.store, &data), Err(expected));
    }

    #[test]
    fn should_name_each_unknown_checkpoint_in_validation_order() {
        let chain = chain();
        let valid = vote(&chain);
        let mut data = valid;
        data.source.root = [1; 32];
        assert_rejected(&chain, data, RejectionReason::UnknownSourceBlock);
        data = valid;
        data.target.root = [2; 32];
        assert_rejected(&chain, data, RejectionReason::UnknownTargetBlock);
        data = valid;
        data.head.root = [3; 32];
        assert_rejected(&chain, data, RejectionReason::UnknownHeadBlock);
    }

    #[test]
    fn should_reject_every_invalid_slot_relationship() {
        let chain = chain();
        let valid = vote(&chain);
        let mut data = valid;
        data.source = chain.second;
        data.target = chain.first;
        assert_rejected(&chain, data, RejectionReason::SourceAfterTarget);
        data = valid;
        data.target = chain.second;
        data.head = chain.first;
        assert_rejected(&chain, data, RejectionReason::HeadOlderThanTarget);
        data = valid;
        data.source.slot = Slot(1);
        assert_rejected(&chain, data, RejectionReason::SourceSlotMismatch);
        data = valid;
        data.target.slot = Slot(0);
        assert_rejected(&chain, data, RejectionReason::TargetSlotMismatch);
        data = valid;
        data.head.slot = Slot(3);
        assert_rejected(&chain, data, RejectionReason::HeadSlotMismatch);
        data = valid;
        data.slot = Slot(1);
        assert_rejected(&chain, data, RejectionReason::AttestationSlotBeforeHead);
        data.slot = Slot(3);
        assert_rejected(&chain, data, RejectionReason::AttestationTooFarInFuture);
    }

    #[test]
    fn should_reject_checkpoints_that_do_not_share_the_required_ancestry() {
        let mut chain = chain();
        let first_state = chain.store.states[&chain.first.root].clone();
        let sibling_block = import_at(&mut chain.store, &first_state, 3);
        let sibling = Checkpoint {
            root: hash_tree_root(&sibling_block),
            slot: Slot(3),
        };
        chain.store.time = Interval(15);

        let source_off_target = AttestationData {
            slot: Slot(3),
            source: chain.second,
            target: Checkpoint {
                slot: Slot(3),
                ..sibling
            },
            head: sibling,
        };
        assert_eq!(
            validate_attestation(&chain.store, &source_off_target),
            Err(RejectionReason::SourceNotAncestorOfTarget)
        );

        let target_off_head = AttestationData {
            slot: Slot(3),
            source: chain.genesis,
            target: chain.second,
            head: sibling,
        };
        assert_eq!(
            validate_attestation(&chain.store, &target_off_head),
            Err(RejectionReason::TargetNotAncestorOfHead)
        );

        chain.store.latest_finalized = chain.second;
        let finalized_off_head = AttestationData {
            slot: Slot(3),
            source: chain.genesis,
            target: chain.genesis,
            head: sibling,
        };
        assert_eq!(
            validate_attestation(&chain.store, &finalized_off_head),
            Err(RejectionReason::HeadNotDescendantOfFinalized)
        );
    }

    #[test]
    fn should_validate_and_record_only_signers_in_the_target_registry() {
        let mut chain = chain();
        let data = vote(&chain);
        assert_eq!(
            validate_attestation_signer(&chain.store, ValidatorIndex(2), &data),
            Err(RejectionReason::ValidatorNotInState)
        );

        record_attestation_signature(
            &mut chain.store,
            ValidatorIndex(1),
            data,
            AttestationSignature(vec![7]),
        )
        .expect("the signer is in the target registry");
        assert_eq!(chain.store.attestation_signatures[&data].len(), 1);
    }

    #[test]
    fn should_reject_empty_or_out_of_registry_aggregates_without_mutating_the_pool() {
        let mut chain = chain();
        let data = vote(&chain);
        let empty = SignedAggregatedAttestation {
            data,
            proof: proof(&[]),
        };
        assert_eq!(
            record_aggregated_payload(&mut chain.store, &empty),
            Err(RejectionReason::EmptyAggregationBits)
        );

        let outside = SignedAggregatedAttestation {
            data,
            proof: proof(&[false, false, true]),
        };
        assert_eq!(
            record_aggregated_payload(&mut chain.store, &outside),
            Err(RejectionReason::ValidatorNotInState)
        );
        assert!(chain.store.latest_new_aggregated_payloads.is_empty());

        let valid = SignedAggregatedAttestation {
            data,
            proof: proof(&[true, false]),
        };
        record_aggregated_payload(&mut chain.store, &valid).expect("one known validator");
        assert!(chain.store.latest_new_aggregated_payloads[&data].contains(&valid.proof));
    }
}

#[cfg(kani)]
mod harnesses {
    use verity_types::Slot;

    use super::{RejectionReason, validate_slot_order};

    /// A successful topology check orders source, target, and head; failures name the first
    /// violated relation in leanSpec's order.
    // Lean overlap: FC-3 (`Store.attestation_topology`). Future Lean-adoption deletion candidate.
    #[kani::proof]
    fn attestation_slots_are_topologically_ordered() {
        let source = Slot(kani::any());
        let target = Slot(kani::any());
        let head = Slot(kani::any());
        match validate_slot_order(source, target, head) {
            Ok(()) => {
                assert!(source.0 <= target.0);
                assert!(target.0 <= head.0);
            }
            Err(RejectionReason::SourceAfterTarget) => {
                assert!(source.0 > target.0);
            }
            Err(RejectionReason::HeadOlderThanTarget) => {
                assert!(source.0 <= target.0);
                assert!(head.0 < target.0);
            }
            Err(_) => unreachable!("no other rejection is defined"),
        }
    }
}
