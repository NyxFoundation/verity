//! The only path by which anything becomes durable.
//!
//! There are four commits, and each one is a single cross-table batch that applies whole or
//! not at all: the anchor a chain starts from, a processed block, an interval-3 safe target,
//! and the interval tick that moves the head. Nothing else writes.
//!
//! Every commit checks its own internal consistency **before** the batch is handed to the
//! backend, so an inconsistent commit is refused rather than made durable and repaired later.
//! The checks are the ones `docs/design/storage.md` names: the header roots to the block
//! root, the body roots to the header, the state roots to the header, and the parent links
//! agree.
//!
//! Transcribed from `docs/design/storage.md`, "Writer, batches, and restart".

use std::collections::HashSet;

use verity_types::config::INTERVALS_PER_SLOT;
use verity_types::primitives::{Bytes32, Interval, Slot, ValidatorIndex};
use verity_types::{
    AttestationData, BlockBody, BlockHeader, Checkpoint, MultiMessageAggregate, State,
};

use crate::backend::{Durability, StorageBackend, WriteBatch};
use crate::column::ColumnFamily;
use crate::diff::{StateDiff, crosses_snapshot_boundary};
use crate::error::StorageError;
use crate::key;
use crate::merkle::hash_tree_root;
use crate::metadata::MetadataKey;
use crate::repository::{Repository, put_metadata};
use crate::schema::{SCHEMA_VERSION, ssz_schema_digest};
use crate::votes;

use libssz::SszEncode;

/// The state a chain is started or restarted from.
///
/// The anchor block's header is not passed in, because it is not free to differ from the
/// state: [`stored_header`] derives it, and `block_root` is checked against what that
/// derivation produces. Genesis and a checkpoint-sync anchor take the same path. Only the
/// body is genuinely extra, and only when the operator fetched one — an anchor without its
/// body is serviceable, it is simply not servable over `BlocksByRange`.
#[derive(Debug, Clone, Copy)]
pub struct AnchorCommit<'a> {
    /// Root of the anchor block.
    pub block_root: Bytes32,
    /// The state the anchor pins.
    pub state: &'a State,
    /// The anchor block's body, when the operator supplied one.
    pub body: Option<&'a BlockBody>,
    /// First slot this node may serve proof-bearing history from.
    ///
    /// An anchor above genesis has no history below itself, so range service starts here and
    /// not at slot zero.
    pub served_from_slot: Slot,
}

/// A block that has passed the state transition, with the state it produced.
///
/// As with [`AnchorCommit`], the header is derived rather than supplied: every one of its
/// fields is already fixed by the post-state, so accepting one would only create a way for a
/// caller to store a header that disagrees with the state beside it.
#[derive(Debug, Clone, Copy)]
pub struct BlockCommit<'a> {
    /// Root of the block.
    pub block_root: Bytes32,
    /// The block's body.
    pub body: &'a BlockBody,
    /// The single proof covering the block's attestations and its proposer signature.
    pub proof: &'a MultiMessageAggregate,
    /// The state the block produced.
    pub post_state: &'a State,
    /// The slot of the block's parent, which decides whether a snapshot is due.
    pub parent_slot: Slot,
}

/// The header a block is stored under, derived from the state it produced.
///
/// A post-state carries the block's header with its `state_root` still empty —
/// `process_block_header` leaves it for the next slot to fill, and `process_slots` fills it
/// with the root of the state as of that block. Filling it here reproduces the header the
/// chain itself roots, which is the root the block's children name as their parent. Deriving
/// it is therefore not a convenience: it is the definition.
#[must_use]
pub fn stored_header(post_state: &State) -> BlockHeader {
    BlockHeader {
        state_root: hash_tree_root(post_state),
        ..post_state.latest_block_header
    }
}

