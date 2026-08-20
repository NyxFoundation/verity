//! The block container family: body, header, block, and the signed envelope.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};

use crate::aggregation::MultiMessageAggregate;
use crate::attestation::AggregatedAttestations;
use crate::primitives::{Bytes32, Slot, ValidatorIndex};

/// Payload of a block, carrying its attestations.
///
/// The attestation signatures are not here: they are folded into the block-level proof on
/// [`SignedBlock`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlockBody {
    /// Attestations included in the block.
    pub attestations: AggregatedAttestations,
}

/// Metadata summarizing a block without its body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlockHeader {
    /// The slot in which the block was proposed.
    pub slot: Slot,
    /// The index of the validator that proposed the block.
    pub proposer_index: ValidatorIndex,
    /// The root of the parent block.
    pub parent_root: Bytes32,
    /// The root of the state after applying this block.
    pub state_root: Bytes32,
    /// The root of the block body.
    pub body_root: Bytes32,
}

/// A complete block: header fields inline, plus the body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct Block {
    /// The slot in which the block was proposed.
    pub slot: Slot,
    /// The index of the validator that proposed the block.
    pub proposer_index: ValidatorIndex,
    /// The root of the parent block.
    pub parent_root: Bytes32,
    /// The root of the state after applying this block.
    pub state_root: Bytes32,
    /// The block's payload.
    pub body: BlockBody,
}

/// Envelope carrying a block with a single aggregated proof for all of its signatures.
///
/// The one proof binds every attestation in the body and the proposer signature over the
/// block root.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct SignedBlock {
    /// The block being signed.
    pub block: Block,
    /// Single full-block proof covering attestations and the proposer signature.
    pub proof: MultiMessageAggregate,
}
