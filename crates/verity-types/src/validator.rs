//! The validator registry tracked in the consensus state.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

use crate::config::VALIDATOR_REGISTRY_LIMIT;
use crate::primitives::{Bytes52, ValidatorIndex};

/// A validator's static registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct Validator {
    /// XMSS public key for signing attestations.
    pub attestation_public_key: Bytes52,
    /// XMSS public key the proposer signs the block root with.
    pub proposal_public_key: Bytes52,
    /// Validator index in the registry.
    pub index: ValidatorIndex,
}

/// The validator registry tracked in the state.
///
/// leanSpec additionally rejects any registry whose stored `index` disagrees with the entry's
/// position. That is a validity rule over a well-formed shape, so it is checked where the
/// registry is admitted, not in the type.
pub type Validators = SszList<Validator, VALIDATOR_REGISTRY_LIMIT>;
