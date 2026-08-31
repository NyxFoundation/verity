//! What gets deleted, and what a peer may therefore ask for.
//!
//! Only two things are ever deleted: aggregate block proofs older than the retention window,
//! and votes current fork-choice rules have declared irrelevant. Headers, bodies, snapshots,
//! diffs, state-root mappings, and fork-choice entries are retained, so all processed state
//! history stays reconstructible.
//!
//! The proof window is what makes the deletion worth doing at all. Measured against
//! leanSpec's production-scheme fixtures, an aggregate block proof is 155–236 KB against
//! ~100–800 B for everything else — about 4.1 GB/day of proofs versus ~5 MB/day of blocks and
//! states at four-second slots.
//!
//! Transcribed from `docs/design/storage.md`, "Retention and range sync".

use verity_types::Checkpoint;
use verity_types::primitives::{Bytes32, Slot};

use crate::backend::{Durability, StorageBackend, WriteBatch};
use crate::column::ColumnFamily;
use crate::error::StorageError;
use crate::key;
use crate::repository::Repository;

/// How long aggregate block proofs are kept, in slots.
///
/// At `SECONDS_PER_SLOT = 4` this is about one day. It is an operational choice six times
/// above leanSpec's [`MIN_SLOTS_FOR_BLOCK_REQUESTS`] floor: it is how far a peer can fall
/// behind and still catch up over P2P instead of needing a checkpoint.
pub const PROOF_RETENTION_SLOTS: u64 = 21_600;

/// The sliding window a `BlocksByRange` responder must serve, in slots.
///
/// leanSpec's one MUST for the ReqResp suite: four hours at four-second slots. Below it, a
/// request is answered `RESOURCE_UNAVAILABLE` rather than with a short response.
pub const MIN_SLOTS_FOR_BLOCK_REQUESTS: u64 = 3_600;

impl<B: StorageBackend> Repository<B> {
    /// Deletes expired aggregate proofs, returning the exclusive cutoff when anything was
    /// deleted.
    ///
    /// Two conditions gate the delete, and both matter. The cutoff is
    /// `tip_slot − PROOF_RETENTION_SLOTS`, saturating at zero, so a young chain prunes
    /// nothing. And the delete happens only when the cutoff is at or below the finalized
    /// slot, which is what guarantees a proof inside the current non-finalized range is never
    /// removed — a reorg into that range would otherwise need a proof that had already gone.
    ///
    /// # Errors
    ///
    /// [`StorageError::Backend`] when the engine fails, or [`StorageError::Decode`] when the
    /// finalized checkpoint does not decode.
    pub fn prune_block_proofs(&mut self, tip_slot: Slot) -> Result<Option<Slot>, StorageError> {
        let cutoff = Slot(tip_slot.0.saturating_sub(PROOF_RETENTION_SLOTS));
        if cutoff.0 == 0 {
            return Ok(None);
        }
        let finalized = self.latest_finalized()?.unwrap_or_default();
        if cutoff.0 > finalized.slot.0 {
            return Ok(None);
        }

        let (low, high) = key::slot_and_root_bounds(Slot(0), cutoff);
        let mut batch = WriteBatch::new();
        batch.queue_delete_range(ColumnFamily::BlockProofs, low.to_vec(), high.to_vec());
        self.commit(batch, Durability::Buffered)?;
        Ok(Some(cutoff))
    }

    /// Deletes votes the fork choice can no longer be moved by, returning how many went.
    ///
    /// A vote is irrelevant when its head is at or below the finalized slot, or when its head
    /// is not a descendant of the finalized block. The blocks themselves stay: losing a vote's
    /// relevance says nothing about whether the block it names is still worth serving.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] on a damaged vote row, [`StorageError::Backend`] on engine
    /// failure.
    pub fn prune_stale_votes(&mut self) -> Result<usize, StorageError> {
        let Some(finalized) = self.latest_finalized()? else {
            return Ok(0);
        };

        let mut batch = WriteBatch::new();
        let mut pruned = 0;
        for table in [ColumnFamily::KnownVotes, ColumnFamily::PendingVotes] {
            let votes = self.all_votes(table)?;
            for (validator, vote) in votes {
                if self.vote_is_relevant(vote.head, finalized)? {
                    continue;
                }
                batch.queue_delete(table, key::validator(validator));
                pruned += 1;
            }
        }
        self.commit(batch, Durability::Buffered)?;
        Ok(pruned)
    }

    /// Whether a vote for `head` can still influence a fork choice finalized at `finalized`.
    fn vote_is_relevant(
        &self,
        head: Checkpoint,
        finalized: Checkpoint,
    ) -> Result<bool, StorageError> {
        if head.slot.0 <= finalized.slot.0 {
            return Ok(false);
        }
        self.descends_from(head.root, finalized)
    }

    /// Whether `root` is a descendant of `ancestor`, by walking the stored headers back.
    ///
    /// An unknown block answers `false`: descent that cannot be shown is not descent, and a
    /// vote for a block this node never processed is not one it can count.
    fn descends_from(&self, root: Bytes32, ancestor: Checkpoint) -> Result<bool, StorageError> {
        let mut cursor = root;
        loop {
            if cursor == ancestor.root {
                return Ok(true);
            }
            let Some(header) = self.block_header(cursor)? else {
                return Ok(false);
            };
            if header.slot.0 <= ancestor.slot.0 {
                return Ok(false);
            }
            cursor = header.parent_root;
        }
    }

    /// The lowest slot this node will answer a `BlocksByRange` request from.
    ///
    /// It is the higher of the spec's sliding four-hour floor and the first slot this node
    /// actually holds proved history for. A checkpoint-sync node starts above genesis, and
    /// advertising down to the spec floor before backfilling would promise history it does
    /// not have.
    ///
    /// # Errors
    ///
    /// [`StorageError::MissingMetadata`] when `served_from_slot` has never been written,
    /// which means the database was never anchored.
    pub fn range_service_floor(&self, current_slot: Slot) -> Result<Slot, StorageError> {
        let served_from = self
            .served_from_slot()?
            .ok_or(StorageError::MissingMetadata(
                crate::metadata::MetadataKey::ServedFromSlot,
            ))?;
        Ok(Slot(
            current_slot
                .0
                .saturating_sub(MIN_SLOTS_FOR_BLOCK_REQUESTS)
                .max(served_from.0),
        ))
    }

    /// Whether a `BlocksByRange` request starting at `requested_slot` may be served.
    ///
    /// A `false` here is a `RESOURCE_UNAVAILABLE` response, not an empty one.
    ///
    /// # Errors
    ///
    /// As [`Repository::range_service_floor`].
    pub fn can_serve_range(
        &self,
        current_slot: Slot,
        requested_slot: Slot,
    ) -> Result<bool, StorageError> {
        Ok(requested_slot.0 >= self.range_service_floor(current_slot)?.0)
    }
}
