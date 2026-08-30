//! The consensus repository: what Verity persists, and the only way it is written.
//!
//! # What belongs here
//!
//! A Runtime Shell concern, not a consensus object. Nothing in this crate decides whether a
//! block is valid, which fork is the head, or whether a vote should count — it records what
//! the chain already decided, and refuses to record something self-contradictory. That is why
//! it depends on `verity-types` and not on `verity-chain`: a repository able to reach a
//! consensus decision would be a second place where one could be made.
//!
//! Consensus values are stored as their exact SSZ types, so a row never invents an
//! alternative encoding for a protocol container. The one type defined here that leanSpec
//! does not have is [`StateDiff`], and it exists for a size reason set out in [`diff`].
//!
//! # The layers
//!
//! - [`backend`] — four byte-level operations, with a RocksDB implementation and an in-memory
//!   sibling that preserves its iteration order.
//! - [`column`] and [`key`] — which table a value lives in and how its identifier is encoded.
//!   Integer keys are big-endian so that lexicographic order is numeric order, which is what
//!   makes a slot range a scan and a proof expiry a single range tombstone.
//! - [`Repository`] — consensus values over the backend, plus the identity check that refuses
//!   a database belonging to another chain, fork, or schema.
//! - [`writer`] — the four commits, each one batch, each checked before it can become durable.
//! - [`retention`] — the only two things ever deleted, and what a peer may therefore request.
//!
//! # One writer
//!
//! `docs/design/storage.md` requires one writer per database, with no exceptions: P2P, RPC,
//! validator duties, and maintenance submit requests to the chain writer rather than writing
//! here. The rule is carried by ownership rather than by convention — [`Repository`] owns its
//! backend by value, reads take `&self`, and commits take `&mut self` — so a second writer
//! cannot be constructed without moving the repository out of the first.
//!
//! Within a process that is the whole enforcement. Across processes it is RocksDB's: opening
//! a directory takes an exclusive lock on it, so a second node aimed at the same path fails
//! to open rather than interleaving writes. Nothing here needs to detect that case, because
//! [`RocksBackend::open`] cannot succeed in it.
//!
//! # No signing state
//!
//! No table holds validator signing state. XMSS no-reuse is a property of the duty loop's
//! structure and nothing about it is written; see `docs/design/key-management.md`.

pub mod backend;
pub mod column;
pub mod diff;
pub mod error;
pub mod key;
mod merkle;
pub mod metadata;
pub mod read;
pub mod reconstruct;
pub mod repository;
pub mod retention;
pub mod schema;
pub mod votes;
pub mod writer;

pub use backend::{Durability, MemoryBackend, RocksBackend, StorageBackend, WriteBatch};
pub use column::ColumnFamily;
pub use diff::{SNAPSHOT_INTERVAL_SLOTS, StateDiff};
pub use error::{IdentityMismatch, StorageError};
pub use metadata::MetadataKey;
pub use read::ForkChoiceEntry;
pub use repository::Repository;
pub use retention::{MIN_SLOTS_FOR_BLOCK_REQUESTS, PROOF_RETENTION_SLOTS};
pub use schema::{Identity, SCHEMA_VERSION, ssz_schema_digest};
pub use writer::{AnchorCommit, BlockCommit, TickCommit, stored_header};
