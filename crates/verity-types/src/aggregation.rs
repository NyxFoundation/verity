//! Post-quantum signature aggregation proofs, as they appear inside consensus containers.
//!
//! Only the wire shapes live here. Producing and verifying these proofs is a `verity-crypto`
//! capability over the upstream aggregation library.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::{SszBitlist, SszList};

use crate::config::{BYTE_LIST_512_KIB_LIMIT, VALIDATOR_REGISTRY_LIMIT};

/// Bitfield naming the validators that contributed to an aggregate.
pub type AggregationBits = SszBitlist<VALIDATOR_REGISTRY_LIMIT>;

/// A serialized aggregation proof, bounded at 512 KiB.
pub type ByteList512KiB = SszList<u8, BYTE_LIST_512_KIB_LIMIT>;

/// Single-message proof aggregating signatures from many validators.
///
/// Every named validator signed the same message for the same slot. The message and the slot
/// stay outside the proof: a verifier rederives them from the block body it already trusts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct SingleMessageAggregate {
    /// Bitfield indicating which validators contributed signatures.
    pub participants: AggregationBits,
    /// Aggregated proof bytes, in the compact public-key-free representation.
    pub proof: ByteList512KiB,
}

/// Merged proof covering many distinct messages.
///
/// Each component is a single-message proof over its own message. Merging binds the
/// components into the one proof a block carries whole.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct MultiMessageAggregate {
    /// Compact public-key-free serialized multi-message aggregate proof bytes.
    pub proof: ByteList512KiB,
}
