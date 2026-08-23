//! Advancing the state through slots that carry no block.

use verity_types::State;
use verity_types::config::HISTORICAL_ROOTS_LIMIT;
use verity_types::primitives::{Slot, ZERO_HASH};

use crate::error::RejectionReason;
use crate::merkle::hash_tree_root;

/// Advances `state` through empty slots up to, but not including, `target_slot`.
///
/// The pre-block state root is cached into the latest header at most once per block: the
/// header's root is empty only on the first empty slot after a block, and later slots reuse
/// what that one filled in.
///
/// # Errors
///
/// - [`RejectionReason::BlockSlotNotInFuture`] when `target_slot` is not ahead of the state.
/// - [`RejectionReason::BlockSlotGapTooLarge`] when the walk would run longer than
///   [`HISTORICAL_ROOTS_LIMIT`] slots. leanSpec places this guard in fork choice, immediately
///   before it calls the transition; Verity places it at the transition's own entry so the
///   loop is bounded by the function's own signature rather than by its caller. The threshold
///   is leanSpec's, unchanged.
#[must_use = "this returns the advanced state; the argument is left at its original slot"]
pub fn process_slots(state: &State, target_slot: Slot) -> Result<State, RejectionReason> {
    if state.slot.0 >= target_slot.0 {
        return Err(RejectionReason::BlockSlotNotInFuture);
    }
    // The subtraction cannot underflow: the branch above established `target_slot > slot`.
    if target_slot.0 - state.slot.0 > HISTORICAL_ROOTS_LIMIT as u64 {
        return Err(RejectionReason::BlockSlotGapTooLarge);
    }

    let mut advanced = state.clone();
    while advanced.slot.0 < target_slot.0 {
        if advanced.latest_block_header.state_root == ZERO_HASH {
            // Rooted before the slot moves, so the cached root is the pre-advance state.
            advanced.latest_block_header.state_root = hash_tree_root(&advanced);
        }
        advanced.slot = Slot(advanced.slot.0 + 1);
    }
    Ok(advanced)
}

#[cfg(test)]
mod tests {
    use crate::merkle::hash_tree_root;
    use crate::state_transition::testing::genesis_with;

    use super::{HISTORICAL_ROOTS_LIMIT, RejectionReason, Slot, ZERO_HASH, process_slots};

    #[test]
    fn should_reject_when_the_target_slot_is_not_ahead_of_the_state() {
        let state = genesis_with(4);
        assert_eq!(
            process_slots(&state, Slot(0)),
            Err(RejectionReason::BlockSlotNotInFuture)
        );
    }

    #[test]
    fn should_stop_at_the_target_slot() {
        let advanced = process_slots(&genesis_with(4), Slot(3)).unwrap();
        assert_eq!(advanced.slot, Slot(3));
    }

    #[test]
    fn should_cache_the_pre_block_state_root_once_and_then_leave_it_alone() {
        let genesis = genesis_with(4);
        let expected = hash_tree_root(&genesis);

        let one = process_slots(&genesis, Slot(1)).unwrap();
        assert_eq!(
            one.latest_block_header.state_root, expected,
            "the first empty slot fills the header's empty root"
        );

        let many = process_slots(&genesis, Slot(9)).unwrap();
        assert_eq!(
            many.latest_block_header.state_root, expected,
            "later empty slots must reuse it, not re-root the advanced state"
        );
    }

    #[test]
    fn should_leave_the_header_root_untouched_when_it_is_already_filled() {
        let mut state = genesis_with(4);
        state.latest_block_header.state_root = [7u8; 32];
        let advanced = process_slots(&state, Slot(4)).unwrap();
        assert_eq!(advanced.latest_block_header.state_root, [7u8; 32]);
        assert_ne!(advanced.latest_block_header.state_root, ZERO_HASH);
    }

    #[test]
    fn should_reject_a_walk_longer_than_the_tracked_history_rather_than_spin() {
        let state = genesis_with(4);
        assert_eq!(
            process_slots(&state, Slot(HISTORICAL_ROOTS_LIMIT as u64 + 1)),
            Err(RejectionReason::BlockSlotGapTooLarge)
        );
    }
}
