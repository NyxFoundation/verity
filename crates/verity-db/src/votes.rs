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

/// Whether `candidate` replaces `stored` in a vote map.
///
/// Equal votes do not supersede: re-observing the same vote must not rewrite the row, so a
/// duplicate aggregate produces an empty batch rather than a redundant write.
#[must_use]
pub fn supersedes(candidate: &AttestationData, stored: &AttestationData) -> bool {
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

    use super::supersedes;

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
        assert!(supersedes(&vote(5, 1), &vote(4, 9)));
        assert!(!supersedes(&vote(4, 9), &vote(5, 1)));
    }

    #[test]
    fn should_not_replace_a_vote_with_itself() {
        assert!(!supersedes(&vote(5, 1), &vote(5, 1)));
    }

    #[test]
    fn should_break_a_slot_tie_the_same_way_whichever_arrives_first() {
        let (a, b) = (vote(5, 1), vote(5, 2));
        assert_ne!(
            supersedes(&a, &b),
            supersedes(&b, &a),
            "exactly one direction wins, so insertion order cannot change the survivor"
        );
    }
}
