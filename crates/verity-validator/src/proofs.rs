//! Moving between the wire shape of an aggregate and the prover's own.
//!
//! A `SingleMessageAggregate` on the network is a participation bitfield and proof bytes: the
//! public keys are deliberately absent, and the verifier resolves the bitfield against the
//! registry it already trusts. So every conversion here needs a registry, and a caller that
//! cannot resolve a bitfield cannot decode the proof at all — which is the intended failure.

use verity_chain::fork_choice::participants;
use verity_crypto::aggregate::SingleMessageProof;
use verity_crypto::containers::PublicKey;
use verity_types::aggregation::ByteList512KiB;
use verity_types::{
    AggregationBits, MultiMessageAggregate, SingleMessageAggregate, ValidatorIndex, Validators,
};

use crate::error::DutyError;

/// The public keys a bitfield names, in bitfield order.
///
/// # Errors
///
/// [`DutyError::Aggregation`] when the bitfield names a validator outside the registry, or
/// when a stored key does not parse.
pub fn attestation_keys(
    validators: &Validators,
    bits: &AggregationBits,
) -> Result<Vec<PublicKey>, DutyError> {
    participants(bits)
        .map(|index| attestation_key(validators, index))
        .collect()
}

/// One validator's attestation key, resolved from the registry.
///
/// # Errors
///
/// [`DutyError::Aggregation`] when the index is outside the registry or the key does not
/// parse.
pub fn attestation_key(
    validators: &Validators,
    index: ValidatorIndex,
) -> Result<PublicKey, DutyError> {
    let validator = validators
        .get(index.0 as usize)
        .ok_or(DutyError::Aggregation(
            verity_crypto::AggregationError::MalformedPublicKey,
        ))?;
    PublicKey::from_bytes52(&validator.attestation_public_key)
        .map_err(|_| DutyError::Aggregation(verity_crypto::AggregationError::MalformedPublicKey))
}

/// One validator's proposal key, resolved from the registry.
///
/// The registry's key is used rather than the one this node signed with: it is what a
/// verifier resolves, so a disagreement between the two shows up here as a proof that cannot
/// be built rather than later as a block every peer rejects.
///
/// # Errors
///
/// [`DutyError::Aggregation`] when the index is outside the registry or the key does not
/// parse.
pub fn proposal_key(
    validators: &Validators,
    index: ValidatorIndex,
) -> Result<PublicKey, DutyError> {
    let validator = validators
        .get(index.0 as usize)
        .ok_or(DutyError::Aggregation(
            verity_crypto::AggregationError::MalformedPublicKey,
        ))?;
    PublicKey::from_bytes52(&validator.proposal_public_key)
        .map_err(|_| DutyError::Aggregation(verity_crypto::AggregationError::MalformedPublicKey))
}

/// Parses a wire aggregate back into a proof the prover can fold.
///
/// # Errors
///
/// [`DutyError::Aggregation`] when the bitfield cannot be resolved against the registry or
/// the bytes do not decompress into a proof of this shape.
pub fn decode(
    aggregate: &SingleMessageAggregate,
    validators: &Validators,
) -> Result<SingleMessageProof, DutyError> {
    let keys = attestation_keys(validators, &aggregate.participants)?;
    Ok(SingleMessageProof::from_proof_bytes(
        &aggregate.proof,
        &keys,
    )?)
}

/// Renders a single-message proof into the container a vote carries.
///
/// # Errors
///
/// [`DutyError::Aggregation`] when the proof is larger than the 512 KiB the container is
/// sized for, which means the aggregation topology outgrew the container rather than one
/// proof being unlucky.
pub fn to_single_container(
    proof: &SingleMessageProof,
    participants: AggregationBits,
) -> Result<SingleMessageAggregate, DutyError> {
    Ok(SingleMessageAggregate {
        participants,
        proof: to_bytes(proof.to_proof_bytes())?,
    })
}

/// Renders a merged proof into the container a block carries.
///
/// # Errors
///
/// [`DutyError::Aggregation`] when the proof exceeds the container's 512 KiB bound.
pub fn to_multi_container(
    proof: &verity_crypto::aggregate::MultiMessageProof,
) -> Result<MultiMessageAggregate, DutyError> {
    Ok(MultiMessageAggregate {
        proof: to_bytes(proof.to_proof_bytes())?,
    })
}

fn to_bytes(bytes: Vec<u8>) -> Result<ByteList512KiB, DutyError> {
    ByteList512KiB::try_from(bytes).map_err(|_| {
        DutyError::Aggregation(verity_crypto::AggregationError::InvalidRequest {
            reason: "the proof is larger than the 512 KiB a container carries".to_string(),
        })
    })
}
