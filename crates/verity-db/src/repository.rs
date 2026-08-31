//! The repository: consensus values over a byte-level backend.
//!
//! Everything above [`crate::backend`] lives here. The backend knows keys and values; the
//! repository knows which table a container belongs in, which key encodes its identifier, and
//! which values must agree before a batch is allowed to become durable.
//!
//! Opening is where a wrong database is caught. `docs/design/storage.md` is explicit that a
//! populated database opens only when every identity value matches, and that a mismatch,
//! missing row, decode failure, or integrity failure stops the node and preserves the
//! directory: it is never treated as an empty database and never overwritten automatically.

use libssz::{SszDecode, SszEncode};
use verity_types::Checkpoint;
use verity_types::primitives::{Bytes32, Interval, Slot};

use crate::backend::{Durability, Rows, StorageBackend, WriteBatch};
use crate::column::ColumnFamily;
use crate::error::{IdentityMismatch, StorageError};
use crate::metadata::MetadataKey;
use crate::schema::{Identity, SCHEMA_VERSION, ssz_schema_digest};

/// A consensus repository over one backend.
///
/// Reads borrow shared and writes borrow unique, so the one-writer-per-database rule of
/// `docs/design/storage.md` is carried by ownership: P2P, RPC, and validator duties can hold
/// a `&Repository` and read, and only the chain writer can hold the `&mut` that commits.
#[derive(Debug)]
pub struct Repository<B: StorageBackend> {
    backend: B,
    identity: Identity,
}

impl<B: StorageBackend> Repository<B> {
    /// Opens `backend` as the repository of the chain described by `identity`.
    ///
    /// An empty database — one carrying none of the four identity values — opens and waits
    /// for an anchor commit to write them. A database carrying any of them must carry all
    /// four and must agree with `identity` and with this build's schema.
    ///
    /// # Errors
    ///
    /// - [`StorageError::Identity`] when a stored identity value disagrees.
    /// - [`StorageError::MissingMetadata`] when a populated database is missing one of the
    ///   four. This is corruption, not an empty database, and is never repaired here.
    /// - [`StorageError::Decode`] when an identity value does not decode.
    pub fn open(backend: B, identity: Identity) -> Result<Self, StorageError> {
        let repository = Self { backend, identity };
        if repository.is_populated()? {
            repository.check_identity()?;
        }
        Ok(repository)
    }

    /// The chain and fork this repository was opened for.
    #[must_use]
    pub const fn identity(&self) -> Identity {
        self.identity
    }

