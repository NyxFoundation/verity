//! Extraction slice of `verity-chain::justification::is_justifiable_after`.
//!
//! The function body is copied from `crates/verity-chain/src/justification.rs` without
//! rewriting. `Slot` is the same `u64` newtype minus the SSZ impls, which this function
//! never uses. This crate exists only on verification branches and is not merged to
//! `develop`.

/// A slot index. Matches `verity_types::Slot` in representation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Slot(pub u64);

/// Slots within this distance of the finalized boundary are always justification candidates.
pub const IMMEDIATE_JUSTIFICATION_WINDOW: u64 = 5;

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

fn is_perfect_square(value: u128) -> bool {
    let root = value.isqrt();
    root * root == value
}

#[cfg(test)]
mod tests {
    use super::{Slot, is_justifiable_after};

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
        assert!(!is_justifiable_after(Slot(u64::MAX), Slot(0)));
    }
}
