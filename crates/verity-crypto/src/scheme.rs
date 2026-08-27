//! The one XMSS instantiation Verity signs and verifies under.
//!
//! # Where the parameters come from
//!
//! Not from here. Every constant below is re-derived from `leansig_wrapper`, which is the
//! crate leanVM's aggregator compiles its circuit against. Restating them locally would let
//! Verity verify a signature leanVM cannot aggregate — the two would diverge silently, and
//! the first symptom would be an aggregate that fails to prove on a live network.
//!
//! The values are nevertheless leanSpec's: `PROD_CONFIG` in
//! `src/lean_spec/spec/crypto/xmss/constants.py` fixes `LOG_LIFETIME = 32`,
//! `DIMENSION = 46`, `BASE = 8`, `TARGET_SUM = 200`, and the field-element lengths.
//! [`tests`] pins that agreement so an upstream parameter change fails here rather than on
//! a devnet.
//!
//! # Epoch is the slot
//!
//! leanSpec passes the consensus `Slot` to the XMSS scheme unmodified as its epoch index.
//! There is no offset and no mapping, so the only work [`epoch_for_slot`] does is the
//! `u64 -> u32` narrowing the scheme's own index type forces.

use core::fmt;

use verity_types::Slot;

use crate::error::SignatureError;

/// Bytes of message the scheme signs: a `hash_tree_root`, always.
pub use leansig_wrapper::MESSAGE_LENGTH;

/// Base-two logarithm of the scheme lifetime in epochs.
pub const LOG_LIFETIME: usize = leansig_wrapper::LOG_LIFETIME;

/// Number of Winternitz chains in a one-time signature, leanSpec's `DIMENSION`.
pub const DIMENSION: usize = leansig_wrapper::V;

/// Field elements in one Poseidon digest.
pub const HASH_LENGTH_FIELD_ELEMENTS: usize = leansig_wrapper::HASH_LEN_FE;

/// Field elements in a key's public parameter.
pub const PARAMETER_LENGTH: usize = leansig_wrapper::PARAMETER_LEN;

/// Field elements in a signature's encoding randomness.
pub const RAND_LENGTH_FIELD_ELEMENTS: usize = leansig_wrapper::RAND_LEN_FE;

/// Total epochs the scheme spans. `2^32` at four-second slots is roughly 544 years, which is
/// why the protocol has no key rotation: a key outlives any deployment that would need one.
pub const LIFETIME: u64 = 1 << LOG_LIFETIME;

/// Epochs one bottom tree covers, `sqrt(LIFETIME)`.
pub const EPOCHS_PER_BOTTOM_TREE: u64 = 1 << (LOG_LIFETIME / 2);

/// Width of the signable window: the two resident bottom trees.
///
/// About six days at four-second slots, which is the margin
/// [`crate::SecretKey::advance_preparation`] has to work inside.
pub const PREPARED_WINDOW_EPOCHS: u64 = 2 * EPOCHS_PER_BOTTOM_TREE;

/// SSZ-encoded length of a public key: an 8-element root plus a 5-element parameter.
pub const PUBLIC_KEY_BYTES: usize = (HASH_LENGTH_FIELD_ELEMENTS + PARAMETER_LENGTH) * 4;

/// SSZ-encoded length of a signature, which is the same for every valid signature.
///
/// Two of the three fields are lists at the type level, but a valid signature pins both to
/// scheme constants — one sibling per tree level, one released hash per chain — so the
/// encoded length never varies. The three offsets are the two top-level variable fields plus
/// the one nested inside the authentication path.
pub const SIGNATURE_BYTES: usize = LOG_LIFETIME * HASH_LENGTH_FIELD_ELEMENTS * 4
    + RAND_LENGTH_FIELD_ELEMENTS * 4
    + DIMENSION * HASH_LENGTH_FIELD_ELEMENTS * 4
    + 3 * 4;

