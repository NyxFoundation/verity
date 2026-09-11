//! What a validator should vote for, given the store's view.
//!
//! Target selection and the vote built around it. Signing that vote needs a key and a
//! signature library, neither of which this crate has (see the crate docs), so the duty stops
//! at the unsigned [`AttestationData`].
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/validator_duties.py`, read at
//! commit `0b7d33ec`, plus leanSpec PR #1207 (issue #1206): the vote's target is raised to
//! the source when the walk lands behind it, which every lean client already did.

use verity_types::config::JUSTIFICATION_LOOKBACK_SLOTS;
use verity_types::primitives::ZERO_HASH;
use verity_types::{AttestationData, Bytes32, Checkpoint, Slot};

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
/// The target walk is bounded by the safe target and the finalized checkpoint, never by the
/// justified one, so on a sparse chain the justifiability walk can land behind the source
/// (Verity issue #44 saw the head chain 62 → 59 → 51 → 50 → 49 → 45 → 42 with finalized
/// 42 and justified 49: three steps reach 50, neither 50 nor 49 is justifiable after 42,
/// and the walk stops at 45). A vote with its source after its target is one no peer admits,
/// so the target is raised to the justified checkpoint instead: it is on the head chain, it
/// is justifiable, and the vote keeps its weight in fork choice rather than being dropped.
#[must_use = "this produces the vote; signing it is the validator client's job"]
pub fn attestation_data(store: &Store, slot: Slot) -> AttestationData {
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

    let target = if source.slot.0 > target.slot.0 {
        source
    } else {
        target
    };

    AttestationData {
        slot,
        head,
        target,
        source,
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use verity_types::{Block, Bytes32, Checkpoint, Slot};

    use crate::fork_choice::store::Store;
    use crate::fork_choice::testing::anchored_on_genesis;
    use crate::merkle::hash_tree_root;

    use super::{attestation_data, attestation_target};

    /// The head chain seen in the three-node devnet run behind issue #44, laid over a genesis
    /// store without state transitions: the target walk reads only slots and parent links.
    ///
    /// Finalized at 42, safe target at 42, and the head state's justified where the caller
    /// says. Returns the store and the root of each slot.
    fn devnet_chain(justified_slot: u64) -> (Store, HashMap<u64, Bytes32>) {
        let (mut store, genesis) = anchored_on_genesis(3);
        let anchor_root = store.head;
        let mut roots = HashMap::new();
        let mut parent_root = anchor_root;
        for slot in [42, 45, 49, 50, 51, 59, 62] {
            let block = Block {
                slot: Slot(slot),
                parent_root,
                state_root: store.blocks[&anchor_root].state_root,
                ..Block::default()
            };
            parent_root = hash_tree_root(&block);
            store.blocks.insert(parent_root, block);
            roots.insert(slot, parent_root);
        }

        let finalized = Checkpoint {
            root: roots[&42],
            slot: Slot(42),
        };
        let justified = Checkpoint {
            root: roots[&justified_slot],
            slot: Slot(justified_slot),
        };
        let mut head_state = genesis;
        head_state.latest_justified = justified;
        head_state.latest_finalized = finalized;

        store.head = parent_root;
        store.safe_target = roots[&42];
        store.latest_finalized = finalized;
        store.latest_justified = justified;
        store.states.insert(parent_root, head_state);
        (store, roots)
    }

    #[test]
    fn should_raise_the_target_to_the_source_when_the_walk_lands_behind_it() {
        // Three steps from 62 reach 50; neither 50 nor 49 is justifiable after 42 (deltas 8
        // and 7), so the walk stops at 45 while the head's justified sits at 49.
        let (store, roots) = devnet_chain(49);
        let justified = Checkpoint {
            root: roots[&49],
            slot: Slot(49),
        };

        assert_eq!(
            attestation_target(&store),
            Checkpoint {
                root: roots[&45],
                slot: Slot(45),
            }
        );

        let data = attestation_data(&store, Slot(63));

        assert_eq!(data.source, justified);
        assert_eq!(data.target, justified);
        assert_eq!(
            data.head,
            Checkpoint {
                root: roots[&62],
                slot: Slot(62),
            }
        );
    }

    #[test]
    fn should_keep_the_target_when_it_sits_at_or_after_the_source() {
        let (store, roots) = devnet_chain(45);

        let data = attestation_data(&store, Slot(63));

        assert_eq!(data.source.slot, Slot(45));
        assert_eq!(
            data.target,
            Checkpoint {
                root: roots[&45],
                slot: Slot(45),
            }
        );
    }
}
