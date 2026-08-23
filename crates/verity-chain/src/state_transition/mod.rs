//! The state transition: how a block moves the consensus state forward.
//!
//! Signatures are verified before any of this runs. Nothing here reads a key, a clock, a
//! socket, or a database — the transition is a pure function from a pre-state and a block to
//! a post-state, which is what keeps it a candidate to move into the Verified Core whole.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/state_transition.py`, read at
//! commit `0588c2d215a955a516378677a92db2a5666802f3`.

pub mod attestations;
pub mod genesis;
pub mod header;
pub mod slots;

pub use attestations::process_attestations;
pub use genesis::generate_genesis;
pub use header::process_block_header;
pub use slots::process_slots;

use verity_types::{Block, State};

use crate::error::RejectionReason;
use crate::merkle::hash_tree_root;

/// Applies `block` to `state`, advancing through any empty slots first.
///
/// # Errors
///
/// Any [`RejectionReason`] the stages below produce, plus
/// [`RejectionReason::StateRootMismatch`] when the block does not commit to the post-state it
/// actually produces.
#[must_use = "this returns the post-state; the argument is not advanced in place"]
pub fn state_transition(state: &State, block: &Block) -> Result<State, RejectionReason> {
    let advanced = process_slots(state, block.slot)?;
    let post = process_block(&advanced, block)?;

    if block.state_root != hash_tree_root(&post) {
        return Err(RejectionReason::StateRootMismatch);
    }
    Ok(post)
}

/// Applies a block that already sits at the state's slot: header first, then the body.
///
/// # Errors
///
/// Any [`RejectionReason`] from header validation or attestation processing.
#[must_use = "this returns the state after the block; the argument is left untouched"]
pub fn process_block(state: &State, block: &Block) -> Result<State, RejectionReason> {
    let with_header = process_block_header(state, block)?;
    process_attestations(&with_header, &block.body.attestations)
}

#[cfg(test)]
pub(crate) mod testing {
    //! Builders shared by the unit tests of this module's submodules.

    use verity_types::primitives::ZERO_HASH;
    use verity_types::{Block, BlockBody, Slot, State, Validator, ValidatorIndex, Validators};

    use crate::merkle::hash_tree_root;
    use crate::proposer::proposer_for_slot;

    use super::generate_genesis;

    /// A genesis state holding `count` distinguishable validators.
    pub(crate) fn genesis_with(count: u64) -> State {
        let mut validators = Validators::default();
        for index in 0..count {
            let seed = u8::try_from(index % 256).expect("modulo keeps this in range");
            validators
                .push(Validator {
                    attestation_public_key: [seed; 52],
                    proposal_public_key: [seed.wrapping_add(128); 52],
                    index: ValidatorIndex(index),
                })
                .expect("count stays well under the registry limit");
        }
        generate_genesis(0, validators)
    }

    /// An empty block that `state` should accept, assuming it already sits at `slot`.
    pub(crate) fn empty_block_at(state: &State, slot: u64) -> Block {
        Block {
            slot: Slot(slot),
            proposer_index: proposer_for_slot(Slot(slot), state.validators.len() as u64)
                .expect("the registry is not empty"),
            parent_root: hash_tree_root(&state.latest_block_header),
            state_root: ZERO_HASH,
            body: BlockBody::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use verity_types::Slot;

    use crate::error::RejectionReason;
    use crate::merkle::hash_tree_root;

    use super::testing::{empty_block_at, genesis_with};
    use super::{process_slots, state_transition};

    #[test]
    fn should_reject_a_block_that_does_not_commit_to_the_state_it_produces() {
        let genesis = genesis_with(4);
        let advanced = process_slots(&genesis, Slot(1)).unwrap();
        let mut block = empty_block_at(&advanced, 1);
        block.state_root = [5u8; 32];

        assert_eq!(
            state_transition(&genesis, &block),
            Err(RejectionReason::StateRootMismatch)
        );
    }

    #[test]
    fn should_accept_a_block_committing_to_its_own_post_state() {
        let genesis = genesis_with(4);
        let advanced = process_slots(&genesis, Slot(1)).unwrap();
        let mut block = empty_block_at(&advanced, 1);
        block.state_root = hash_tree_root(&super::process_block(&advanced, &block).unwrap());

        let post = state_transition(&genesis, &block).expect("a self-consistent block");
        assert_eq!(post.slot, Slot(1));
        assert_eq!(hash_tree_root(&post), block.state_root);
    }
}
