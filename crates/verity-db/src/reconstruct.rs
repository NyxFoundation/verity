//! Rebuilding a state from the snapshot it descends from and the diffs in between.
//!
//! The walk goes backwards to the nearest snapshot, then forwards applying diffs. It is
//! bounded at [`SNAPSHOT_INTERVAL_SLOTS`] steps by the snapshot rule of [`crate::diff`]: a
//! longer walk means the snapshot that should have terminated it is missing, which is
//! corruption rather than a long chain, and is reported as such instead of being followed to
//! genesis.
//!
//! Two things are checked on every step, because a diff on its own cannot be wrong in a way
//! that shows up later: the diff's base must be the child header's parent, and the state the
//! step produces must hash to the `state_root` the child header committed to.
//!
//! Transcribed from `docs/design/storage.md`, "State storage: snapshots and diffs".

use verity_types::primitives::{Bytes32, ZERO_HASH};
use verity_types::state::HistoricalBlockHashes;
use verity_types::{BlockHeader, State};

use crate::backend::StorageBackend;
use crate::column::ColumnFamily;
use crate::diff::{SNAPSHOT_INTERVAL_SLOTS, StateDiff};
use crate::error::StorageError;
use crate::key;
use crate::merkle::hash_tree_root;
use crate::repository::Repository;

impl<B: StorageBackend> Repository<B> {
    /// The state a block produced, rebuilt from stored data.
    ///
    /// Returns the snapshot directly when the block is a snapshot base; otherwise walks back
    /// to the nearest one and replays the diffs forward.
    ///
    /// # Errors
    ///
    /// - [`StorageError::MissingRow`] when a header or diff on the path is absent.
    /// - [`StorageError::ReplayTooLong`] when no snapshot is reached within the snapshot
    ///   interval.
    /// - [`StorageError::ParentMismatch`] when a diff claims a base its block does not have.
    /// - [`StorageError::RootMismatch`] when a rebuilt state does not hash to the root its
    ///   header committed to.
    pub fn state_at(&self, block_root: Bytes32) -> Result<State, StorageError> {
        let (base, path) = self.path_to_snapshot(block_root)?;

        let mut state = base;
        for (header, diff) in path.into_iter().rev() {
            state = apply(&state, &header, &diff)?;
            let root = hash_tree_root(&state);
            if root != header.state_root {
                return Err(StorageError::RootMismatch {
                    expected: header.state_root,
                    computed: root,
                });
            }
        }
        Ok(state)
    }

    /// Walks back from `block_root` to the nearest snapshot.
    ///
    /// The returned path is ordered child-first, so replaying it means iterating in reverse.
    fn path_to_snapshot(
        &self,
        block_root: Bytes32,
    ) -> Result<(State, Vec<(BlockHeader, StateDiff)>), StorageError> {
        let mut path = Vec::new();
        let mut cursor = block_root;

        loop {
            if let Some(snapshot) = self.state_snapshot(cursor)? {
                return Ok((snapshot, path));
            }
            if path.len() >= SNAPSHOT_INTERVAL_SLOTS as usize {
                return Err(StorageError::ReplayTooLong {
                    from: block_root,
                    walked: path.len(),
                });
            }
            let header: BlockHeader =
                self.read_required(ColumnFamily::BlockHeaders, &key::root(cursor))?;
            let diff: StateDiff =
                self.read_required(ColumnFamily::StateDiffs, &key::root(cursor))?;
            path.push((header, diff));
            cursor = header.parent_root;
        }
    }
}

/// Applies one block's diff on top of the state of its parent.
///
/// `config` and the validator registry come from `base` — they are static across the fork and
/// the snapshot is their only source. The latest header is the child's own, with its state
/// root cleared, exactly as `process_block_header` leaves it. The historical roots are
/// extended the way that function extends them: the parent's root, then one zero hash per
/// slot missed since the parent.
fn apply(base: &State, header: &BlockHeader, diff: &StateDiff) -> Result<State, StorageError> {
    if diff.base_block_root != header.parent_root {
        return Err(StorageError::ParentMismatch {
            expected: header.parent_root,
            found: diff.base_block_root,
        });
    }
    if diff.slot != header.slot {
        return Err(StorageError::RejectedBatch(
            "stored diff and header disagree on the slot",
        ));
    }

    Ok(State {
        config: base.config,
        slot: header.slot,
        latest_block_header: BlockHeader {
            state_root: ZERO_HASH,
            ..*header
        },
        latest_justified: diff.latest_justified,
        latest_finalized: diff.latest_finalized,
        historical_block_hashes: extend_history(base, header)?,
        justified_slots: diff.justified_slots.clone(),
        validators: base.validators.clone(),
        justifications_roots: diff.justifications_roots.clone(),
        justifications_validators: diff.justifications_validators.clone(),
    })
}

/// The chain view after `header`, given the view its parent left.
fn extend_history(
    base: &State,
    header: &BlockHeader,
) -> Result<HistoricalBlockHashes, StorageError> {
    let parent_slot = base.latest_block_header.slot;
    if header.slot.0 <= parent_slot.0 {
        return Err(StorageError::RejectedBatch(
            "stored block does not advance past its parent's slot",
        ));
    }

    let mut hashes = base.historical_block_hashes.clone();
    hashes
        .push(header.parent_root)
        .map_err(|_| StorageError::CapacityExceeded("historical_block_hashes"))?;
    for _ in 0..(header.slot.0 - parent_slot.0 - 1) {
        hashes
            .push(ZERO_HASH)
            .map_err(|_| StorageError::CapacityExceeded("historical_block_hashes"))?;
    }
    Ok(hashes)
}
