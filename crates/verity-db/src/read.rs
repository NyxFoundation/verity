//! Reading consensus values back out of the repository.
//!
//! Every method here answers with `Option` where absence is legitimate — a slot with no
//! block, a validator that has not voted, a block whose proof has been pruned — and with an
//! error where absence is not. The distinction is the whole point: inside the advertised
//! range-sync window a missing proof for a canonical block is corruption, and a reader that
//! collapsed it to "empty slot" would serve a hole to a peer instead of failing.

use libssz::SszDecode;
use verity_types::primitives::{Bytes32, Slot, ValidatorIndex};
use verity_types::{AttestationData, BlockBody, BlockHeader, MultiMessageAggregate, State};

use crate::backend::StorageBackend;
use crate::column::ColumnFamily;
use crate::diff::StateDiff;
use crate::error::StorageError;
use crate::key;
use crate::repository::Repository;

/// One edge of the stored fork-choice tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ForkChoiceEntry {
    /// The slot the block was proposed in.
    pub slot: Slot,
    /// The block's own root.
    pub root: Bytes32,
    /// The root of the block's parent.
    pub parent_root: Bytes32,
}

impl<B: StorageBackend> Repository<B> {
    /// The header of a processed block.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored header does not decode.
    pub fn block_header(&self, root: Bytes32) -> Result<Option<BlockHeader>, StorageError> {
        self.read(ColumnFamily::BlockHeaders, &key::root(root))
    }

    /// The body of a processed block. An empty body is stored, so this is `None` only when
    /// the block itself is unknown.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored body does not decode.
    pub fn block_body(&self, root: Bytes32) -> Result<Option<BlockBody>, StorageError> {
        self.read(ColumnFamily::BlockBodies, &key::root(root))
    }

    /// The aggregate proof of a block, if it is still inside the retention window.
    ///
    /// The slot is required because proofs are keyed `slot ‖ root`: that ordering is what
    /// makes expiry a single range tombstone rather than a scan.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored proof does not decode.
    pub fn block_proof(
        &self,
        slot: Slot,
        root: Bytes32,
    ) -> Result<Option<MultiMessageAggregate>, StorageError> {
        self.read(ColumnFamily::BlockProofs, &key::slot_and_root(slot, root))
    }

    /// The full state snapshot anchored at a block, when that block is a snapshot base.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored state does not decode.
    pub fn state_snapshot(&self, root: Bytes32) -> Result<Option<State>, StorageError> {
        self.read(ColumnFamily::StateSnapshots, &key::root(root))
    }

    /// The state delta a processed non-anchor block wrote.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored diff does not decode.
    pub fn state_diff(&self, root: Bytes32) -> Result<Option<StateDiff>, StorageError> {
        self.read(ColumnFamily::StateDiffs, &key::root(root))
    }

    /// The canonical block root at a slot, or `None` for a slot with no canonical block.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored root does not decode.
    pub fn canonical_root(&self, slot: Slot) -> Result<Option<Bytes32>, StorageError> {
        self.read(ColumnFamily::CanonicalBlocks, &key::slot(slot))
    }

    /// The canonical roots in `[start, end)`, ascending by slot.
    ///
    /// Empty slots are absent rather than zero-filled, so the result is the response body a
    /// `BlocksByRange` reply is built from: partial responses are legal, and a gap here is a
    /// gap there.
    ///
    /// # Errors
    ///
    /// [`StorageError::KeyWidth`] or [`StorageError::Decode`] on a damaged row.
    pub fn canonical_range(
        &self,
        start: Slot,
        end: Slot,
    ) -> Result<Vec<(Slot, Bytes32)>, StorageError> {
        let (low, high) = key::slot_bounds(start, end);
        self.scan(ColumnFamily::CanonicalBlocks, &low, &high)?
            .into_iter()
            .map(|(raw_key, value)| {
                let slot = key::decode_slot(ColumnFamily::CanonicalBlocks, &raw_key)?;
                let root = decode_root_value(ColumnFamily::CanonicalBlocks, &raw_key, &value)?;
                Ok((slot, root))
            })
            .collect()
    }

    /// The block that produced a given state root.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored root does not decode.
    pub fn block_root_for_state_root(
        &self,
        state_root: Bytes32,
    ) -> Result<Option<Bytes32>, StorageError> {
        self.read(ColumnFamily::StateRoots, &key::root(state_root))
    }

