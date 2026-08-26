//! Justification candidacy — which slots may be justified after a given finalized boundary.
//!
//! leanSpec defines these as methods on `Slot` and `Checkpoint`. Verity keeps them off the
//! container types on purpose: they are the leading candidates to move into the Verified
//! Core, and binding them to `verity-types` would make every crate that merely uses a slot
//! link the FFI boundary once that move happens. See `ARCHITECTURE.md`, "Capability
//! contracts".
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/slot.py`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`.

use verity_types::config::HISTORICAL_ROOTS_LIMIT;
use verity_types::{Checkpoint, JustifiedSlots, Slot};

use crate::error::RejectionReason;

/// Slots within this distance of the finalized boundary are always justification candidates.
pub const IMMEDIATE_JUSTIFICATION_WINDOW: u64 = 5;

/// Position of `slot` in the justification bitfield anchored at `finalized`.
///
/// Returns `None` for a slot at or behind the boundary: those are justified by definition and
/// carry no tracked index. Slot `finalized + 1` maps to index 0.
#[must_use = "this only locates the bit; reading or setting it in the state is the caller's job"]
pub fn justified_index_after(slot: Slot, finalized: Slot) -> Option<usize> {
    if slot.0 <= finalized.0 {
        return None;
    }
    // The subtraction cannot underflow: the branch above established `slot > finalized`, so
    // the difference is at least 1 and the decrement stays in range.
    Some((slot.0 - finalized.0 - 1) as usize)
}

/// Whether `slot` is a valid justification candidate after `finalized`.
///
/// Per 3SF-mini, the distance from the finalized slot must be within the immediate window, a
/// perfect square, or a pronic number (`n(n+1)`: 6, 12, 20, …). A slot behind the boundary is
/// already settled and is never a future candidate.
#[must_use = "this answers the question; it neither records the verdict nor rejects the slot"]
pub fn is_justifiable_after(slot: Slot, finalized: Slot) -> bool {
    if slot.0 < finalized.0 {
        return false;
    }
    let delta = slot.0 - finalized.0;

    // Most candidates land here, so this runs before either square root.
    if delta <= IMMEDIATE_JUSTIFICATION_WINDOW {
        return true;
    }

    // Squares 1 and 4 already returned above; the first to reach here is 9.
    if is_perfect_square(u128::from(delta)) {
        return true;
    }

    // For a pronic delta = n(n+1), 4*delta + 1 = (2n+1)^2. Widened to u128 because 4*delta + 1
    // overflows u64 near the top of the slot range. The parity test mirrors leanSpec; an odd
    // square always has an odd root, so it never changes the answer on its own.
    let discriminant = 4 * u128::from(delta) + 1;
    is_perfect_square(discriminant) && discriminant.isqrt() % 2 == 1
}

/// The later of two checkpoints, keeping `current` on a slot tie.
///
/// Selection is by slot alone. That the candidate descends from `current` is a separate store
/// invariant and is not checked here.
#[must_use = "this returns the advanced checkpoint; neither argument is modified"]
pub fn advance_checkpoint(current: Checkpoint, candidate: Checkpoint) -> Checkpoint {
    if candidate.slot.0 > current.slot.0 {
        candidate
    } else {
        current
    }
}

/// Whether `slot` is already justified, per the bitfield anchored at `finalized`.
///
/// A slot at or behind the boundary is justified by definition and is not looked up.
///
/// # Errors
///
/// [`RejectionReason::JustifiedSlotOutOfRange`] when the slot is ahead of the boundary but
/// past the end of the tracked bitfield. leanSpec surfaces the same out-of-range access as a
/// domain rejection rather than letting an index error escape block processing.
#[must_use = "this answers whether the slot is justified; it does not justify it"]
pub fn is_slot_justified(
    justified_slots: &JustifiedSlots,
    finalized: Slot,
    slot: Slot,
) -> Result<bool, RejectionReason> {
    let Some(index) = justified_index_after(slot, finalized) else {
        return Ok(true);
    };
    justified_slots
        .get(index)
        .ok_or(RejectionReason::JustifiedSlotOutOfRange)
}

/// Grows the tracked bitfield until `slot` is addressable, filling new positions with `false`.
///
/// Returns the bitfield unchanged when `slot` is at or behind the boundary, or when the
/// bitfield already reaches it.
///
/// # Errors
///
/// [`RejectionReason::JustifiedSlotOutOfRange`] when addressing `slot` would need more bits
/// than [`HISTORICAL_ROOTS_LIMIT`] allows. That bound is the bitfield's SSZ limit, so a
/// larger one is not representable in the state at all.
#[must_use = "this returns the grown bitfield; the argument is left untouched"]
pub fn extend_justified_slots_to(
    justified_slots: &JustifiedSlots,
    finalized: Slot,
    slot: Slot,
) -> Result<JustifiedSlots, RejectionReason> {
    let Some(index) = justified_index_after(slot, finalized) else {
        return Ok(justified_slots.clone());
    };

    // Zero-based index, so covering it takes one more bit than its value.
    let required = index.saturating_add(1);
    if required <= justified_slots.len() {
        return Ok(justified_slots.clone());
    }
    if required > HISTORICAL_ROOTS_LIMIT {
        return Err(RejectionReason::JustifiedSlotOutOfRange);
    }

    let mut extended = justified_slots.clone();
    while extended.len() < required {
        extended
            .push(false)
            .map_err(|_| RejectionReason::JustifiedSlotOutOfRange)?;
    }
    Ok(extended)
}

fn is_perfect_square(value: u128) -> bool {
    let root = value.isqrt();
    root * root == value
}

