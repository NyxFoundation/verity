//! Why the spec rejects an input.
//!
//! This is the `ProcessingError` of `docs/src/reference/architecture.md`'s capability contracts — a plain enum,
//! no structured payload, because rejection reasons are a small closed set and nothing but
//! the discriminant has to survive a future trip across the C ABI. It is named after the
//! leanSpec enum it mirrors so the two stay greppable against each other.
//!
//! Only the reasons Verity can currently produce are defined. leanSpec's enum has 36; the
//! rest belong to fork choice and gossip validation, and land with them. An unmodelled
//! reason is not silently tolerated: [`RejectionReason::as_str`] is what the fixture suites
//! compare against, so a vector expecting a reason this enum lacks fails the run.
//!
//! One variant is here ahead of the code that leanSpec raises it from.
//! [`RejectionReason::BlockSlotGapTooLarge`] guards the transition's empty-slot walk, which
//! leanSpec guards from fork choice instead — see `state_transition::process_slots`.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/errors.py`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`.

use core::fmt;

/// Language-neutral reason the spec rejects an invalid input.
///
/// The variant names, and the strings [`RejectionReason::as_str`] returns, are leanSpec's
/// verbatim. They are the wire form: fixtures carry them as `rejectionReason`, and a future
/// FFI status code maps one-to-one onto them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectionReason {
    /// The block slot is not strictly greater than the current state slot.
    BlockSlotNotInFuture,
    /// The block slot runs so far beyond its parent it would force an unbounded empty-slot walk.
    BlockSlotGapTooLarge,
    /// The block slot disagrees with the state slot after slot processing.
    BlockSlotMismatch,
    /// The block slot is not newer than the latest block header.
    BlockOlderThanLatestHeader,
    /// The block parent root disagrees with the latest block header root.
    ParentRootMismatch,
    /// The block state root disagrees with the computed post-state root.
    StateRootMismatch,
    /// The block proposer is not the scheduled proposer for its slot.
    WrongProposer,
    /// The registry holds no validators, so no proposer can be scheduled for any slot.
    EmptyValidatorRegistry,
    /// A set aggregation bit points outside the validator registry.
    ValidatorIndexOutOfRange,
    /// The block carries more distinct attestation data entries than allowed.
    TooManyAttestationData,
    /// An aggregated attestation references no validator at all.
    EmptyAggregationBits,
    /// The flat vote list length is not the tracked-root count times the validator count.
    JustificationVotesLengthMismatch,
    /// A queried slot is active but outside the tracked justification range.
    JustifiedSlotOutOfRange,
    /// A tracked justification root is the zero hash, which marks a slot with no block.
    ZeroHashJustificationRoot,
}

impl RejectionReason {
    /// The leanSpec name for this reason, as it appears in a test vector.
    #[must_use = "this renders the reason; it does not raise or record it"]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BlockSlotNotInFuture => "BLOCK_SLOT_NOT_IN_FUTURE",
            Self::BlockSlotGapTooLarge => "BLOCK_SLOT_GAP_TOO_LARGE",
            Self::BlockSlotMismatch => "BLOCK_SLOT_MISMATCH",
            Self::BlockOlderThanLatestHeader => "BLOCK_OLDER_THAN_LATEST_HEADER",
            Self::ParentRootMismatch => "PARENT_ROOT_MISMATCH",
            Self::StateRootMismatch => "STATE_ROOT_MISMATCH",
            Self::WrongProposer => "WRONG_PROPOSER",
            Self::EmptyValidatorRegistry => "EMPTY_VALIDATOR_REGISTRY",
            Self::ValidatorIndexOutOfRange => "VALIDATOR_INDEX_OUT_OF_RANGE",
            Self::TooManyAttestationData => "TOO_MANY_ATTESTATION_DATA",
            Self::EmptyAggregationBits => "EMPTY_AGGREGATION_BITS",
            Self::JustificationVotesLengthMismatch => "JUSTIFICATION_VOTES_LENGTH_MISMATCH",
            Self::JustifiedSlotOutOfRange => "JUSTIFIED_SLOT_OUT_OF_RANGE",
            Self::ZeroHashJustificationRoot => "ZERO_HASH_JUSTIFICATION_ROOT",
        }
    }
}

impl fmt::Display for RejectionReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl core::error::Error for RejectionReason {}

#[cfg(test)]
mod tests {
    use super::RejectionReason;

    /// Every variant, so a new one cannot be added without giving it a wire name here.
    const ALL: &[RejectionReason] = &[
        RejectionReason::BlockSlotNotInFuture,
        RejectionReason::BlockSlotGapTooLarge,
        RejectionReason::BlockSlotMismatch,
        RejectionReason::BlockOlderThanLatestHeader,
        RejectionReason::ParentRootMismatch,
        RejectionReason::StateRootMismatch,
        RejectionReason::WrongProposer,
        RejectionReason::EmptyValidatorRegistry,
        RejectionReason::ValidatorIndexOutOfRange,
        RejectionReason::TooManyAttestationData,
        RejectionReason::EmptyAggregationBits,
        RejectionReason::JustificationVotesLengthMismatch,
        RejectionReason::JustifiedSlotOutOfRange,
        RejectionReason::ZeroHashJustificationRoot,
    ];

    #[test]
    fn should_render_a_distinct_screaming_snake_name_when_each_reason_is_displayed() {
        let mut names: Vec<&str> = ALL.iter().map(|reason| reason.as_str()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two reasons share a wire name");
        assert!(
            ALL.iter().all(|reason| {
                let name = reason.as_str();
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            }),
            "a wire name is not SCREAMING_SNAKE_CASE"
        );
    }
}
