//! Which validator is scheduled to propose in a slot.
//!
//! Proposer selection lives chain-side, next to the state transition and fork choice, rather
//! than in the validator crate: the state transition validates a block's proposer with the
//! same function a validator uses to learn it is on duty, and there is only one schedule.
//!
//! leanSpec defines this as a `ValidatorIndex` classmethod. Verity keeps it off the type for
//! the same reason as the justification predicates — see `justification`.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/containers/identifiers.py`, read
//! at commit `0588c2d215a955a516378677a92db2a5666802f3`.

use verity_types::ValidatorIndex;
use verity_types::primitives::Slot;

use crate::error::RejectionReason;

/// The validator scheduled to propose at `slot`, by round-robin over the registry.
///
/// # Errors
///
/// [`RejectionReason::EmptyValidatorRegistry`] when the registry is empty. Returning rather
/// than dividing keeps the modulo below total.
#[must_use = "this names the scheduled proposer; it does not check the block's claim"]
pub const fn proposer_for_slot(
    slot: Slot,
    validator_count: u64,
) -> Result<ValidatorIndex, RejectionReason> {
    if validator_count == 0 {
        return Err(RejectionReason::EmptyValidatorRegistry);
    }
    Ok(ValidatorIndex(slot.0 % validator_count))
}

#[cfg(test)]
mod tests {
    use super::{RejectionReason, Slot, ValidatorIndex, proposer_for_slot};

    #[test]
    fn should_cycle_through_the_registry_when_slots_advance() {
        let schedule: Vec<u64> = (0..7)
            .map(|slot| proposer_for_slot(Slot(slot), 3).unwrap().0)
            .collect();
        assert_eq!(schedule, vec![0, 1, 2, 0, 1, 2, 0]);
    }

    #[test]
    fn should_reject_when_the_registry_is_empty() {
        assert_eq!(
            proposer_for_slot(Slot(0), 0),
            Err(RejectionReason::EmptyValidatorRegistry)
        );
    }

    #[test]
    fn should_stay_within_the_registry_at_the_top_of_the_slot_range() {
        assert_eq!(
            proposer_for_slot(Slot(u64::MAX), 3),
            Ok(ValidatorIndex(u64::MAX % 3))
        );
    }
}
