//! Header validation and the header-linked state it updates.

use verity_types::primitives::{Slot, ZERO_HASH};
use verity_types::{Block, BlockHeader, Checkpoint, State};

use crate::error::RejectionReason;
use crate::justification::extend_justified_slots_to;
use crate::merkle::hash_tree_root;
use crate::proposer::proposer_for_slot;

/// Validates `block`'s header against `state` and applies what the header alone determines.
///
/// The body is not read here beyond its root. Attestations are applied afterwards, by
/// [`crate::state_transition::process_attestations`].
///
/// # Errors
///
/// - [`RejectionReason::BlockSlotMismatch`] when the block does not sit at the state's slot.
/// - [`RejectionReason::BlockOlderThanLatestHeader`] when it does not advance past the tip.
/// - [`RejectionReason::EmptyValidatorRegistry`] when no proposer can be scheduled.
/// - [`RejectionReason::WrongProposer`] when the proposer is not the scheduled one.
/// - [`RejectionReason::ParentRootMismatch`] when it does not point at the known parent.
/// - [`RejectionReason::BlockSlotGapTooLarge`] when recording the skipped slots would overrun
///   the chain view's SSZ limit.
/// - [`RejectionReason::JustifiedSlotOutOfRange`] when the justification bitfield cannot grow
///   to cover the block's slot.
#[must_use = "this returns the state after the header; the argument is left untouched"]
pub fn process_block_header(state: &State, block: &Block) -> Result<State, RejectionReason> {
    let parent_header = state.latest_block_header;
    let parent_root = hash_tree_root(&parent_header);

    validate(state, block, parent_root)?;
    let (latest_justified, latest_finalized) = derive_header_checkpoints(state, parent_root);
    let historical_block_hashes = record_parent_and_skipped_slots(state, block, parent_root)?;

    // Flags are stored relative to the finalized boundary. The current slot is not
    // materialized until its header finishes, so tracking stops one short of it.
    let justified_slots = extend_justified_slots_to(
        &state.justified_slots,
        latest_finalized.slot,
        Slot(block.slot.0 - 1),
    )?;

    Ok(State {
        latest_justified,
        latest_finalized,
        historical_block_hashes,
        justified_slots,
        // The state root stays empty until the body is processed or the next slot begins.
        latest_block_header: BlockHeader {
            slot: block.slot,
            proposer_index: block.proposer_index,
            parent_root: block.parent_root,
            state_root: ZERO_HASH,
            body_root: hash_tree_root(&block.body),
        },
        ..state.clone()
    })
}

/// Every header check, in leanSpec's order.
fn validate(
    state: &State,
    block: &Block,
    parent_root: verity_types::Bytes32,
) -> Result<(), RejectionReason> {
    // The block must sit at the slot the state was advanced to.
    if block.slot != state.slot {
        return Err(RejectionReason::BlockSlotMismatch);
    }
    // It must be newer than the latest header.
    if block.slot.0 <= state.latest_block_header.slot.0 {
        return Err(RejectionReason::BlockOlderThanLatestHeader);
    }
    // It must come from the validator assigned to this slot.
    let validator_count = state.validators.len() as u64;
    if block.proposer_index != proposer_for_slot(state.slot, validator_count)? {
        return Err(RejectionReason::WrongProposer);
    }
    // It must point at the known parent.
    if block.parent_root != parent_root {
        return Err(RejectionReason::ParentRootMismatch);
    }
    Ok(())
}

/// Derives the justified and finalized checkpoints for the post-header state.
///
/// Genesis is the chain's anchor, justified and finalized by definition, so the first block
/// forces its parent to both. Every later block keeps what only attestations move.
fn derive_header_checkpoints(
    state: &State,
    parent_root: verity_types::Bytes32,
) -> (Checkpoint, Checkpoint) {
    if state.latest_block_header.slot.0 != 0 {
        return (state.latest_justified, state.latest_finalized);
    }
    let anchor = Checkpoint {
        root: parent_root,
        slot: Slot(0),
    };
    (anchor, anchor)
}

