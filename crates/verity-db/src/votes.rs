//! The reduction that turns a stream of verified aggregates into two small vote maps.
//!
//! Raw XMSS signatures and reusable proof pools are bounded in-memory inputs and are
//! discarded on restart. What survives is one latest vote per validator, in
//! `pending_votes` and `known_votes`.
//!
//! Reducing a set to one element needs a total order, or the surviving element depends on
//! insertion order and a restart reconstructs a different fork choice than the one that was
//! running. The order here is: newer attestation slot wins; on a tie, the lexicographically
//! larger `AttestationData` root wins. The tiebreak is arbitrary but total, which is the
//! whole requirement.
//!
//! Transcribed from `docs/design/storage.md`, "Fork-choice votes and time".

use core::cmp::Ordering;

use verity_types::AttestationData;

use crate::merkle::hash_tree_root;

/// Whether `candidate` replaces what a vote map holds for its validator.
///
/// `stored` is an `Option` because the rule covers the empty row too, and both callers need
/// that case: a validator votes for the first time at interval 1, and a pending vote is
/// merged into an empty `known_votes` row at interval 4. Folding it in here is what keeps one
/// rule from being written twice, once per caller, in two shapes that can drift apart.
///
/// A vote does not replace itself. Re-observing the same vote must not rewrite the row, so a
/// duplicate aggregate produces an empty batch rather than a redundant write.
#[must_use]
pub fn replaces(candidate: &AttestationData, stored: Option<&AttestationData>) -> bool {
    let Some(stored) = stored else {
        return true;
    };
    match candidate.slot.0.cmp(&stored.slot.0) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => hash_tree_root(candidate) > hash_tree_root(stored),
    }
}

#[cfg(test)]
mod tests {
    use verity_types::primitives::Slot;
    use verity_types::{AttestationData, Checkpoint};

    use super::replaces;

    fn vote(slot: u64, head: u8) -> AttestationData {
        AttestationData {
            slot: Slot(slot),
            head: Checkpoint {
                root: [head; 32],
                slot: Slot(slot),
            },
            ..AttestationData::default()
        }
    }

    #[test]
    fn should_prefer_the_newer_attestation_slot() {
        assert!(replaces(&vote(5, 1), Some(&vote(4, 9))));
        assert!(!replaces(&vote(4, 9), Some(&vote(5, 1))));
    }

    #[test]
    fn should_not_replace_a_vote_with_itself() {
        assert!(!replaces(&vote(5, 1), Some(&vote(5, 1))));
    }

    #[test]
    fn should_fill_an_empty_row_with_any_vote() {
        assert!(replaces(&vote(0, 0), None));
    }

    #[test]
    fn should_break_a_slot_tie_the_same_way_whichever_arrives_first() {
        let (a, b) = (vote(5, 1), vote(5, 2));
        assert_ne!(
            replaces(&a, Some(&b)),
            replaces(&b, Some(&a)),
            "exactly one direction wins, so insertion order cannot change the survivor"
        );
    }
}
