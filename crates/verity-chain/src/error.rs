//! Why the spec rejects an input.
//!
//! This is the `ProcessingError` of `docs/src/reference/architecture.md`'s capability contracts — a plain enum,
//! no structured payload, because rejection reasons are a small closed set and nothing but
//! the discriminant has to survive a future trip across the C ABI. It is named after the
//! leanSpec enum it mirrors so the two stay greppable against each other.
//!
//! Only the reasons Verity can currently produce are defined. leanSpec's enum has 36; the
//! four still absent are raised from paths this workspace has not reached — proposer-index
//! range checking, block-proof verification, and wire decoding. An unmodelled reason is not
//! silently tolerated: [`RejectionReason::as_str`] is what the fixture suites compare
//! against, so a vector expecting a reason this enum lacks fails the run.
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
    /// The anchor block does not commit to the anchor state it was handed with.
    AnchorStateRootMismatch,
    /// The block's parent has no state in the store, so the transition has nothing to start from.
    UnknownParentBlock,
    /// The block's slot runs past the horizon the store's own clock admits.
    BlockTooFarInFuture,
    /// The block repeats one attestation data entry, which the wire format forbids.
    DuplicateAttestationData,
    /// The vote names a source block the store has never seen.
    UnknownSourceBlock,
    /// The vote names a target block the store has never seen.
    UnknownTargetBlock,
    /// The vote names a head block the store has never seen.
    UnknownHeadBlock,
    /// The vote's source sits later than its target, which history forbids.
    SourceAfterTarget,
    /// The vote's head sits earlier than its target, which history forbids.
    HeadOlderThanTarget,
    /// The vote's source checkpoint slot disagrees with the slot of the block it names.
    SourceSlotMismatch,
    /// The vote's target checkpoint slot disagrees with the slot of the block it names.
    TargetSlotMismatch,
    /// The vote's head checkpoint slot disagrees with the slot of the block it names.
    HeadSlotMismatch,
    /// The vote's source does not lie on the target's chain of ancestors.
    SourceNotAncestorOfTarget,
    /// The vote's target does not lie on the head's chain of ancestors.
    TargetNotAncestorOfHead,
    /// The vote's head does not descend from the finalized block, so it can carry no weight.
    HeadNotDescendantOfFinalized,
    /// The vote's slot has not started locally yet, beyond the clock-skew margin.
    AttestationTooFarInFuture,
    /// The vote claims a head from a slot the vote itself precedes.
    AttestationSlotBeforeHead,
    /// The vote names a validator the target block's post-state registry does not hold.
    ValidatorNotInState,
    /// Signature verification failed.
    InvalidSignature,
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
            Self::AnchorStateRootMismatch => "ANCHOR_STATE_ROOT_MISMATCH",
            Self::UnknownParentBlock => "UNKNOWN_PARENT_BLOCK",
            Self::BlockTooFarInFuture => "BLOCK_TOO_FAR_IN_FUTURE",
            Self::DuplicateAttestationData => "DUPLICATE_ATTESTATION_DATA",
            Self::UnknownSourceBlock => "UNKNOWN_SOURCE_BLOCK",
            Self::UnknownTargetBlock => "UNKNOWN_TARGET_BLOCK",
            Self::UnknownHeadBlock => "UNKNOWN_HEAD_BLOCK",
            Self::SourceAfterTarget => "SOURCE_AFTER_TARGET",
            Self::HeadOlderThanTarget => "HEAD_OLDER_THAN_TARGET",
            Self::SourceSlotMismatch => "SOURCE_SLOT_MISMATCH",
            Self::TargetSlotMismatch => "TARGET_SLOT_MISMATCH",
            Self::HeadSlotMismatch => "HEAD_SLOT_MISMATCH",
            Self::SourceNotAncestorOfTarget => "SOURCE_NOT_ANCESTOR_OF_TARGET",
            Self::TargetNotAncestorOfHead => "TARGET_NOT_ANCESTOR_OF_HEAD",
            Self::HeadNotDescendantOfFinalized => "HEAD_NOT_DESCENDANT_OF_FINALIZED",
            Self::AttestationTooFarInFuture => "ATTESTATION_TOO_FAR_IN_FUTURE",
            Self::AttestationSlotBeforeHead => "ATTESTATION_SLOT_BEFORE_HEAD",
            Self::ValidatorNotInState => "VALIDATOR_NOT_IN_STATE",
            Self::InvalidSignature => "INVALID_SIGNATURE",
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
        RejectionReason::AnchorStateRootMismatch,
        RejectionReason::UnknownParentBlock,
        RejectionReason::BlockTooFarInFuture,
        RejectionReason::DuplicateAttestationData,
        RejectionReason::UnknownSourceBlock,
        RejectionReason::UnknownTargetBlock,
        RejectionReason::UnknownHeadBlock,
        RejectionReason::SourceAfterTarget,
        RejectionReason::HeadOlderThanTarget,
        RejectionReason::SourceSlotMismatch,
        RejectionReason::TargetSlotMismatch,
        RejectionReason::HeadSlotMismatch,
        RejectionReason::SourceNotAncestorOfTarget,
        RejectionReason::TargetNotAncestorOfHead,
        RejectionReason::HeadNotDescendantOfFinalized,
        RejectionReason::AttestationTooFarInFuture,
        RejectionReason::AttestationSlotBeforeHead,
        RejectionReason::ValidatorNotInState,
        RejectionReason::InvalidSignature,
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