/// The canonical-index edits a head move implies.
///
/// Two lists rather than one, because they are applied in order and the order matters: a slot
/// can both leave and rejoin — the same height on a different branch — and the upsert has to
/// land after the delete.
#[derive(Debug, Clone, Default)]
struct CanonicalSwitch {
    /// Slots leaving the canonical chain.
    leaving: Vec<Slot>,
    /// Slots joining it, with the root that now occupies each.
    joining: Vec<(Slot, Bytes32)>,
}

/// The interval tick that moves the node's view forward.
///
/// Interval 4 is the one that merges votes; the others carry only the head recomputation and
/// the interval marker. Both go through here so that a vote-table change, the metadata it
/// affects, and `last_processed_interval` are always one batch — which is what makes a
/// restart able to tell which interval finished.
#[derive(Debug, Clone, Copy)]
pub struct TickCommit {
    /// The recomputed head block root.
    pub head: Bytes32,
    /// The head-derived justified checkpoint.
    pub latest_justified: Checkpoint,
    /// The head-derived finalized checkpoint.
    pub latest_finalized: Checkpoint,
    /// The interval whose events this batch completes.
    pub interval: Interval,
    /// Whether pending votes merge into the counted map and the pending map is cleared.
    pub merge_pending_votes: bool,
}

const fn tick_durability(stored: Option<Checkpoint>, next: Checkpoint) -> Durability {
    if let Some(stored) = stored
        && next.slot.0 <= stored.slot.0
    {
        return Durability::Buffered;
    }
    Durability::Synced
}

fn queue_switch_edits(batch: &mut WriteBatch, switch: CanonicalSwitch) {
    for slot in switch.leaving {
        batch.queue_delete(ColumnFamily::CanonicalBlocks, key::slot(slot));
    }
    for (slot, root) in switch.joining {
        batch.queue_put(
            ColumnFamily::CanonicalBlocks,
            key::slot(slot),
            root.to_vec(),
        );
    }
}

impl<B: StorageBackend> Repository<B> {
    /// Writes the anchor a chain starts from, identity included.
    ///
    /// Fsynced: everything after this is interpreted relative to it, so it must not be the
    /// commit that a power cut leaves half-written.
    ///
    /// # Errors
    ///
    /// - [`StorageError::RejectedBatch`] when the anchor block, its body, and the anchor
    ///   state do not agree with each other or with `block_root`.
    /// - [`StorageError::Backend`] when the engine fails.
    pub fn commit_anchor(&mut self, anchor: &AnchorCommit<'_>) -> Result<(), StorageError> {
        let header = stored_header(anchor.state);
        check_anchor(anchor, &header)?;
        let state_root = header.state_root;
        let slot = header.slot;

        let mut batch = WriteBatch::new();

        put_metadata(&mut batch, MetadataKey::SchemaVersion, &SCHEMA_VERSION);
        put_metadata(
            &mut batch,
            MetadataKey::ChainFingerprint,
            &self.identity().chain_fingerprint,
        );
        put_metadata(
            &mut batch,
            MetadataKey::ForkVersion,
            &self.identity().fork_version,
        );
        put_metadata(
            &mut batch,
            MetadataKey::SszSchemaDigest,
            &ssz_schema_digest(),
        );

        put_metadata(&mut batch, MetadataKey::Head, &anchor.block_root);
        put_metadata(&mut batch, MetadataKey::SafeTarget, &anchor.block_root);
        put_metadata(
            &mut batch,
            MetadataKey::LatestJustified,
            &anchor.state.latest_justified,
        );
        put_metadata(
            &mut batch,
            MetadataKey::LatestFinalized,
            &anchor.state.latest_finalized,
        );
        put_metadata(
            &mut batch,
            MetadataKey::ServedFromSlot,
            &anchor.served_from_slot.0,
        );
        // The anchor is complete at the first interval of its own slot; nothing later has run.
        put_metadata(
            &mut batch,
            MetadataKey::LastProcessedInterval,
            &(slot.0 * INTERVALS_PER_SLOT),
        );

        batch.queue_put(
            ColumnFamily::BlockHeaders,
            key::root(anchor.block_root),
            header.to_ssz(),
        );
        if let Some(body) = anchor.body {
            batch.queue_put(
                ColumnFamily::BlockBodies,
                key::root(anchor.block_root),
                body.to_ssz(),
            );
        }
        batch.queue_put(
            ColumnFamily::StateSnapshots,
            key::root(anchor.block_root),
            anchor.state.to_ssz(),
        );
        batch.queue_put(
            ColumnFamily::StateRoots,
            key::root(state_root),
            anchor.block_root.to_vec(),
        );
        batch.queue_put(
            ColumnFamily::CanonicalBlocks,
            key::slot(slot),
            anchor.block_root.to_vec(),
        );
        batch.queue_put(
            ColumnFamily::ForkChoiceBlocks,
            key::slot_and_root(slot, anchor.block_root),
            header.parent_root.to_vec(),
        );

        self.commit(batch, Durability::Synced)
    }