    /// The backend underneath, for tests and for operational inspection.
    #[must_use]
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// Whether the database has been anchored yet.
    ///
    /// # Errors
    ///
    /// [`StorageError::Backend`] when the engine fails.
    pub fn is_populated(&self) -> Result<bool, StorageError> {
        for key in MetadataKey::IDENTITY {
            if self.raw_metadata(key)?.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn check_identity(&self) -> Result<(), StorageError> {
        let stored_version: u32 = self.required_metadata(MetadataKey::SchemaVersion)?;
        if stored_version != SCHEMA_VERSION {
            return Err(IdentityMismatch::SchemaVersion {
                expected: SCHEMA_VERSION,
                found: stored_version,
            }
            .into());
        }

        let fingerprint: Bytes32 = self.required_metadata(MetadataKey::ChainFingerprint)?;
        if fingerprint != self.identity.chain_fingerprint {
            return Err(IdentityMismatch::ChainFingerprint {
                expected: self.identity.chain_fingerprint,
                found: fingerprint,
            }
            .into());
        }

        let fork: u64 = self.required_metadata(MetadataKey::ForkVersion)?;
        if fork != self.identity.fork_version {
            return Err(IdentityMismatch::ForkVersion {
                expected: self.identity.fork_version,
                found: fork,
            }
            .into());
        }

        let digest: Bytes32 = self.required_metadata(MetadataKey::SszSchemaDigest)?;
        let expected = ssz_schema_digest();
        if digest != expected {
            return Err(IdentityMismatch::SszSchemaDigest {
                expected,
                found: digest,
            }
            .into());
        }
        Ok(())
    }

    // --- Low-level typed access -----------------------------------------------------------

    /// Decodes the value at `key`, or `None` when the row is absent.
    ///
    /// An absent row is `None`; a row that fails to decode is an error. The two are never
    /// collapsed: a decode failure is storage corruption, and treating it as an empty slot
    /// would let a damaged database look like a shorter chain.
    pub(crate) fn read<T: SszDecode>(
        &self,
        table: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<T>, StorageError> {
        let Some(bytes) = self.backend.get(table, key)? else {
            return Ok(None);
        };
        T::from_ssz_bytes(&bytes)
            .map(Some)
            .map_err(|_| StorageError::Decode {
                table,
                key: key.to_vec(),
            })
    }

    /// Decodes a row the caller cannot proceed without.
    pub(crate) fn read_required<T: SszDecode>(
        &self,
        table: ColumnFamily,
        key: &[u8],
    ) -> Result<T, StorageError> {
        self.read(table, key)?
            .ok_or_else(|| StorageError::MissingRow {
                table,
                key: key.to_vec(),
            })
    }

    /// Reads a half-open key range, in ascending key order.
    pub(crate) fn scan(
        &self,
        table: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Rows, StorageError> {
        self.backend.range(table, start, end)
    }

    /// Applies a batch. The only path by which anything becomes durable.
    pub(crate) fn commit(
        &mut self,
        batch: WriteBatch,
        durability: Durability,
    ) -> Result<(), StorageError> {
        if batch.is_empty() {
            return Ok(());
        }
        self.backend.write(batch, durability)
    }

    // --- Metadata -------------------------------------------------------------------------

    fn raw_metadata(&self, key: MetadataKey) -> Result<Option<Vec<u8>>, StorageError> {
        self.backend.get(ColumnFamily::Metadata, key.as_bytes())
    }

    /// Decodes a metadata value, or `None` when it has never been written.
    ///
    /// # Errors
    ///
    /// [`StorageError::Decode`] when the value does not decode as `T`.
    pub fn metadata<T: SszDecode>(&self, key: MetadataKey) -> Result<Option<T>, StorageError> {
        self.read(ColumnFamily::Metadata, key.as_bytes())
    }

    /// Decodes a metadata value a populated database must carry.
    ///
    /// # Errors
    ///
    /// [`StorageError::MissingMetadata`] when absent, [`StorageError::Decode`] when it does
    /// not decode as `T`.
    pub fn required_metadata<T: SszDecode>(&self, key: MetadataKey) -> Result<T, StorageError> {
        self.metadata(key)?
            .ok_or(StorageError::MissingMetadata(key))
    }

    /// The current head block root.
    ///
    /// # Errors
    ///
    /// As [`Repository::metadata`].
    pub fn head(&self) -> Result<Option<Bytes32>, StorageError> {
        self.metadata(MetadataKey::Head)
    }

    /// The current safe target block root.
    ///
    /// # Errors
    ///
    /// As [`Repository::metadata`].
    pub fn safe_target(&self) -> Result<Option<Bytes32>, StorageError> {
        self.metadata(MetadataKey::SafeTarget)
    }

    /// The highest justified checkpoint committed.
    ///
    /// # Errors
    ///
    /// As [`Repository::metadata`].
    pub fn latest_justified(&self) -> Result<Option<Checkpoint>, StorageError> {
        self.metadata(MetadataKey::LatestJustified)
    }

    /// The highest finalized checkpoint committed.
    ///
    /// # Errors
    ///
    /// As [`Repository::metadata`].
    pub fn latest_finalized(&self) -> Result<Option<Checkpoint>, StorageError> {
        self.metadata(MetadataKey::LatestFinalized)
    }

    /// The first slot this node can serve proof-bearing history from.
    ///
    /// A checkpoint-sync node starts above genesis and must not advertise range service below
    /// this; see [`crate::retention`].
    ///
    /// # Errors
    ///
    /// As [`Repository::metadata`].
    pub fn served_from_slot(&self) -> Result<Option<Slot>, StorageError> {
        Ok(self.metadata::<u64>(MetadataKey::ServedFromSlot)?.map(Slot))
    }

    /// The last interval whose events fully committed.
    ///
    /// Wall-clock time alone cannot say which interval finished before a crash, which is why
    /// this is persisted rather than recomputed from the clock at startup.
    ///
    /// # Errors
    ///
    /// As [`Repository::metadata`].
    pub fn last_processed_interval(&self) -> Result<Option<Interval>, StorageError> {
        Ok(self
            .metadata::<u64>(MetadataKey::LastProcessedInterval)?
            .map(Interval))
    }
}

/// Queues a metadata write into `batch`.
///
/// Free function rather than a method so a caller assembling one cross-table batch does not
/// need a second mutable borrow of the repository while holding the batch.
pub(crate) fn put_metadata<T: SszEncode>(batch: &mut WriteBatch, key: MetadataKey, value: &T) {
    batch.queue_put(ColumnFamily::Metadata, key.as_bytes(), value.to_ssz());
}
