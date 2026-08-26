//! Casper-FFG checkpoints and the attestation content they anchor.
//!
//! leanSpec puts `Checkpoint.advance_to` and `AttestationData.lies_on_chain` on these
//! containers. They are consensus decisions, not shape, so they live behind the capability
//! that owns them rather than here — see `docs/src/reference/architecture.md`, "Capability contracts".

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};

use crate::primitives::{Bytes32, Slot};

/// A (block root, slot) pair that can be justified and finalized.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct Checkpoint {
    /// The root hash of the checkpoint's block.
    pub root: Bytes32,
    /// The slot number of the checkpoint's block.
    pub slot: Slot,
}

/// Attestation content describing the validator's observed chain view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct AttestationData {
    /// The slot for which the attestation is made.
    pub slot: Slot,
    /// The checkpoint representing the head block as observed by the validator.
    pub head: Checkpoint,
    /// The checkpoint representing the target block as observed by the validator.
    pub target: Checkpoint,
    /// The checkpoint representing the source block as observed by the validator.
    pub source: Checkpoint,
}
