//! The logical tables the repository is split into.
//!
//! One column family per logical table, so a range tombstone issued for one keyspace cannot
//! reach another. RocksDB's own default column family is reserved and never named here.
//!
//! Transcribed from `docs/design/storage.md`, "Column families".

use core::fmt;

/// A logical table in the repository.
///
/// The `name` of a variant is the on-disk column-family name. It is part of the stored
/// format: renaming one makes an existing database unreadable, so a rename is a schema
/// migration, not a refactor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColumnFamily {
    /// `block_root` -> `SSZ(BlockHeader)`. Every processed block; retained.
    BlockHeaders,
    /// `block_root` -> `SSZ(BlockBody)`. Every processed block, empty bodies included.
    BlockBodies,
    /// `slot_be ‖ block_root` -> `SSZ(MultiMessageAggregate)`. Range-pruned.
    BlockProofs,
    /// `block_root` -> `SSZ(State)`. Anchors and periodic bases; retained.
    StateSnapshots,
    /// `block_root` -> `SSZ(StateDiff)`. One per processed non-anchor block; retained.
    StateDiffs,
    /// `slot_be` -> `block_root`. The canonical root at each non-empty slot.
    CanonicalBlocks,
    /// `state_root` -> `block_root`. Reverse lookup for checkpoint sync.
    StateRoots,
    /// `slot_be ‖ block_root` -> `parent_root`. The all-branch fork-choice tree.
    ForkChoiceBlocks,
    /// `validator_index_be` -> `SSZ(AttestationData)`. Latest counted vote.
    KnownVotes,
    /// `validator_index_be` -> `SSZ(AttestationData)`. Latest not-yet-counted vote.
    PendingVotes,
    /// Fixed ASCII key -> typed scalar or SSZ value. Identity and view pointers.
    Metadata,
}

impl ColumnFamily {
    /// Every column family, in the order `docs/design/storage.md` lists them.
    ///
    /// Opening a database creates exactly these, so a variant added without appearing here
    /// would be a table nothing ever creates.
    pub const ALL: [Self; 11] = [
        Self::BlockHeaders,
        Self::BlockBodies,
        Self::BlockProofs,
        Self::StateSnapshots,
        Self::StateDiffs,
        Self::CanonicalBlocks,
        Self::StateRoots,
        Self::ForkChoiceBlocks,
        Self::KnownVotes,
        Self::PendingVotes,
        Self::Metadata,
    ];

    /// The on-disk column-family name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BlockHeaders => "block_headers",
            Self::BlockBodies => "block_bodies",
            Self::BlockProofs => "block_proofs",
            Self::StateSnapshots => "state_snapshots",
            Self::StateDiffs => "state_diffs",
            Self::CanonicalBlocks => "canonical_blocks",
            Self::StateRoots => "state_roots",
            Self::ForkChoiceBlocks => "fork_choice_blocks",
            Self::KnownVotes => "known_votes",
            Self::PendingVotes => "pending_votes",
            Self::Metadata => "metadata",
        }
    }

    /// Whether values in this table are worth compressing.
    ///
    /// Aggregate proofs are high-entropy cryptographic blobs: compressing them spends CPU on
    /// every write and compaction to save almost nothing. Everything else is small, repetitive
    /// SSZ and takes LZ4 well.
    #[must_use]
    pub const fn compressed(self) -> bool {
        match self {
            Self::BlockProofs => false,
            Self::BlockHeaders
            | Self::BlockBodies
            | Self::StateSnapshots
            | Self::StateDiffs
            | Self::CanonicalBlocks
            | Self::StateRoots
            | Self::ForkChoiceBlocks
            | Self::KnownVotes
            | Self::PendingVotes
            | Self::Metadata => true,
        }
    }

    /// The fixed key width of this table, or `None` where keys are variable-width.
    ///
    /// Only [`ColumnFamily::Metadata`] is variable: its keys are fixed ASCII strings of
    /// differing length. Everywhere else a key of the wrong width is a caller bug, and
    /// [`crate::key`] rejects it rather than silently producing a row nothing can find.
    #[must_use]
    pub const fn key_width(self) -> Option<usize> {
        match self {
            Self::BlockHeaders
            | Self::BlockBodies
            | Self::StateSnapshots
            | Self::StateDiffs
            | Self::StateRoots => Some(32),
            Self::BlockProofs | Self::ForkChoiceBlocks => Some(40),
            Self::CanonicalBlocks | Self::KnownVotes | Self::PendingVotes => Some(8),
            Self::Metadata => None,
        }
    }
}

impl fmt::Display for ColumnFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::ColumnFamily;

    #[test]
    fn should_give_every_column_family_a_distinct_name() {
        let mut names: Vec<_> = ColumnFamily::ALL.iter().map(|cf| cf.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn should_never_use_the_reserved_default_column_family() {
        assert!(ColumnFamily::ALL.iter().all(|cf| cf.name() != "default"));
    }

    #[test]
    fn should_leave_only_the_proof_table_uncompressed() {
        let uncompressed: Vec<_> = ColumnFamily::ALL
            .iter()
            .filter(|cf| !cf.compressed())
            .collect();
        assert_eq!(uncompressed, vec![&ColumnFamily::BlockProofs]);
    }
}
