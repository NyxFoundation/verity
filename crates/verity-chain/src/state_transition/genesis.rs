//! The genesis state a chain starts from.

use verity_types::primitives::{Slot, ValidatorIndex, ZERO_HASH};
use verity_types::{BlockBody, BlockHeader, Checkpoint, GenesisConfig, State, Validators};

use crate::merkle::hash_tree_root;

/// Builds the genesis state for a registry, anchored at `genesis_time` (Unix seconds).
///
/// History is empty and both checkpoints are the zero anchor. The genesis header's body root
/// is the root of an empty body, which is what makes the first real block's `parent_root`
/// computable before any block exists.
#[must_use = "this builds the genesis state; it neither stores nor starts a chain"]
pub fn generate_genesis(genesis_time: u64, validators: Validators) -> State {
    let genesis_header = BlockHeader {
        slot: Slot(0),
        proposer_index: ValidatorIndex(0),
        parent_root: ZERO_HASH,
        state_root: ZERO_HASH,
        body_root: hash_tree_root(&BlockBody::default()),
    };

    State {
        config: GenesisConfig { genesis_time },
        slot: Slot(0),
        latest_block_header: genesis_header,
        latest_justified: Checkpoint {
            root: ZERO_HASH,
            slot: Slot(0),
        },
        latest_finalized: Checkpoint {
            root: ZERO_HASH,
            slot: Slot(0),
        },
        historical_block_hashes: Default::default(),
        justified_slots: Default::default(),
        validators,
        justifications_roots: Default::default(),
        justifications_validators: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use verity_types::primitives::ZERO_HASH;

    use crate::merkle::hash_tree_root;
    use crate::state_transition::testing::genesis_with;

    use super::{BlockBody, Slot, generate_genesis};

    #[test]
    fn should_root_the_empty_body_into_the_genesis_header() {
        let state = genesis_with(4);
        assert_eq!(
            state.latest_block_header.body_root,
            hash_tree_root(&BlockBody::default()),
            "the first block's parent root depends on this"
        );
    }

    #[test]
    fn should_anchor_both_checkpoints_at_the_zero_root_and_slot_zero() {
        let state = genesis_with(4);
        for checkpoint in [state.latest_justified, state.latest_finalized] {
            assert_eq!(checkpoint.root, ZERO_HASH);
            assert_eq!(checkpoint.slot, Slot(0));
        }
    }

    #[test]
    fn should_start_with_no_history_and_no_votes_in_flight() {
        let state = genesis_with(4);
        assert!(state.historical_block_hashes.is_empty());
        assert!(state.justified_slots.is_empty());
        assert!(state.justifications_roots.is_empty());
        assert!(state.justifications_validators.is_empty());
    }

    #[test]
    fn should_carry_the_genesis_time_it_was_given() {
        let state = generate_genesis(1_234_567_890, genesis_with(1).validators);
        assert_eq!(state.config.genesis_time, 1_234_567_890);
    }
}