    /// Writes a processed block: its data, its state delta, and its index entries.
    ///
    /// The node's view — head, canonical index, checkpoints — is deliberately untouched. A
    /// block on a side branch is processed exactly like one on the canonical chain, and only
    /// [`Repository::commit_tick`] decides which of them the node is following.
    ///
    /// # Errors
    ///
    /// - [`StorageError::RejectedBatch`] when the block, its body, and its post-state do not
    ///   agree with each other or with `block_root`.
    /// - [`StorageError::Backend`] when the engine fails.
    pub fn commit_block(&mut self, commit: &BlockCommit<'_>) -> Result<(), StorageError> {
        let header = stored_header(commit.post_state);
        check_block(commit, &header)?;
        let slot = header.slot;
        let root = commit.block_root;

        let mut batch = WriteBatch::new();
        batch.queue_put(ColumnFamily::BlockHeaders, key::root(root), header.to_ssz());
        batch.queue_put(
            ColumnFamily::BlockBodies,
            key::root(root),
            commit.body.to_ssz(),
        );
        batch.queue_put(
            ColumnFamily::BlockProofs,
            key::slot_and_root(slot, root),
            commit.proof.to_ssz(),
        );
        batch.queue_put(
            ColumnFamily::StateDiffs,
            key::root(root),
            StateDiff::from_post_state(commit.post_state).to_ssz(),
        );
        if crosses_snapshot_boundary(commit.parent_slot, slot) {
            batch.queue_put(
                ColumnFamily::StateSnapshots,
                key::root(root),
                commit.post_state.to_ssz(),
            );
        }
        batch.queue_put(
            ColumnFamily::StateRoots,
            key::root(header.state_root),
            root.to_vec(),
        );
        batch.queue_put(
            ColumnFamily::ForkChoiceBlocks,
            key::slot_and_root(slot, root),
            header.parent_root.to_vec(),
        );

        // Not fsynced. A block lost to a power cut is reacquired over the network; a block
        // wrongly reported as durable is not recoverable at all, and the batch's atomicity is
        // what rules that out, not the fsync.
        self.commit(batch, Durability::Buffered)
    }

    /// Records verified block-external aggregate votes as pending.
    ///
    /// A vote only replaces the stored one under the total order of [`crate::votes`], so the
    /// surviving vote does not depend on the order proofs happened to arrive in — which is
    /// what makes the reduction survive a restart unchanged.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] on a damaged vote row, [`StorageError::Backend`] on engine
    /// failure.
    pub fn record_pending_votes(
        &mut self,
        votes: &[(ValidatorIndex, AttestationData)],
        interval: Interval,
    ) -> Result<(), StorageError> {
        let mut batch = WriteBatch::new();
        for (validator, vote) in votes {
            let stored = self.pending_vote(*validator)?;
            if !votes::replaces(vote, stored.as_ref()) {
                continue;
            }
            batch.queue_put(
                ColumnFamily::PendingVotes,
                key::validator(*validator),
                vote.to_ssz(),
            );
        }
        if batch.is_empty() {
            return Ok(());
        }
        put_metadata(&mut batch, MetadataKey::LastProcessedInterval, &interval.0);
        self.commit(batch, Durability::Buffered)
    }

