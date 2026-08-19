//! Attestation vote envelopes: per-validator, aggregated, and their list form.
//!
//! leanSpec's `SignedAttestation` — an [`Attestation`] plus its raw XMSS signature — is not
//! defined here. Its signature field is an XMSS container supplied by the signature library,
//! and this crate takes no cryptographic dependency; it lands with `verity-crypto`.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

use crate::aggregation::{AggregationBits, SingleMessageAggregate};
use crate::checkpoint::AttestationData;
use crate::config::VALIDATOR_REGISTRY_LIMIT;
use crate::primitives::ValidatorIndex;

/// One validator's vote, wrapping the shared attestation data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct Attestation {
    /// The index of the validator making the attestation.
    pub validator_index: ValidatorIndex,
    /// The attestation data produced by the validator.
    pub data: AttestationData,
}

/// A vote shared by many validators, with a bitfield naming them.
///
/// This is the form stored in a block; the signatures are folded into the block-level proof.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct AggregatedAttestation {
    /// Bitfield indicating which validators participated in the aggregation.
    pub aggregation_bits: AggregationBits,
    /// Attestation data common to every validator in the aggregate.
    pub data: AttestationData,
}

/// The gossiped form of an aggregate: the shared data plus its aggregated proof.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct SignedAggregatedAttestation {
    /// Attestation data common to every validator in the aggregate.
    pub data: AttestationData,
    /// Aggregated single-message proof covering all participating validators.
    pub proof: SingleMessageAggregate,
}

/// The aggregated attestations carried in a block body.
pub type AggregatedAttestations = SszList<AggregatedAttestation, VALIDATOR_REGISTRY_LIMIT>;