/// Appends the parent root, then one zero hash per slot missed since the parent.
fn record_parent_and_skipped_slots(
    state: &State,
    block: &Block,
    parent_root: verity_types::Bytes32,
) -> Result<verity_types::HistoricalBlockHashes, RejectionReason> {
    // Cannot underflow: the caller established `block.slot > parent_header.slot`.
    let empty_slots = block.slot.0 - state.latest_block_header.slot.0 - 1;

    let mut hashes = state.historical_block_hashes.clone();
    hashes
        .push(parent_root)
        .map_err(|_| RejectionReason::BlockSlotGapTooLarge)?;
    for _ in 0..empty_slots {
        hashes
            .push(ZERO_HASH)
            .map_err(|_| RejectionReason::BlockSlotGapTooLarge)?;
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use verity_types::primitives::ZERO_HASH;

    use crate::state_transition::process_slots;
    use crate::state_transition::testing::{empty_block_at, genesis_with};

    use super::{RejectionReason, Slot, process_block_header};

    /// Genesis advanced to slot 1, which is where a first block is applied from.
    fn at_slot_one() -> verity_types::State {
        process_slots(&genesis_with(4), Slot(1)).expect("slot 1 is ahead of genesis")
    }

    #[test]
    fn should_accept_the_first_block_after_genesis() {
        let state = at_slot_one();
        let block = empty_block_at(&state, 1);
        let post = process_block_header(&state, &block).expect("a well-formed first block");
        assert_eq!(post.latest_block_header.slot, Slot(1));
        assert_eq!(
            post.latest_block_header.state_root, ZERO_HASH,
            "the root is filled by the next slot, not by the header stage"
        );
    }

    #[test]
    fn should_reject_when_the_block_does_not_sit_at_the_state_slot() {
        let state = at_slot_one();
        let mut block = empty_block_at(&state, 1);
        block.slot = Slot(2);
        assert_eq!(
            process_block_header(&state, &block),
            Err(RejectionReason::BlockSlotMismatch)
        );
    }

    #[test]
    fn should_reject_when_the_proposer_is_not_the_scheduled_one() {
        let state = at_slot_one();
        let mut block = empty_block_at(&state, 1);
        block.proposer_index = verity_types::ValidatorIndex(block.proposer_index.0 + 1);
        assert_eq!(
            process_block_header(&state, &block),
            Err(RejectionReason::WrongProposer)
        );
    }

    #[test]
    fn should_reject_when_the_parent_root_does_not_match_the_tip() {
        let state = at_slot_one();
        let mut block = empty_block_at(&state, 1);
        block.parent_root = [9u8; 32];
        assert_eq!(
            process_block_header(&state, &block),
            Err(RejectionReason::ParentRootMismatch)
        );
    }

    #[test]
    fn should_anchor_both_checkpoints_on_the_parent_when_it_is_genesis() {
        let state = at_slot_one();
        let block = empty_block_at(&state, 1);
        let post = process_block_header(&state, &block).unwrap();
        assert_eq!(post.latest_justified.root, block.parent_root);
        assert_eq!(post.latest_finalized.root, block.parent_root);
        assert_eq!(post.latest_justified.slot, Slot(0));
    }

    #[test]
    fn should_record_the_parent_then_one_zero_hash_per_slot_missed() {
        let state = process_slots(&genesis_with(4), Slot(4)).unwrap();
        let block = empty_block_at(&state, 4);
        let post = process_block_header(&state, &block).unwrap();

        let recorded: Vec<_> = post.historical_block_hashes.iter().copied().collect();
        assert_eq!(
            recorded,
            vec![block.parent_root, ZERO_HASH, ZERO_HASH, ZERO_HASH],
            "slots 1 through 3 were missed"
        );
    }

    #[test]
    fn should_grow_the_justification_bitfield_to_one_short_of_the_block() {
        let state = process_slots(&genesis_with(4), Slot(4)).unwrap();
        let post = process_block_header(&state, &empty_block_at(&state, 4)).unwrap();
        assert_eq!(
            post.justified_slots.len(),
            3,
            "the block's own slot is not materialized until its header finishes"
        );
    }
}
