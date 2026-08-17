//! Consensus container definitions and chain constants for Verity.
//!
//! # What belongs here
//!
//! Container **shapes** and their SSZ serialization, and nothing else. Field order is
//! consensus-critical — it determines `hash_tree_root`, hence the state and block roots — so
//! every container below transcribes leanSpec field for field, in leanSpec's order.
//!
//! Consensus *behavior* is deliberately absent. leanSpec defines predicates such as
//! `Slot.is_justifiable_after` and `Checkpoint.advance_to` as methods on these containers;
//! Verity places them behind the capability that owns them instead. The reason is migration
//! cost: those predicates are the leading candidates to move into the Verified Core, and
//! binding them here would make every crate that merely uses a type link the FFI boundary.
//! See `ARCHITECTURE.md`, "Capability contracts".
//!
//! # Source
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`. leanSpec is the only authority for these
//! shapes; where a Verity design document disagrees, leanSpec wins.

pub mod aggregation;
pub mod attestation;
pub mod block;
pub mod checkpoint;
pub mod config;
pub mod primitives;
pub mod state;
pub mod validator;

pub use aggregation::{
    AggregationBits, ByteList512KiB, MultiMessageAggregate, SingleMessageAggregate,
};
pub use attestation::{
    AggregatedAttestation, AggregatedAttestations, Attestation, SignedAggregatedAttestation,
};
pub use block::{Block, BlockBody, BlockHeader, SignedBlock};
pub use checkpoint::{AttestationData, Checkpoint};
pub use primitives::{Bytes32, Bytes52, Interval, Slot, SubnetId, ValidatorIndex, ZERO_HASH};
pub use state::{
    GenesisConfig, HistoricalBlockHashes, JustificationRoots, JustificationValidators,
    JustifiedSlots, State,
};
pub use validator::{Validator, Validators};
