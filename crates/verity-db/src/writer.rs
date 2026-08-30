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
use crate::votes::supersedes;

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

        batch.put(
            ColumnFamily::BlockHeaders,
            key::root(anchor.block_root),
            header.to_ssz(),
        );
        if let Some(body) = anchor.body {
            batch.put(
                ColumnFamily::BlockBodies,
                key::root(anchor.block_root),
                body.to_ssz(),
            );
        }
        batch.put(
            ColumnFamily::StateSnapshots,
            key::root(anchor.block_root),
            anchor.state.to_ssz(),
        );
        batch.put(
            ColumnFamily::StateRoots,
            key::root(state_root),
            anchor.block_root.to_vec(),
        );
        batch.put(
            ColumnFamily::CanonicalBlocks,
            key::slot(slot),
            anchor.block_root.to_vec(),
        );
        batch.put(
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
        batch.put(ColumnFamily::BlockHeaders, key::root(root), header.to_ssz());
        batch.put(
            ColumnFamily::BlockBodies,
            key::root(root),
            commit.body.to_ssz(),
        );
        batch.put(
            ColumnFamily::BlockProofs,
            key::slot_and_root(slot, root),
            commit.proof.to_ssz(),
        );
        batch.put(
            ColumnFamily::StateDiffs,
            key::root(root),
            StateDiff::of(commit.post_state).to_ssz(),
        );
        if crosses_snapshot_boundary(commit.parent_slot, slot) {
            batch.put(
                ColumnFamily::StateSnapshots,
                key::root(root),
                commit.post_state.to_ssz(),
            );
        }
        batch.put(
            ColumnFamily::StateRoots,
            key::root(header.state_root),
            root.to_vec(),
        );
        batch.put(
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
            if stored.is_some_and(|stored| !supersedes(vote, &stored)) {
                continue;
            }
            batch.put(
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

        let advances_finalization = self
            .latest_finalized()?
            .is_none_or(|stored| tick.latest_finalized.slot.0 > stored.slot.0);
        let durability = if advances_finalization {
            Durability::Synced
        } else {
            Durability::Buffered
        };
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
            Some(previous) => self.divergence(previous, new_head)?,
            // No previous head: nothing to unwind, and the anchor already wrote its own slot.
            None => CanonicalSwitch {
                leaving: Vec::new(),
                joining: self.ancestry_to_canonical(new_head)?,
            },
        };
        for slot in switch.leaving {
            batch.delete(ColumnFamily::CanonicalBlocks, key::slot(slot));
        }
        for (slot, root) in switch.joining {
            batch.put(
                ColumnFamily::CanonicalBlocks,
                key::slot(slot),
                root.to_vec(),
            );
        }
        Ok(())
    }

    /// Walks both heads back to their common ancestor.
    ///
    /// The walk is bounded by the chain itself: each step moves one of the two pointers
    /// strictly backwards, and a missing header stops it as corruption rather than looping.
    fn divergence(
        &self,
        previous: Bytes32,
        new_head: Bytes32,
    ) -> Result<CanonicalSwitch, StorageError> {
        let (mut old, mut new) = (previous, new_head);
        let mut switch = CanonicalSwitch::default();

        while old != new {
            let old_header = self.required_header(old)?;
            let new_header = self.required_header(new)?;
            if old_header.slot.0 >= new_header.slot.0 {
                switch.leaving.push(old_header.slot);
                old = old_header.parent_root;
            } else {
                switch.joining.push((new_header.slot, new));
                new = new_header.parent_root;
            }
        }
        Ok(switch)
    }

    /// Every `(slot, root)` from `head` back to the first block already recorded as canonical
    /// at its own slot.
    fn ancestry_to_canonical(&self, head: Bytes32) -> Result<Vec<(Slot, Bytes32)>, StorageError> {
        let mut joining = Vec::new();
        let mut cursor = head;
        loop {
            let header = self.required_header(cursor)?;
            if self.canonical_root(header.slot)? == Some(cursor) {
                return Ok(joining);
            }
            joining.push((header.slot, cursor));
            cursor = header.parent_root;
        }
    }

    fn required_header(&self, root: Bytes32) -> Result<BlockHeader, StorageError> {
        self.read_required(ColumnFamily::BlockHeaders, &key::root(root))
    }

    /// Queues the interval-4 merge: every pending vote is offered to the counted map under
    /// the same total order, and the pending map is emptied whether or not it won.
    fn queue_vote_merge(&self, batch: &mut WriteBatch) -> Result<(), StorageError> {
        for (validator, vote) in self.pending_votes()? {
            let stored = self.known_vote(validator)?;
            if stored.is_none_or(|stored| supersedes(&vote, &stored)) {
                batch.put(
                    ColumnFamily::KnownVotes,
                    key::validator(validator),
                    vote.to_ssz(),
                );
            }
            batch.delete(ColumnFamily::PendingVotes, key::validator(validator));
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
