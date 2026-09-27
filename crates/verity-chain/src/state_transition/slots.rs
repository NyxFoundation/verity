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

#[cfg(kani)]
mod harnesses {
    use libssz_merkle::HashTreeRoot;
    use verity_types::{Bytes32, ZERO_HASH};

    use crate::state_transition::testing::genesis_with;

    use super::{HISTORICAL_ROOTS_LIMIT, RejectionReason, Slot, process_slots};

    /// Stands in for the SHA-256 hasher, which is outside both this crate and the checker's
    /// budget. The root's value plays no part in how the walk advances.
    #[allow(dead_code)]
    fn any_root<T: HashTreeRoot>(_: &T) -> Bytes32 {
        kani::any()
    }

    /// Walks of at most this many slots keep the proof tractable.
    const WALK: u64 = 3;

    /// The non-future guard decides exactly when a short walk runs, and a successful walk
    /// lands on the target.
    // Lean overlap: ST-1 (`State.process_slots_advances`). Future Lean-adoption deletion candidate.
    #[kani::proof]
    #[kani::unwind(34)]
    #[kani::stub(crate::merkle::hash_tree_root, any_root)]
    fn the_walk_is_guarded_and_lands_on_the_target() {
        let mut state = genesis_with(1);
        state.slot = Slot(kani::any());
        let target: u64 = kani::any();
        kani::assume(target <= state.slot.0.saturating_add(WALK));
        match process_slots(&state, Slot(target)) {
            Err(RejectionReason::BlockSlotNotInFuture) => {
                assert!(target <= state.slot.0);
            }
            Err(RejectionReason::BlockSlotGapTooLarge) => {
                unreachable!("a short walk cannot exceed the history limit");
            }
            Err(_) => {
                unreachable!("no other rejection is defined");
            }
            Ok(advanced) => {
                assert!(target > state.slot.0);
                assert!(advanced.slot.0 == target);
                assert!(advanced.latest_block_header.slot == state.latest_block_header.slot);
            }
        }
    }

    /// A walk beyond the protocol history limit is rejected before iteration begins.
    #[kani::proof]
    #[kani::unwind(34)]
    #[kani::stub(crate::merkle::hash_tree_root, any_root)]
    fn a_walk_beyond_the_history_limit_is_rejected() {
        let mut state = genesis_with(1);
        let gap = HISTORICAL_ROOTS_LIMIT as u64 + 1;
        let current: u64 = kani::any();
        kani::assume(current <= u64::MAX - gap);
        state.slot = Slot(current);
        assert!(matches!(
            process_slots(&state, Slot(current + gap)),
            Err(RejectionReason::BlockSlotGapTooLarge)
        ));
    }

    /// Empty-slot processing may fill an empty state root, but preserves every header
    /// identity field and never overwrites a root that was already filled.
    #[kani::proof]
    #[kani::unwind(34)]
    #[kani::stub(crate::merkle::hash_tree_root, any_root)]
    fn a_walk_only_fills_an_empty_header_state_root() {
        let mut state = genesis_with(1);
        let current: u64 = kani::any();
        kani::assume(current < u64::MAX);
        state.slot = Slot(current);
        state.latest_block_header.state_root = kani::any();
        let before = state.latest_block_header;
        let advanced = process_slots(&state, Slot(current + 1)).expect("one slot is in range");
        let after = advanced.latest_block_header;

        assert!(after.slot == before.slot);
        assert!(after.proposer_index == before.proposer_index);
        assert!(after.parent_root == before.parent_root);
        assert!(after.body_root == before.body_root);
        if before.state_root != ZERO_HASH {
            assert!(after == before);
        }
    }
}