    /// Commits the interval-3 safe target.
    ///
    /// # Errors
    ///
    /// [`StorageError::Backend`] when the engine fails.
    pub fn commit_safe_target(
        &mut self,
        root: Bytes32,
        interval: Interval,
    ) -> Result<(), StorageError> {
        let mut batch = WriteBatch::new();
        put_metadata(&mut batch, MetadataKey::SafeTarget, &root);
        put_metadata(&mut batch, MetadataKey::LastProcessedInterval, &interval.0);
        self.commit(batch, Durability::Buffered)
    }

    /// Commits an interval tick: the head, the canonical index it implies, and — at interval
    /// 4 — the vote merge.
    ///
    /// Fsynced when finalization advances. That is the one commit whose loss would let the
    /// node re-derive a *different* finalized checkpoint after a restart, so it is the one
    /// that pays for the disk flush.
    ///
    /// # Errors
    ///
    /// - [`StorageError::MissingRow`] when walking to the common ancestor reaches a block
    ///   whose header is absent, which means the fork-choice index and the block store
    ///   disagree.
    /// - [`StorageError::AncestryCycle`] or [`StorageError::NonDecreasingAncestry`] when
    ///   stored parent links cannot form a finite backwards walk.
    /// - [`StorageError::Backend`] when the engine fails.
    pub fn commit_tick(&mut self, tick: &TickCommit) -> Result<(), StorageError> {
        let mut batch = WriteBatch::new();

        let previous_head = self.head()?;
        if previous_head != Some(tick.head) {
            self.queue_canonical_switch(&mut batch, previous_head, tick.head)?;
        }
        if tick.merge_pending_votes {
            self.queue_vote_merge(&mut batch)?;
        }

        put_metadata(&mut batch, MetadataKey::Head, &tick.head);
        put_metadata(
            &mut batch,
            MetadataKey::LatestJustified,
            &tick.latest_justified,
        );
        put_metadata(
            &mut batch,
            MetadataKey::LatestFinalized,
            &tick.latest_finalized,
        );
        put_metadata(
            &mut batch,
            MetadataKey::LastProcessedInterval,
            &tick.interval.0,
        );

        let durability = tick_durability(self.latest_finalized()?, tick.latest_finalized);
        self.commit(batch, durability)
    }

    /// Queues the canonical-index edits that move the head from `previous` to `new_head`.
    ///
    /// Deletes are queued before upserts, so a slot that both leaves and rejoins the
    /// canonical chain — the same height on a different branch — ends up holding the new
    /// root rather than nothing.
    fn queue_canonical_switch(
        &self,
        batch: &mut WriteBatch,
        previous: Option<Bytes32>,
        new_head: Bytes32,
    ) -> Result<(), StorageError> {
        let switch = match previous {
            Some(previous) => self.canonical_switch(previous, new_head)?,
            // No previous head: nothing to unwind, and the anchor already wrote its own slot.
            None => CanonicalSwitch {
                leaving: Vec::new(),
                joining: self.ancestry_to_canonical(new_head)?,
            },
        };
        queue_switch_edits(batch, switch);
        Ok(())
    }

    /// The edits moving the head from `previous` to `new_head` implies, found by walking both
    /// back to their common ancestor.
    ///
    /// The walk is bounded by the chain itself: each step moves one of the two pointers
    /// strictly backwards, and a missing header stops it as corruption rather than looping.
    fn canonical_switch(
        &self,
        previous: Bytes32,
        new_head: Bytes32,
    ) -> Result<CanonicalSwitch, StorageError> {
        let mut old = (previous, self.required_header(previous)?);
        let mut new = (new_head, self.required_header(new_head)?);
        let mut old_visited = HashSet::from([previous]);
        let mut new_visited = HashSet::from([new_head]);
        let mut switch = CanonicalSwitch::default();

        while old.0 != new.0 {
            if old.1.slot.0 >= new.1.slot.0 {
                switch.leaving.push(old.1.slot);
                old = self.required_parent(old.0, &old.1, &mut old_visited)?;
            } else {
                switch.joining.push((new.1.slot, new.0));
                new = self.required_parent(new.0, &new.1, &mut new_visited)?;
            }
        }
        Ok(switch)
    }

