#![allow(dead_code)]

//! A real chain to store.
//!
//! States are produced by `verity-chain`'s state transition rather than assembled by hand.
//! A repository test that stores a hand-built state proves only that bytes round-trip; the
//! interesting failures — a derived header that does not root to the block root, a
//! reconstruction that does not reproduce `state_root` — only appear against states the
//! transition actually produced.

use verity_chain::{generate_genesis, hash_tree_root, process_block, process_slots};
use verity_db::{Identity, stored_header};
use verity_types::primitives::{Bytes32, Slot, ZERO_HASH};
use verity_types::{
    Block, BlockBody, MultiMessageAggregate, State, Validator, ValidatorIndex, Validators,
};

/// One processed block, with everything a commit needs.
#[derive(Debug, Clone)]
pub struct Link {
    /// The block's root, as its children name it.
    pub root: Bytes32,
    /// The slot of the block's parent.
    pub parent_slot: Slot,
    /// The block's body.
    pub body: BlockBody,
    /// The state the block produced.
    pub post: State,
}

impl Link {
    /// The slot this block sits at.
    pub fn slot(&self) -> Slot {
        self.post.slot
    }
}

/// A genesis state holding `count` distinguishable validators.
pub fn genesis(count: u64) -> State {
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

/// The identity of a chain anchored at `state`.
pub fn identity_of(state: &State) -> Identity {
    Identity {
        chain_fingerprint: hash_tree_root(state),
        fork_version: 1,
    }
}

/// The root the anchor block is stored under.
pub fn anchor_root(state: &State) -> Bytes32 {
    hash_tree_root(&stored_header(state))
}

/// Extends `parent` with an empty block at `slot`.
///
/// # Panics
///
/// When `slot` is not ahead of `parent`, which is a bug in the test rather than a rejection
/// worth returning.
pub fn extend(parent: &State, slot: u64) -> Link {
    let parent_slot = parent.slot;
    let advanced = process_slots(parent, Slot(slot)).expect("the slot is ahead of the parent");

    let proposer = verity_chain::proposer_for_slot(Slot(slot), advanced.validators.len() as u64)
        .expect("the registry is not empty");
    let block = Block {
        slot: Slot(slot),
        proposer_index: proposer,
        parent_root: hash_tree_root(&advanced.latest_block_header),
        state_root: ZERO_HASH,
        body: BlockBody::default(),
    };

    let post = process_block(&advanced, &block).expect("an empty block on a well-formed parent");
    Link {
        root: hash_tree_root(&stored_header(&post)),
        parent_slot,
        body: block.body,
        post,
    }
}

/// A straight chain of empty blocks at the given slots.
pub fn chain(validators: u64, slots: &[u64]) -> (State, Vec<Link>) {
    let genesis = genesis(validators);
    let mut links: Vec<Link> = Vec::new();
    for slot in slots {
        let parent = links.last().map_or(&genesis, |link| &link.post);
        links.push(extend(parent, *slot));
    }
    (genesis, links)
}

/// A proof stand-in. Nothing in `verity-db` verifies a proof; it stores the bytes it is given.
pub fn proof(marker: u8) -> MultiMessageAggregate {
    let mut proof = MultiMessageAggregate::default();
    for _ in 0..64 {
        proof.proof.push(marker).expect("64 bytes fit in 512 KiB");
    }
    proof
}
