//! Importing a block into the store, and the head update that follows it.
//!
//! Signature verification is not here. leanSpec's `on_block` verifies the block proof
//! between its wire checks and the state transition; this crate has no cryptographic
//! dependency (see the crate docs), so the caller verifies before calling. Everything else
//! leanSpec does in that function runs below, in its order.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/fork_choice.py`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`.

use std::collections::HashSet;

use verity_types::config::HISTORICAL_ROOTS_LIMIT;
use verity_types::{AttestationData, Block, Checkpoint};

use crate::error::RejectionReason;
use crate::fork_choice::prune::prune_stale_attestation_data;
use crate::fork_choice::store::Store;
use crate::fork_choice::weights::{latest_votes, lmd_ghost_head};
use crate::justification::advance_checkpoint;
use crate::merkle::hash_tree_root;
use crate::state_transition::state_transition;

/// Imports `block` and recomputes the head.
///
/// **The caller must have verified the block's proof first.** This crate cannot: see the
/// module docs.
///
/// A block already in the store is accepted as a no-op — re-importing it could change
/// nothing, and treating a duplicate as an error would make gossip's own redundancy look
/// like a fault.
///
/// # Errors
///
/// - [`RejectionReason::UnknownParentBlock`] when the parent has no state here, which means
///   the chain below this block still has to be synced.
/// - [`RejectionReason::BlockSlotGapTooLarge`] when the block runs so far beyond its parent
///   that the transition's empty-slot walk would be unbounded.
/// - [`RejectionReason::BlockTooFarInFuture`] when the block's slot is past the horizon the
///   store's own clock admits.
/// - [`RejectionReason::DuplicateAttestationData`] when the body repeats one vote.
/// - Any [`RejectionReason`] the state transition itself produces.
///
/// The store is left untouched on every one of them: nothing is written until the
/// transition has returned a post-state.
pub fn on_block(store: &mut Store, block: &Block) -> Result<(), RejectionReason> {
    let block_root = hash_tree_root(block);
    if store.blocks.contains_key(&block_root) {
        return Ok(());
    }

    let post_state = {
        let parent_state = store
            .states
            .get(&block.parent_root)
            .ok_or(RejectionReason::UnknownParentBlock)?;

        if block.slot.0.saturating_sub(parent_state.slot.0) > HISTORICAL_ROOTS_LIMIT as u64 {
            return Err(RejectionReason::BlockSlotGapTooLarge);
        }
        if block.slot.0 > store.current_slot().0.saturating_add(1) {
            return Err(RejectionReason::BlockTooFarInFuture);
        }
        reject_duplicate_attestation_data(block)?;

        state_transition(parent_state, block)?
    };

    let previous_finalized_slot = store.latest_finalized.slot;

    store.latest_justified =
        advance_checkpoint(store.latest_justified, post_state.latest_justified);
    seed_block_votes(store, block);
    store.blocks.insert(block_root, block.clone());
    store.states.insert(block_root, post_state);

    update_head(store);

    if store.latest_finalized.slot.0 > previous_finalized_slot.0 {
        prune_stale_attestation_data(store);
    }
    Ok(())
}

/// Rejects a body that files the same vote twice.
///
/// Collapsing the aggregates to their distinct data exposes any repeat: fewer distinct
/// entries than aggregates means one was sent twice. This is the wire-level prohibition
/// only — the transition separately bounds how many distinct entries a body may carry.
fn reject_duplicate_attestation_data(block: &Block) -> Result<(), RejectionReason> {
    let attestations = &block.body.attestations;
    let distinct: HashSet<&AttestationData> = attestations
        .iter()
        .map(|attestation| &attestation.data)
        .collect();

    if distinct.len() == attestations.len() {
        Ok(())
    } else {
        Err(RejectionReason::DuplicateAttestationData)
    }
}

/// Files each vote the block carries into the counted pool, with no proof behind it.
///
/// A block's merged proof is never split back into per-vote proofs, so the entries start
/// empty and the votes add no head weight of their own. The per-vote proofs arrive on the
/// gossip path instead, which defers a block-carried vote's weight by up to one slot.
///
/// An existing entry keeps the proofs it already has: the same vote reaching the store twice,
/// once by gossip and once inside a block, must not lose the proof that gave it weight.
fn seed_block_votes(store: &mut Store, block: &Block) {
    for attestation in block.body.attestations.iter() {
        store
            .latest_known_aggregated_payloads
            .entry(attestation.data)
            .or_default();
    }
}

/// Recomputes the head, and with it the finalized checkpoint the head's own state names.
///
/// The walk starts at the justified root and descends to the heaviest leaf, so the head is
/// always a descendant of that root.
///
/// The finalized checkpoint is then re-derived by climbing from the new head to its ancestor
/// at the slot the head's post-state finalized. Re-deriving it from the head is what makes
/// pruning sound: a finalized checkpoint that drifted off the head's chain would prune votes
/// that still matter. Where that ancestor cannot be resolved — a checkpoint-sync anchor
/// stores no block below itself — the trusted checkpoint stays rather than an unresolved
/// root being published.
pub fn update_head(store: &mut Store) {
    let votes = latest_votes(
        &store.latest_known_aggregated_payloads,
        store.latest_finalized.slot,
    );
    store.head = lmd_ghost_head(store, store.latest_justified.root, &votes, None);

    let Some(head_state) = store.states.get(&store.head) else {
        return;
    };
    let finalized_slot = head_state.latest_finalized.slot;
    if let Some(root) = store.ancestor_at_slot(store.head, finalized_slot) {
        store.latest_finalized = Checkpoint {
            root,
            slot: finalized_slot,
        };
    }
}
