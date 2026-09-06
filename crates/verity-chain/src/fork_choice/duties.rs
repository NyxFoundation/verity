//! What a validator should vote for, given the store's view.
//!
//! Target selection and the vote built around it. Signing that vote needs a key and a
//! signature library, neither of which this crate has (see the crate docs), so the duty stops
//! at the unsigned [`AttestationData`].
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/validator_duties.py`, read at
//! commit `0588c2d215a955a516378677a92db2a5666802f3`.

use verity_types::config::JUSTIFICATION_LOOKBACK_SLOTS;
use verity_types::primitives::ZERO_HASH;
use verity_types::{AttestationData, Bytes32, Checkpoint, Slot};

use crate::error::RejectionReason;
use crate::fork_choice::store::Store;
use crate::justification::is_justifiable_after;

/// The checkpoint a validator should name as its attestation target.
///
/// The walk starts at the head and steps back, balancing two pulls. Advancing the head is
/// what moves the chain forward; staying at or behind the safe target is what keeps the vote
/// from backing something that can still disappear. The first loop gives the head at most
/// [`JUSTIFICATION_LOOKBACK_SLOTS`] steps back toward that bound, and the second keeps
/// stepping until the slot is one that may actually be justified.
///
/// Neither walk crosses the finalized boundary. When the safe target has fallen behind
/// finalization, the finalized slot becomes the lower bound instead, so target selection
/// never inspects a slot below it.
#[must_use = "this chooses the target; casting the vote is the validator client's job"]
pub fn attestation_target(store: &Store) -> Checkpoint {
    let finalized_slot = store.latest_finalized.slot;
    let safe_target_slot = slot_of(store, store.safe_target).unwrap_or(finalized_slot);
    let lower_bound_slot = Slot(safe_target_slot.0.max(finalized_slot.0));

    let mut target_root = store.head;
    for _ in 0..JUSTIFICATION_LOOKBACK_SLOTS {
        let Some(slot) = slot_of(store, target_root) else {
            break;
        };
        if slot.0 <= lower_bound_slot.0 {
            break;
        }
        let parent = parent_of(store, target_root);
        if parent == target_root {
            break;
        }
        target_root = parent;
    }

    while let Some(slot) = slot_of(store, target_root) {
        if slot.0 <= finalized_slot.0 || is_justifiable_after(slot, finalized_slot) {
            break;
        }
        let parent = parent_of(store, target_root);
        if parent == target_root {
            break;
        }
        target_root = parent;
    }

    Checkpoint {
        root: target_root,
        slot: slot_of(store, target_root).unwrap_or(finalized_slot),
    }
}

/// The vote a validator should cast at `slot`.
///
/// The head is named as observed, the target comes from [`attestation_target`], and the
/// source is the *head chain's* own justified checkpoint rather than the store's. The store
/// can advance its justified checkpoint from a minority fork the head never extended, and a
/// vote sourced there would name a checkpoint off the chain it is extending.
///
/// A genesis state carries the zero hash as its justified root, which names no block. The
/// head stands in for it, which at that point is the anchor at slot 0 — the block the
/// checkpoint means.
///
/// # Errors
///
/// [`RejectionReason::SourceAfterTarget`] when the source ends up ahead of the target.
/// leanSpec asserts here; Runtime Shell code must not panic, so the impossible vote is
/// returned as a rejection instead of being cast.
#[must_use = "this produces the vote; signing it is the validator client's job"]
pub fn attestation_data(store: &Store, slot: Slot) -> Result<AttestationData, RejectionReason> {
    let head = Checkpoint {
        root: store.head,
        slot: slot_of(store, store.head).unwrap_or(store.latest_finalized.slot),
    };
    let target = attestation_target(store);

    let mut source = store
        .states
        .get(&store.head)
        .map_or(store.latest_justified, |state| state.latest_justified);
    if source.root == ZERO_HASH {
        source = Checkpoint {
            root: store.head,
            slot: source.slot,
        };
    }

    if source.slot.0 > target.slot.0 {
        return Err(RejectionReason::SourceAfterTarget);
    }

    Ok(AttestationData {
        slot,
        head,
        target,
        source,
    })
}

/// The slot of a known block, or `None` where the root is not in the local view.
fn slot_of(store: &Store, root: Bytes32) -> Option<Slot> {
    store.blocks.get(&root).map(|block| block.slot)
}

/// The parent of a known block, or the root itself where the walk has left the known tree.
///
/// Both walks above treat an unchanged root as the end of the chain, which is what stops
/// them from climbing into an unknown branch or circling a block that names itself.
fn parent_of(store: &Store, root: Bytes32) -> Bytes32 {
    store
        .blocks
        .get(&root)
        .map_or(root, |block| block.parent_root)
}