    /// Every `(slot, root)` from `head` back to the first block already recorded as canonical
    /// at its own slot.
    fn ancestry_to_canonical(&self, head: Bytes32) -> Result<Vec<(Slot, Bytes32)>, StorageError> {
        let mut joining = Vec::new();
        let mut cursor = (head, self.required_header(head)?);
        let mut visited = HashSet::from([head]);
        loop {
            if self.canonical_root(cursor.1.slot)? == Some(cursor.0) {
                return Ok(joining);
            }
            joining.push((cursor.1.slot, cursor.0));
            cursor = self.required_parent(cursor.0, &cursor.1, &mut visited)?;
        }
    }

    fn required_parent(
        &self,
        child: Bytes32,
        child_header: &BlockHeader,
        visited: &mut HashSet<Bytes32>,
    ) -> Result<(Bytes32, BlockHeader), StorageError> {
        self.checked_parent(child, child_header, visited)?
            .ok_or_else(|| StorageError::MissingRow {
                table: ColumnFamily::BlockHeaders,
                key: key::root(child_header.parent_root).to_vec(),
            })
    }

    fn required_header(&self, root: Bytes32) -> Result<BlockHeader, StorageError> {
        self.read_required(ColumnFamily::BlockHeaders, &key::root(root))
    }

    /// Queues the interval-4 merge: every pending vote is offered to the counted map under
    /// the same total order, and the pending map is emptied whether or not it won.
    fn queue_vote_merge(&self, batch: &mut WriteBatch) -> Result<(), StorageError> {
        for (validator, vote) in self.pending_votes()? {
            let stored = self.known_vote(validator)?;
            if votes::replaces(&vote, stored.as_ref()) {
                batch.queue_put(
                    ColumnFamily::KnownVotes,
                    key::validator(validator),
                    vote.to_ssz(),
                );
            }
            batch.queue_delete(ColumnFamily::PendingVotes, key::validator(validator));
        }
        Ok(())
    }
}

/// Checks an anchor commit against the header derived from its state.
fn check_anchor(anchor: &AnchorCommit<'_>, header: &BlockHeader) -> Result<(), StorageError> {
    // The anchor state must be a block's post-state, not one advanced through empty slots.
    // An advanced state hashes to something the anchor block never committed to, so the
    // derived header would carry a `state_root` no child could ever agree with.
    if anchor.state.slot != anchor.state.latest_block_header.slot {
        return Err(StorageError::RejectedBatch(
            "anchor state has been advanced past its own block's slot",
        ));
    }
    if hash_tree_root(header) != anchor.block_root {
        return Err(StorageError::RejectedBatch(
            "anchor block does not commit to the anchor state it was handed with",
        ));
    }
    if let Some(body) = anchor.body
        && hash_tree_root(body) != header.body_root
    {
        return Err(StorageError::RejectedBatch(
            "anchor body does not root to the anchor header's body root",
        ));
    }
    Ok(())
}

/// Checks a block commit against the header derived from its post-state.
fn check_block(commit: &BlockCommit<'_>, header: &BlockHeader) -> Result<(), StorageError> {
    if hash_tree_root(header) != commit.block_root {
        return Err(StorageError::RejectedBatch(
            "post-state does not produce the block root it was committed under",
        ));
    }
    if hash_tree_root(commit.body) != header.body_root {
        return Err(StorageError::RejectedBatch(
            "body does not root to the header's body root",
        ));
    }
    if commit.post_state.slot != header.slot {
        return Err(StorageError::RejectedBatch(
            "post-state and header disagree on the slot",
        ));
    }
    if commit.parent_slot.0 >= header.slot.0 {
        return Err(StorageError::RejectedBatch(
            "parent slot is not below the block's slot",
        ));
    }
    Ok(())
}

