//! What can go wrong between a consensus value and the bytes that hold it.
//!
//! Every variant here is fatal to the node. `docs/design/storage.md` is explicit that a
//! decode failure, a missing required row, an identity mismatch, or a root-integrity failure
//! stops the node and preserves the directory for diagnosis — none of them is ever treated as
//! an absent value, and none is recovered from by overwriting. There is deliberately no
//! "not found" variant: an absent row that is legitimately absent is `Ok(None)`.

use core::fmt;

use verity_types::Bytes32;

use crate::column::ColumnFamily;
use crate::metadata::MetadataKey;

/// A failure of the repository or of the database underneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// The backend engine itself failed. Carries the engine's own message.
    Backend(String),
    /// A stored value did not decode as the SSZ type its table declares.
    Decode {
        /// The table the value came from.
        table: ColumnFamily,
        /// The key it was stored under.
        key: Vec<u8>,
    },
    /// A key of the wrong width was offered for a fixed-width table.
    KeyWidth {
        /// The table that rejected it.
        table: ColumnFamily,
        /// The width the table requires.
        expected: usize,
        /// The width supplied.
        found: usize,
    },
    /// A row the repository requires in order to make progress is absent.
    MissingRow {
        /// The table the row was expected in.
        table: ColumnFamily,
        /// The key it was expected under.
        key: Vec<u8>,
    },
    /// A `metadata` value a populated database must carry is absent.
    MissingMetadata(MetadataKey),
    /// The database on disk describes a different chain, fork, or schema.
    Identity(IdentityMismatch),
    /// A value does not hash to the root that was committed for it.
    RootMismatch {
        /// The root the committed data claims.
        expected: Bytes32,
        /// The root the stored value actually produces.
        computed: Bytes32,
    },
    /// A stored parent link disagrees with the child that points at it.
    ParentMismatch {
        /// The parent the child header names.
        expected: Bytes32,
        /// The parent the stored row names.
        found: Bytes32,
    },
    /// A batch was refused before it reached the backend.
    ///
    /// The writer checks a commit's internal consistency first, so an inconsistent batch is
    /// never made durable and never partially applied.
    RejectedBatch(&'static str),
    /// Rebuilding a state ran longer than the snapshot interval allows.
    ///
    /// The snapshot rule bounds any replay at 1,023 diffs. Exceeding it means the snapshot
    /// that should have terminated the walk is missing, which is corruption, not a long chain.
    ReplayTooLong {
        /// The block the walk started from.
        from: Bytes32,
        /// The number of diffs walked before giving up.
        walked: usize,
    },
    /// A reconstructed collection overran the SSZ limit that bounds it.
    CapacityExceeded(&'static str),
}

/// Which identity value disagrees with the node's configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityMismatch {
    /// The database was written by a different repository schema version.
    SchemaVersion {
        /// The version this build writes.
        expected: u32,
        /// The version found on disk.
        found: u32,
    },
    /// The database holds a different chain.
    ChainFingerprint {
        /// The configured genesis state root.
        expected: Bytes32,
        /// The genesis state root found on disk.
        found: Bytes32,
    },
    /// The database was written under a different protocol fork.
    ForkVersion {
        /// The configured fork version.
        expected: u64,
        /// The fork version found on disk.
        found: u64,
    },
    /// A stored SSZ container has changed shape since the database was written.
    SszSchemaDigest {
        /// The digest of this build's stored-type manifest.
        expected: Bytes32,
        /// The digest found on disk.
        found: Bytes32,
    },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(message) => write!(f, "storage backend failed: {message}"),
            Self::Decode { table, key } => {
                write!(f, "value in {table} at {} did not decode", Hex(key))
            }
            Self::KeyWidth {
                table,
                expected,
                found,
            } => write!(f, "{table} keys are {expected} bytes wide, got {found}"),
            Self::MissingRow { table, key } => {
                write!(f, "required row {} is absent from {table}", Hex(key))
            }
            Self::MissingMetadata(key) => write!(f, "required metadata value {key} is absent"),
            Self::Identity(mismatch) => write!(f, "{mismatch}"),
            Self::RootMismatch { expected, computed } => write!(
                f,
                "committed root {} but the value hashes to {}",
                Hex(expected),
                Hex(computed)
            ),
            Self::ParentMismatch { expected, found } => write!(
                f,
                "child names parent {} but the stored link names {}",
                Hex(expected),
                Hex(found)
            ),
            Self::RejectedBatch(reason) => write!(f, "batch refused: {reason}"),
            Self::ReplayTooLong { from, walked } => write!(
                f,
                "no snapshot found within {walked} diffs of {}",
                Hex(from)
            ),
            Self::CapacityExceeded(what) => write!(f, "{what} overran its SSZ limit"),
        }
    }
}

impl fmt::Display for IdentityMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaVersion { expected, found } => write!(
                f,
                "database schema version is {found}, this build writes {expected}"
            ),
            Self::ChainFingerprint { expected, found } => write!(
                f,
                "database holds chain {}, configured for {}",
                Hex(found),
                Hex(expected)
            ),
            Self::ForkVersion { expected, found } => write!(
                f,
                "database was written at fork version {found}, configured for {expected}"
            ),
            Self::SszSchemaDigest { expected, found } => write!(
                f,
                "database stored-type manifest is {}, this build's is {}",
                Hex(found),
                Hex(expected)
            ),
        }
    }
}

impl core::error::Error for StorageError {}
impl core::error::Error for IdentityMismatch {}

impl From<IdentityMismatch> for StorageError {
    fn from(mismatch: IdentityMismatch) -> Self {
        Self::Identity(mismatch)
    }
}

/// Renders bytes as lowercase hex, so a key or root in a message is greppable against the
/// database rather than being printed as a decimal array.
struct Hex<'a>(&'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("0x")?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ColumnFamily, Hex, IdentityMismatch, StorageError};

    #[test]
    fn should_render_keys_as_hex_so_a_message_is_greppable() {
        assert_eq!(Hex(&[0x0a, 0xff]).to_string(), "0x0aff");
    }

    #[test]
    fn should_name_the_table_and_key_when_a_value_fails_to_decode() {
        let error = StorageError::Decode {
            table: ColumnFamily::StateDiffs,
            key: vec![0x01],
        };
        assert_eq!(
            error.to_string(),
            "value in state_diffs at 0x01 did not decode"
        );
    }

    #[test]
    fn should_report_the_stored_side_and_the_configured_side_of_a_fork_mismatch() {
        let error = StorageError::from(IdentityMismatch::ForkVersion {
            expected: 2,
            found: 1,
        });
        assert_eq!(
            error.to_string(),
            "database was written at fork version 1, configured for 2"
        );
    }
}
