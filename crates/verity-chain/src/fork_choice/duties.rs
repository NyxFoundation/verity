//! What a validator should vote for, given the store's view.
//!
//! Only the target selection lives here. Producing and signing the attestation itself needs
//! a key and a signature library, neither of which this crate has (see the crate docs).
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/validator_duties.py`, read at
//! commit `0588c2d215a955a516378677a92db2a5666802f3`.

use verity_types::config::JUSTIFICATION_LOOKBACK_SLOTS;
use verity_types::{Bytes32, Checkpoint, Slot};

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