#[cfg(kani)]
mod harnesses {
    use libssz::SszEncode;
    use verity_types::{Checkpoint, Interval, Slot};

    use crate::backend::harness_support::RecordingBackend;
    use crate::backend::{Durability, Op, WriteBatch};
    use crate::metadata::MetadataKey;
    use crate::repository::Repository;
    use crate::schema::Identity;

    fn repository(backend: RecordingBackend) -> Repository<RecordingBackend> {
        Repository::from_backend_for_harness(
            backend,
            Identity {
                chain_fingerprint: [0; 32],
                fork_version: 0,
            },
        )
    }

    #[allow(dead_code)]
    fn put_metadata_stub<T: SszEncode>(batch: &mut WriteBatch, key: MetadataKey, _value: &T) {
        batch.queue_put(crate::ColumnFamily::Metadata, key.as_bytes(), Vec::new());
    }

    /// A safe-target update is one atomic, buffered batch containing both view mutations.
    #[kani::proof]
    #[kani::unwind(25)]
    #[kani::stub(crate::repository::put_metadata, put_metadata_stub)]
    fn a_safe_target_update_is_one_atomic_batch() {
        let mut repository = repository(RecordingBackend::default());
        let interval = Interval(kani::any());
        repository
            .commit_safe_target([7; 32], interval)
            .expect("the recording backend cannot fail");

        let writes = &repository.backend().writes;
        assert!(writes.len() == 1);
        let (batch, durability) = &writes[0];
        assert!(*durability == Durability::Buffered);
        assert!(batch.len() == 2);
        match (&batch.ops()[0], &batch.ops()[1]) {
            (
                Op::Put {
                    table: first_table,
                    key: first_key,
                    ..
                },
                Op::Put {
                    table: second_table,
                    key: second_key,
                    ..
                },
            ) => {
                assert!(*first_table == crate::ColumnFamily::Metadata);
                assert!(*second_table == crate::ColumnFamily::Metadata);
                assert!(*first_key == MetadataKey::SafeTarget.as_bytes());
                assert!(*second_key == MetadataKey::LastProcessedInterval.as_bytes());
            }
            _ => unreachable!("both safe-target fields are metadata puts"),
        }
    }

    /// A reorg deletes the old slot before inserting the new root at that same slot.
    #[kani::proof]
    #[kani::unwind(35)]
    fn canonical_switch_queues_deletes_before_puts() {
        let slot = Slot(kani::any());
        let root = [7; 32];
        let mut batch = WriteBatch::new();
        super::queue_switch_edits(
            &mut batch,
            super::CanonicalSwitch {
                leaving: vec![slot],
                joining: vec![(slot, root)],
            },
        );
        assert!(batch.len() == 2);
        match (&batch.ops()[0], &batch.ops()[1]) {
            (
                Op::Delete { key: deleted, .. },
                Op::Put {
                    key: inserted,
                    value,
                    ..
                },
            ) => {
                assert!(*deleted == slot.0.to_be_bytes());
                assert!(*inserted == slot.0.to_be_bytes());
                assert!(*value == root);
            }
            _ => unreachable!("the delete precedes its replacement"),
        }
    }

    /// Finalization advancement alone selects a synced commit for every pair of checkpoints.
    #[kani::proof]
    fn finalization_controls_tick_durability() {
        let stored_slot: u64 = kani::any();
        let next_slot: u64 = kani::any();
        let stored = Checkpoint {
            root: [0; 32],
            slot: Slot(stored_slot),
        };
        let next = Checkpoint {
            root: [0; 32],
            slot: Slot(next_slot),
        };
        let expected = if next_slot > stored_slot {
            Durability::Synced
        } else {
            Durability::Buffered
        };
        assert!(super::tick_durability(Some(stored), next) == expected);
        assert!(super::tick_durability(None, next) == Durability::Synced);
    }
}