/// Which of a validator's two keys an operation concerns.
///
/// The registry holds two independent XMSS keys per validator because a proposer signs both
/// a block and an attestation in its own slot, and one one-time key cannot cover two
/// messages at one epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// Signs attestations. leanSpec's `attestation_public_key`.
    Attestation,
    /// Signs block roots. leanSpec's `proposal_public_key`.
    Proposal,
}

impl Role {
    /// Both roles, in registry field order, for callers that must cover each one.
    pub const ALL: [Self; 2] = [Self::Attestation, Self::Proposal];

    /// The infix lean-quickstart's generator puts in this role's key file names.
    pub(crate) const fn file_infix(self) -> &'static str {
        match self {
            Self::Attestation => "attester",
            Self::Proposal => "proposer",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attestation => f.write_str("attestation"),
            Self::Proposal => f.write_str("proposal"),
        }
    }
}

/// Narrows a consensus slot to the scheme's epoch index.
///
/// # Errors
///
/// [`SignatureError::SlotOutsideLifetime`] when the slot names an epoch the scheme does not
/// have. A chain that ran long enough to reach it would have exhausted every validator key
/// centuries earlier, so this is a guard against a corrupt slot value, not a real horizon.
pub fn epoch_for_slot(slot: Slot) -> Result<u32, SignatureError> {
    u32::try_from(slot.0).map_err(|_| SignatureError::SlotOutsideLifetime { slot: slot.0 })
}

#[cfg(test)]
mod tests {
    use super::{
        DIMENSION, EPOCHS_PER_BOTTOM_TREE, HASH_LENGTH_FIELD_ELEMENTS, LIFETIME, LOG_LIFETIME,
        MESSAGE_LENGTH, PARAMETER_LENGTH, PUBLIC_KEY_BYTES, RAND_LENGTH_FIELD_ELEMENTS, Role,
        SIGNATURE_BYTES, epoch_for_slot,
    };
    use crate::error::SignatureError;
    use verity_types::Slot;

    /// leanSpec `PROD_CONFIG`, transcribed. If leanVM's instantiation ever drifts off it,
    /// this is where Verity finds out — at compile-and-test time, not on a devnet.
    #[test]
    fn should_match_leanspec_prod_config_when_scheme_parameters_are_read() {
        assert_eq!(LOG_LIFETIME, 32);
        assert_eq!(DIMENSION, 46);
        assert_eq!(PARAMETER_LENGTH, 5);
        assert_eq!(RAND_LENGTH_FIELD_ELEMENTS, 7);
        assert_eq!(HASH_LENGTH_FIELD_ELEMENTS, 8);
        assert_eq!(MESSAGE_LENGTH, 32);
    }

    /// The two sizes other crates lay out buffers against.
    #[test]
    fn should_match_the_measured_wire_sizes_when_lengths_are_derived() {
        assert_eq!(PUBLIC_KEY_BYTES, 52);
        assert_eq!(SIGNATURE_BYTES, 2536);
    }

    #[test]
    fn should_span_the_full_lifetime_when_bottom_trees_are_counted() {
        assert_eq!(EPOCHS_PER_BOTTOM_TREE * EPOCHS_PER_BOTTOM_TREE, LIFETIME);
    }

    #[test]
    fn should_pass_the_slot_through_unchanged_when_it_fits_the_epoch_index() {
        assert_eq!(epoch_for_slot(Slot(0)), Ok(0));
        assert_eq!(epoch_for_slot(Slot(u32::MAX as u64)), Ok(u32::MAX));
    }

    #[test]
    fn should_refuse_when_the_slot_names_an_epoch_the_scheme_does_not_have() {
        let beyond = u32::MAX as u64 + 1;
        assert_eq!(
            epoch_for_slot(Slot(beyond)),
            Err(SignatureError::SlotOutsideLifetime { slot: beyond })
        );
    }

    #[test]
    fn should_name_the_generator_file_infix_when_a_role_is_asked_for_it() {
        assert_eq!(Role::Attestation.file_infix(), "attester");
        assert_eq!(Role::Proposal.file_infix(), "proposer");
    }
}
