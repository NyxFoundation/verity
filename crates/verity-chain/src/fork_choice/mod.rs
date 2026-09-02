//! Fork choice: the store, and the decisions that move its head.
//!
//! # What the store is
//!
//! The state transition answers "is this block valid, and what does it produce". Fork choice
//! answers "which of the valid chains is the one". It needs memory the transition does not —
//! every block above finalization, their post-states, and the votes cast over them — and
//! that memory is the [`Store`].
//!
//! # Why the store is mutated in place
//!
//! Everything else in this crate is a pure function returning a fresh value, and the store
//! deliberately is not. It is a long-lived aggregate with one writer, not a value passed
//! between them, and it holds a [`verity_types::State`] per unfinalized block; copying it per
//! imported block would cost `O(chain)` per block and buy nothing. What the copy guaranteed
//! is kept as a contract instead: **an entry point that returns `Err` leaves the store
//! exactly as it found it**.
//!
//! # Where the cryptography went
//!
//! leanSpec verifies signatures inside three of these operations. This crate has no
//! cryptographic dependency (see the crate docs), so each of the three is split at exactly
//! that point and the caller verifies in between:
//!
//! | leanSpec | here | the caller supplies |
//! |---|---|---|
//! | `on_block` | [`on_block`] | the block proof check |
//! | `on_gossip_attestation` | [`validate_attestation_signer`] + [`record_attestation_signature`] | the XMSS verify |
//! | `on_gossip_aggregated_attestation` | [`record_aggregated_payload`] | the aggregate verify |
//!
//! The aggregator duty at interval 2 is absent for the same reason — see [`timeline`].

pub mod attestation;
pub mod block;
pub mod duties;
pub mod prune;
pub mod store;
pub mod timeline;
pub mod weights;

pub use attestation::{
    record_aggregated_payload, record_attestation_signature, validate_attestation,
    validate_attestation_signer,
};
pub use block::{on_block, update_head};
pub use duties::{attestation_data, attestation_target};
pub use prune::prune_stale_attestation_data;
pub use store::{AttestationSignature, AttestationSignatureEntry, Store};
pub use timeline::{accept_new_attestations, on_tick, update_safe_target};
pub use weights::{block_weights, latest_votes, lmd_ghost_head, participants};

#[cfg(test)]
pub(crate) mod testing {
    //! Store builders shared by the unit tests of this module's submodules.

    use verity_types::{Block, Slot, State, ValidatorIndex};

    use crate::merkle::hash_tree_root;
    use crate::slot_clock::intervals_at_slot_start;
    use crate::state_transition::testing::{empty_block_at, genesis_with};
    use crate::state_transition::{process_block, process_slots};

    use super::{Store, on_block};

    /// A store anchored on a genesis with `count` validators, its clock at slot 0.
    pub(crate) fn anchored_on_genesis(count: u64) -> (Store, State) {
        let genesis = genesis_with(count);
        let mut anchor = empty_block_at(&genesis, 0);
        anchor.state_root = hash_tree_root(&genesis);
        anchor.parent_root = genesis.latest_block_header.parent_root;

        let store = Store::new(&genesis, &anchor, Some(ValidatorIndex(0)))
            .expect("the anchor commits to its own state");
        (store, genesis)
    }

    /// An empty block at `slot`, self-consistent against the state it builds on.
    pub(crate) fn block_on(state: &State, slot: u64) -> Block {
        let advanced = process_slots(state, Slot(slot)).expect("the slot is ahead of the state");
        let mut block = empty_block_at(&advanced, slot);
        block.state_root = hash_tree_root(
            &process_block(&advanced, &block).expect("an empty block on its own parent"),
        );
        block
    }

    /// Imports `slot`'s empty block, ticking the clock to admit it first.
    pub(crate) fn import_at(store: &mut Store, state: &State, slot: u64) -> Block {
        let block = block_on(state, slot);
        store.time = intervals_at_slot_start(block.slot);
        on_block(store, &block).expect("a self-consistent block on a known parent");
        block
    }
}

#[cfg(test)]
mod tests {
    use verity_types::{Checkpoint, Interval, Slot};

    use crate::error::RejectionReason;
    use crate::merkle::hash_tree_root;

    use super::testing::{anchored_on_genesis, block_on, import_at};
    use super::{Store, on_block};

    #[test]
    fn should_reject_an_anchor_whose_block_does_not_commit_to_its_state() {
        let (_, genesis) = anchored_on_genesis(4);
        let mut anchor = crate::state_transition::testing::empty_block_at(&genesis, 0);
        anchor.state_root = [7u8; 32];

        assert_eq!(
            Store::new(&genesis, &anchor, None).unwrap_err(),
            RejectionReason::AnchorStateRootMismatch
        );
    }

    #[test]
    fn should_start_the_clock_at_the_anchor_slot() {
        let (store, _) = anchored_on_genesis(4);
        assert_eq!(store.time, Interval(0));
        assert_eq!(store.head, store.latest_finalized.root);
        assert_eq!(store.latest_justified, store.latest_finalized);
    }

    #[test]
    fn should_leave_the_store_untouched_when_a_block_is_rejected() {
        let (mut store, genesis) = anchored_on_genesis(4);
        let before = store.clone();

        // The clock still sits at slot 0, so a block two slots ahead is past the horizon.
        let block = block_on(&genesis, 2);
        assert_eq!(
            on_block(&mut store, &block),
            Err(RejectionReason::BlockTooFarInFuture)
        );
        assert_eq!(store, before);
    }

    #[test]
    fn should_leave_the_store_untouched_when_the_parent_is_unknown() {
        let (mut store, genesis) = anchored_on_genesis(4);
        let before = store.clone();

        let mut block = block_on(&genesis, 1);
        block.parent_root = [9u8; 32];
        store.time = Interval(5);
        assert_eq!(
            on_block(&mut store, &block),
            Err(RejectionReason::UnknownParentBlock)
        );
        assert_eq!(store.blocks, before.blocks);
        assert_eq!(store.states, before.states);
        assert_eq!(store.latest_justified, before.latest_justified);
    }

    #[test]
    fn should_accept_a_block_already_in_the_store_without_changing_it() {
        let (mut store, genesis) = anchored_on_genesis(4);
        let block = import_at(&mut store, &genesis, 1);
        let after_first = store.clone();

        assert_eq!(on_block(&mut store, &block), Ok(()));
        assert_eq!(store, after_first);
    }

    #[test]
    fn should_follow_the_chain_when_no_vote_has_any_weight() {
        let (mut store, genesis) = anchored_on_genesis(4);
        let first = import_at(&mut store, &genesis, 1);

        assert_eq!(store.head, hash_tree_root(&first));
        assert_eq!(store.blocks.len(), 2);
    }

    #[test]
    fn should_keep_the_justified_checkpoint_when_a_candidate_ties_on_slot() {
        let (mut store, _) = anchored_on_genesis(4);
        let original = store.latest_justified;
        let tie = Checkpoint {
            root: [1u8; 32],
            slot: original.slot,
        };

        store.latest_justified = crate::justification::advance_checkpoint(original, tie);
        assert_eq!(store.latest_justified, original);
    }

    #[test]
    fn should_report_the_slot_its_interval_clock_sits_in() {
        let (mut store, _) = anchored_on_genesis(4);
        store.time = Interval(12);
        assert_eq!(store.current_slot(), Slot(2));
    }
}
