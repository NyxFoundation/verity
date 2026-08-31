//! The byte-level contract every storage engine satisfies, and nothing above it.
//!
//! The trait exposes exactly four operations: point reads, lexicographically ordered
//! half-open range reads, atomic write batches spanning column families, and half-open range
//! deletes. Range deletes are an operation *inside* a batch rather than a method beside it,
//! which is what lets a proof expiry and the metadata that records it commit together.
//!
//! No RocksDB behavior may leak past this contract without being named in it. The in-memory
//! sibling preserves lexicographic iteration order for exactly that reason: a test that
//! passes against it is a test that describes the ordering RocksDB will give in production.
//!
//! # Why writes take `&mut self`
//!
//! `docs/design/storage.md` requires one writer per database, with no exceptions — P2P, RPC,
//! validator duties, and maintenance submit requests to the chain writer rather than writing
//! here. That rule is expressible in the type system, so it is enforced by ownership: reads
//! borrow shared, writes borrow unique, and a second writer cannot be constructed without
//! moving the backend out of the first.

pub mod memory;
pub mod rocks;

use crate::column::ColumnFamily;
use crate::error::StorageError;

pub use memory::MemoryBackend;
pub use rocks::RocksBackend;

/// Key-value pairs read from one table, in ascending key order.
///
/// Named rather than spelled out because the trait returns it and the repository forwards it
/// unchanged: two identical anonymous tuple-vector types in two signatures say less about
/// what they hold than one alias does.
pub type Rows = Vec<(Vec<u8>, Vec<u8>)>;

/// Whether a batch must reach the disk before the call returns.
///
/// Write-ahead logging is always on, so every batch is atomic either way. This chooses only
/// whether the log is fsynced: routine block imports and interval updates are not, while
/// anchors and commits that advance finalization are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Durability {
    /// Atomic, logged, but not fsynced. The default for routine progress.
    Buffered,
    /// Atomic and fsynced before returning.
    Synced,
}

/// One mutation inside a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Write a value, replacing any value already at the key.
    Put {
        /// Table to write into.
        table: ColumnFamily,
        /// Key to write at.
        key: Vec<u8>,
        /// Value to write.
        value: Vec<u8>,
    },
    /// Remove a single key.
    Delete {
        /// Table to delete from.
        table: ColumnFamily,
        /// Key to remove.
        key: Vec<u8>,
    },
    /// Remove every key in the half-open interval `[start, end)`.
    DeleteRange {
        /// Table to delete from.
        table: ColumnFamily,
        /// Inclusive lower bound.
        start: Vec<u8>,
        /// Exclusive upper bound.
        end: Vec<u8>,
    },
}

/// A set of mutations that apply all or none.
///
/// The `queue_` prefix on every method is the point: they mutate this batch and nothing else,
/// and no key moves until the batch is handed to [`StorageBackend::write`]. A commit is one
/// batch, so a crash between two of these calls cannot leave half a block on disk.
///
/// Ops are applied in insertion order, so a later put over an earlier key in the same batch
/// wins, and a range delete does not remove a put issued after it. A reorg to a block at a
/// slot the old branch also occupied relies on exactly that: the delete is queued first and
/// the new root's put lands after it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteBatch {
    ops: Vec<Op>,
}

impl WriteBatch {
    /// An empty batch.
    #[must_use]
    pub const fn new() -> Self {
        Self { ops: Vec::new() }
    }

    /// Writes `value` at `key`, replacing whatever the key already held.
    ///
    /// `key` is generic because callers build one from a root, a slot, a validator index, or
    /// a fixed ASCII name; `value` is not, because every value in this crate is already the
    /// `Vec<u8>` that `to_ssz` returned.
    pub fn queue_put(&mut self, table: ColumnFamily, key: impl Into<Vec<u8>>, value: Vec<u8>) {
        self.ops.push(Op::Put {
            table,
            key: key.into(),
            value,
        });
    }

    /// Removes `key`. A key that is not present is not an error.
    pub fn queue_delete(&mut self, table: ColumnFamily, key: impl Into<Vec<u8>>) {
        self.ops.push(Op::Delete {
            table,
            key: key.into(),
        });
    }

    /// Removes every key in `[start, end)`, leaving `end` itself in place.
    pub fn queue_delete_range(
        &mut self,
        table: ColumnFamily,
        start: impl Into<Vec<u8>>,
        end: impl Into<Vec<u8>>,
    ) {
        self.ops.push(Op::DeleteRange {
            table,
            start: start.into(),
            end: end.into(),
        });
    }

    /// The queued mutations, in the order they will be applied.
    #[must_use]
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Whether the batch would change anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// How many mutations are queued.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len()
    }
}

/// A key-value engine the repository can be built on.
///
/// Implementors own no consensus meaning: keys and values are opaque bytes, and the ordering
/// guarantee below is the only structure the repository is allowed to rely on.
pub trait StorageBackend {
    /// Reads the value at `key`, or `None` when the table holds no such key.
    ///
    /// # Errors
    ///
    /// [`StorageError::Backend`] when the engine fails.
    fn get(&self, table: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError>;

    /// Reads every pair in `[start, end)`, in ascending lexicographic key order.
    ///
    /// The result is materialized rather than streamed. Every caller in the repository scans
    /// a slot-bounded window — a range-sync response, a fork-choice rebuild from the justified
    /// slot, the two validator-indexed vote maps — so the bound is the caller's, not the
    /// engine's, and a borrowed iterator would only push RocksDB's lifetimes into the trait.
    ///
    /// # Errors
    ///
    /// [`StorageError::Backend`] when the engine fails.
    fn range(&self, table: ColumnFamily, start: &[u8], end: &[u8]) -> Result<Rows, StorageError>;

    /// Applies every op in `batch` atomically, or none of them.
    ///
    /// # Errors
    ///
    /// [`StorageError::Backend`] when the engine fails. The batch is not partially applied.
    fn write(&mut self, batch: WriteBatch, durability: Durability) -> Result<(), StorageError>;
}