#[cfg(test)]
mod tests {
    use super::{
        advance_checkpoint, extend_justified_slots_to, is_justifiable_after, is_slot_justified,
        justified_index_after,
    };
    use crate::error::RejectionReason;
    use verity_types::{Checkpoint, JustifiedSlots, Slot};

    #[test]
    fn should_report_no_index_when_slot_is_at_or_behind_the_finalized_boundary() {
        assert_eq!(justified_index_after(Slot(10), Slot(10)), None);
        assert_eq!(justified_index_after(Slot(9), Slot(10)), None);
    }

    #[test]
    fn should_map_the_first_slot_after_the_boundary_to_index_zero() {
        assert_eq!(justified_index_after(Slot(11), Slot(10)), Some(0));
        assert_eq!(justified_index_after(Slot(13), Slot(10)), Some(2));
    }

    #[test]
    fn should_reject_when_the_slot_is_behind_the_finalized_boundary() {
        assert!(!is_justifiable_after(Slot(9), Slot(10)));
    }

    #[test]
    fn should_accept_every_delta_inside_the_immediate_window() {
        assert!((0..=5).all(|delta| is_justifiable_after(Slot(100 + delta), Slot(100))));
    }

    #[test]
    fn should_reject_a_delta_that_is_neither_square_nor_pronic() {
        for delta in [7, 8, 10, 11, 13, 14, 15] {
            assert!(
                !is_justifiable_after(Slot(delta), Slot(0)),
                "delta {delta} should not be justifiable"
            );
        }
    }

    #[test]
    fn should_accept_a_square_delta_beyond_the_immediate_window() {
        for delta in [9, 16, 25, 36] {
            assert!(is_justifiable_after(Slot(delta), Slot(0)), "delta {delta}");
        }
    }

    #[test]
    fn should_accept_a_pronic_delta_beyond_the_immediate_window() {
        for delta in [6, 12, 20, 30, 42] {
            assert!(is_justifiable_after(Slot(delta), Slot(0)), "delta {delta}");
        }
    }

    #[test]
    fn should_not_overflow_when_the_delta_is_near_the_slot_ceiling() {
        // 4 * delta + 1 leaves u64 here; the answer matters less than the absence of a panic.
        assert!(!is_justifiable_after(Slot(u64::MAX), Slot(0)));
    }

    #[test]
    fn should_keep_the_current_checkpoint_when_the_candidate_ties_on_slot() {
        let current = Checkpoint {
            root: [1u8; 32],
            slot: Slot(7),
        };
        let candidate = Checkpoint {
            root: [2u8; 32],
            slot: Slot(7),
        };
        assert_eq!(advance_checkpoint(current, candidate), current);
    }

    #[test]
    fn should_take_the_candidate_when_it_is_strictly_later() {
        let current = Checkpoint {
            root: [1u8; 32],
            slot: Slot(7),
        };
        let candidate = Checkpoint {
            root: [2u8; 32],
            slot: Slot(8),
        };
        assert_eq!(advance_checkpoint(current, candidate), candidate);
    }

    fn bitfield(bits: &[bool]) -> JustifiedSlots {
        JustifiedSlots::try_from(bits.to_vec()).expect("fits well under the tracked limit")
    }

    #[test]
    fn should_report_justified_when_the_slot_is_at_or_behind_the_finalized_boundary() {
        let empty = bitfield(&[]);
        assert_eq!(is_slot_justified(&empty, Slot(10), Slot(10)), Ok(true));
        assert_eq!(is_slot_justified(&empty, Slot(10), Slot(3)), Ok(true));
    }

    #[test]
    fn should_read_the_tracked_bit_when_the_slot_is_ahead_of_the_boundary() {
        let tracked = bitfield(&[false, true, false]);
        assert_eq!(is_slot_justified(&tracked, Slot(0), Slot(1)), Ok(false));
        assert_eq!(is_slot_justified(&tracked, Slot(0), Slot(2)), Ok(true));
    }

    #[test]
    fn should_reject_when_the_queried_slot_is_past_the_tracked_range() {
        assert_eq!(
            is_slot_justified(&bitfield(&[false]), Slot(0), Slot(9)),
            Err(RejectionReason::JustifiedSlotOutOfRange)
        );
    }

    #[test]
    fn should_grow_with_unset_flags_when_the_bitfield_falls_short_of_the_slot() {
        let grown = extend_justified_slots_to(&bitfield(&[true]), Slot(0), Slot(4)).unwrap();
        assert_eq!(grown.len(), 4);
        assert_eq!(grown.get(0), Some(true));
        assert_eq!(grown.count_ones(), 1);
    }

    #[test]
    fn should_leave_the_bitfield_alone_when_it_already_reaches_the_slot() {
        let tracked = bitfield(&[true, true, true]);
        let unchanged = extend_justified_slots_to(&tracked, Slot(0), Slot(2)).unwrap();
        assert_eq!(unchanged.len(), 3, "reaching the slot must not shrink it");
    }

    #[test]
    fn should_leave_the_bitfield_alone_when_the_slot_is_behind_the_boundary() {
        let tracked = bitfield(&[true]);
        let unchanged = extend_justified_slots_to(&tracked, Slot(7), Slot(7)).unwrap();
        assert_eq!(unchanged.len(), 1);
    }

    #[test]
    fn should_reject_when_covering_the_slot_would_overrun_the_tracked_limit() {
        assert_eq!(
            extend_justified_slots_to(&bitfield(&[]), Slot(0), Slot(u64::MAX)),
            Err(RejectionReason::JustifiedSlotOutOfRange)
        );
    }
}
