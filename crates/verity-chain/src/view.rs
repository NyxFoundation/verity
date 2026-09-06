//! The immutable snapshot every reader of the chain sees.
//!
//! `docs/design/concurrency.md` gives the consensus state one owning task and makes this
//! snapshot the entire read path: there is no query channel into that task, so validator
//! duties, the verification stage, and every future reader answer from the `ChainView`
//! current at their own read time.
//!
//! # What it must carry
//!
//! Two clauses, and they are the contract — the field layout below is not:
//!
//! 1. the head and the latest justified and finalized checkpoints;
//! 2. enough of the block tree and post-states to resolve a validator registry by block root.
//!
//! The second is what the verification stage needs: a block's proof verifies against its
//! parent's post-state registry, and an attestation's signature against its target's. So the
//! snapshot covers the unfinalized tree plus the finalized anchor — exactly what fork choice
//! operates on. Anything older is a `verity-db` read, not a snapshot miss.
//!
//! # Why it wraps the store
//!
//! The two clauses above *are* the fork-choice store, minus nothing that matters, and every
//! question a reader asks of a snapshot is already a pure function over it. Wrapping keeps
//! one definition of each answer instead of two; the store stays private so the snapshot's
//! shape is not the contract.

use std::collections::{HashMap, HashSet};

use verity_types::{
    AttestationData, Block, Bytes32, Checkpoint, Interval, SingleMessageAggregate, Slot, State,
    Validators,
};

use crate::error::RejectionReason;
use crate::fork_choice::duties::{attestation_data, attestation_target};
use crate::fork_choice::store::{AttestationSignatureEntry, Store};

/// An immutable view of the chain, as of one completed import.
///
/// A reader holding one can never observe a half-applied mutation: it is a value, published
/// after an import has fully completed, and it has no way to mutate anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainView {
    store: Store,
}

impl ChainView {
    /// Snapshots a store.
    #[must_use = "this builds the snapshot; publishing it is the chain task's job"]
    pub fn of(store: &Store) -> Self {
        Self {
            store: store.clone(),
        }
    }

    /// Intervals elapsed since genesis, as the chain task's clock has ticked them.
    #[must_use]
    pub const fn time(&self) -> Interval {
        self.store.time
    }

    /// The slot the snapshot's clock sits in.
    #[must_use]
    pub const fn slot(&self) -> Slot {
        self.store.current_slot()
    }

    /// The block fork choice selects.
    #[must_use]
    pub const fn head(&self) -> Bytes32 {
        self.store.head
    }

    /// The head as a checkpoint, which is the form a vote names it in.
    #[must_use]
    pub fn head_checkpoint(&self) -> Checkpoint {
        Checkpoint {
            root: self.store.head,
            slot: self
                .store
                .blocks
                .get(&self.store.head)
                .map_or(Slot(0), |block| block.slot),
        }
    }

    /// The deepest block a supermajority of this slot's voters already back.
    #[must_use]
    pub const fn safe_target(&self) -> Bytes32 {
        self.store.safe_target
    }

    /// The highest justified checkpoint.
    #[must_use]
    pub const fn latest_justified(&self) -> Checkpoint {
        self.store.latest_justified
    }

    /// The highest finalized checkpoint.
    #[must_use]
    pub const fn latest_finalized(&self) -> Checkpoint {
        self.store.latest_finalized
    }

    /// A block in the unfinalized tree, or the anchor.
    #[must_use]
    pub fn block(&self, root: Bytes32) -> Option<&Block> {
        self.store.blocks.get(&root)
    }

    /// Every block root in view — what a proposer may stand behind a vote for.
    #[must_use]
    pub fn known_block_roots(&self) -> HashSet<Bytes32> {
        self.store.blocks.keys().copied().collect()
    }

    /// The post-state of a block in view.
    #[must_use]
    pub fn state(&self, root: Bytes32) -> Option<&State> {
        self.store.states.get(&root)
    }

    /// The post-state of the head.
    #[must_use]
    pub fn head_state(&self) -> Option<&State> {
        self.store.states.get(&self.store.head)
    }

    /// The validator registry a signature over a block or vote rooted at `root` verifies
    /// against.
    #[must_use]
    pub fn validators(&self, root: Bytes32) -> Option<&Validators> {
        self.state(root).map(|state| &state.validators)
    }

    /// The checkpoint a validator should name as its attestation target.
    #[must_use]
    pub fn attestation_target(&self) -> Checkpoint {
        attestation_target(&self.store)
    }

    /// The vote a validator should cast at `slot`.
    ///
    /// # Errors
    ///
    /// [`RejectionReason::SourceAfterTarget`] when the head's justified checkpoint sits ahead
    /// of the target the walk selected, which is a vote no peer would admit.
    #[must_use = "this produces the vote; signing it is the validator's job"]
    pub fn attestation_data(&self, slot: Slot) -> Result<AttestationData, RejectionReason> {
        attestation_data(&self.store, slot)
    }

    /// Per-validator signatures an aggregator has collected, grouped by the vote they sign.
    #[must_use]
    pub const fn attestation_signatures(
        &self,
    ) -> &HashMap<AttestationData, HashSet<AttestationSignatureEntry>> {
        &self.store.attestation_signatures
    }

    /// Proofs gathered this slot, which carry no weight until an acceptance tick promotes
    /// them.
    #[must_use]
    pub const fn new_aggregated_payloads(
        &self,
    ) -> &HashMap<AttestationData, HashSet<SingleMessageAggregate>> {
        &self.store.latest_new_aggregated_payloads
    }

    /// Proofs that count toward fork-choice weight, and the pool a proposer fills from.
    #[must_use]
    pub const fn known_aggregated_payloads(
        &self,
    ) -> &HashMap<AttestationData, HashSet<SingleMessageAggregate>> {
        &self.store.latest_known_aggregated_payloads
    }
}

#[cfg(test)]
mod tests {
    use verity_types::{Interval, Slot};

    use crate::fork_choice::testing::{anchored_on_genesis, import_at};

    use super::ChainView;

    #[test]
    fn should_answer_from_the_store_it_snapshotted() {
        let (mut store, genesis) = anchored_on_genesis(4);
        let block = import_at(&mut store, &genesis, 1);

        let view = ChainView::of(&store);
        assert_eq!(view.head(), store.head);
        assert_eq!(view.head_checkpoint().slot, Slot(1));
        assert_eq!(view.block(store.head).map(|b| b.slot), Some(block.slot));
        assert!(view.head_state().is_some());
        assert_eq!(
            view.validators(store.head)
                .map(|validators| validators.len()),
            Some(4)
        );
    }

    #[test]
    fn should_not_follow_the_store_once_it_has_been_snapshotted() {
        let (mut store, genesis) = anchored_on_genesis(4);
        let view = ChainView::of(&store);

        import_at(&mut store, &genesis, 1);
        store.time = Interval(9);

        assert_ne!(view.head(), store.head);
        assert_eq!(view.time(), Interval(0));
    }
}