    /// The parent of a block in the all-branch fork-choice index.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored root does not decode.
    pub fn fork_choice_parent(
        &self,
        slot: Slot,
        root: Bytes32,
    ) -> Result<Option<Bytes32>, StorageError> {
        self.read(
            ColumnFamily::ForkChoiceBlocks,
            &key::slot_and_root(slot, root),
        )
    }

    /// Every fork-choice edge in `[start, end)`, ascending by slot then root.
    ///
    /// This is what a restart rebuilds the fork-choice tree from: the scan begins at
    /// `latest_justified.slot`, because nothing below it can still be reorganized.
    ///
    /// # Errors
    ///
    /// [`StorageError::KeyWidth`] or [`StorageError::Decode`] on a damaged row.
    pub fn fork_choice_range(
        &self,
        start: Slot,
        end: Slot,
    ) -> Result<Vec<ForkChoiceEntry>, StorageError> {
        let (low, high) = key::slot_and_root_bounds(start, end);
        self.scan(ColumnFamily::ForkChoiceBlocks, &low, &high)?
            .into_iter()
            .map(|(raw_key, value)| {
                let (slot, root) =
                    key::decode_slot_and_root(ColumnFamily::ForkChoiceBlocks, &raw_key)?;
                let parent_root =
                    decode_root_value(ColumnFamily::ForkChoiceBlocks, &raw_key, &value)?;
                Ok(ForkChoiceEntry {
                    slot,
                    root,
                    parent_root,
                })
            })
            .collect()
    }

    /// A validator's latest counted vote.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored vote does not decode.
    pub fn known_vote(
        &self,
        validator: ValidatorIndex,
    ) -> Result<Option<AttestationData>, StorageError> {
        self.read(ColumnFamily::KnownVotes, &key::validator(validator))
    }

    /// A validator's latest not-yet-counted vote.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the stored vote does not decode.
    pub fn pending_vote(
        &self,
        validator: ValidatorIndex,
    ) -> Result<Option<AttestationData>, StorageError> {
        self.read(ColumnFamily::PendingVotes, &key::validator(validator))
    }

    /// Every counted vote, ascending by validator index.
    ///
    /// # Errors
    ///
    /// [`StorageError::KeyWidth`] or [`StorageError::Decode`] on a damaged row.
    pub fn known_votes(&self) -> Result<Vec<(ValidatorIndex, AttestationData)>, StorageError> {
        self.all_votes(ColumnFamily::KnownVotes)
    }

    /// Every not-yet-counted vote, ascending by validator index.
    ///
    /// # Errors
    ///
    /// [`StorageError::KeyWidth`] or [`StorageError::Decode`] on a damaged row.
    pub fn pending_votes(&self) -> Result<Vec<(ValidatorIndex, AttestationData)>, StorageError> {
        self.all_votes(ColumnFamily::PendingVotes)
    }

    pub(crate) fn all_votes(
        &self,
        table: ColumnFamily,
    ) -> Result<Vec<(ValidatorIndex, AttestationData)>, StorageError> {
        // The registry is bounded by `VALIDATOR_REGISTRY_LIMIT`, so scanning the whole
        // index-keyed table is bounded by the same constant, not by chain length.
        let (low, high) = key::slot_bounds(Slot(0), Slot(u64::MAX));
        let mut rows = self.scan(table, &low, &high)?;

        // `u64::MAX` is exclusive as a bound, so the last possible index is fetched on its
        // own rather than by widening the key space past what `key` can encode.
        let last = key::validator(ValidatorIndex(u64::MAX));
        if let Some(value) = self.backend().get(table, &last)? {
            rows.push((last.to_vec(), value));
        }

        rows.into_iter()
            .map(|(raw_key, value)| {
                let validator = key::decode_validator(table, &raw_key)?;
                let vote =
                    AttestationData::from_ssz_bytes(&value).map_err(|_| StorageError::Decode {
                        table,
                        key: raw_key.clone(),
                    })?;
                Ok((validator, vote))
            })
            .collect()
    }
}

/// Decodes a 32-byte root stored as a bare value.
fn decode_root_value(
    table: ColumnFamily,
    raw_key: &[u8],
    value: &[u8],
) -> Result<Bytes32, StorageError> {
    value.try_into().map_err(|_| StorageError::Decode {
        table,
        key: raw_key.to_vec(),
    })
}
