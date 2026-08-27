//! The fork-choice store: the node's local view of the chain and the votes over it.
//!
//! leanSpec models the store as an immutable value and returns a fresh copy from every
//! operation. Verity does not. The store owns two maps that grow with the chain — every
//! block and every post-state above finalization — and a [`State`] is the largest value in
//! the system. Copying both per imported block would cost `O(chain)` per block for no gain:
//! `docs/src/reference/architecture.md` gives this aggregate a single writer, so no second
//! holder exists to observe an older copy.
//!
//! What the copy bought is kept as an invariant instead: **every fallible check runs before
//! any field is touched**, so an operation that returns `Err` leaves the store byte-for-byte
//! as it was. Each `&mut` entry point states this in its own contract and is tested for it.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/containers/store.py` and
//! `fork_choice.py`, read at commit `0588c2d215a955a516378677a92db2a5666802f3`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use verity_types::{
    AttestationData, Block, Bytes32, Checkpoint, GenesisConfig, Interval, SingleMessageAggregate,
    State, ValidatorIndex,
};

use crate::error::RejectionReason;
use crate::merkle::hash_tree_root;
use crate::slot_clock::intervals_at_slot_start;

/// One validator's raw signature over a vote, carried without being interpreted.
///
/// The bytes are an XMSS signature container produced by the signature library. This crate
/// takes no cryptographic dependency (see the crate docs), and never needs one for these:
/// the store only holds a signature until an aggregator folds it into a proof, and fork
/// choice weighs votes by *who* signed, never by the signature itself. Verifying the bytes
/// belongs to the caller, above this crate — see [`super::attestation`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttestationSignature(pub Vec<u8>);

/// A signature in the aggregator's pool, paired with the validator that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttestationSignatureEntry {
    /// The validator that signed.
    pub validator_index: ValidatorIndex,
    /// Its uninterpreted signature over the vote this entry is filed under.
    pub signature: AttestationSignature,
}

/// The node's fork-choice view: known blocks and states, the vote pools, and the checkpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    /// Intervals elapsed since genesis, as the node's clock has ticked them.
    pub time: Interval,
    /// Chain configuration, carried from the anchor state.
    pub config: GenesisConfig,
    /// The block fork choice currently selects.
    pub head: Bytes32,
    /// The deepest block a supermajority of this slot's voters already back.
    pub safe_target: Bytes32,
    /// The highest justified checkpoint the store has observed.
    pub latest_justified: Checkpoint,
    /// The highest finalized checkpoint the store has observed.
    pub latest_finalized: Checkpoint,
    /// Every known block, keyed by its root.
    pub blocks: HashMap<Bytes32, Block>,
    /// The post-state of every known block, keyed by the same root.
    pub states: HashMap<Bytes32, State>,
    /// The validator this node attests for, when it runs one.
    pub validator_index: Option<ValidatorIndex>,
    /// Per-validator signatures an aggregator has collected, grouped by the vote they sign.
    pub attestation_signatures: HashMap<AttestationData, HashSet<AttestationSignatureEntry>>,
    /// Proofs gathered this slot. They carry no weight until an acceptance tick promotes them.
    pub latest_new_aggregated_payloads: HashMap<AttestationData, HashSet<SingleMessageAggregate>>,
    /// Proofs that count toward fork-choice weight.
    pub latest_known_aggregated_payloads: HashMap<AttestationData, HashSet<SingleMessageAggregate>>,
}

impl Store {
    /// Builds a store anchored on a trusted block and its post-state.
    ///
    /// The anchor is treated as both justified and finalized: it is the deepest point the
    /// node will ever reconsider. Time starts at the anchor slot's first interval, so a node
    /// resuming from a checkpoint does not replay the intervals before it.
    ///
    /// # Errors
    ///
    /// [`RejectionReason::AnchorStateRootMismatch`] when the block does not commit to the
    /// state it was handed with. The pair would otherwise seed a store whose first state
    /// transition compares against a root nothing produced.
    #[must_use = "this builds the store; it neither registers nor starts anything"]
    pub fn new(
        state: &State,
        anchor_block: &Block,
        validator_index: Option<ValidatorIndex>,
    ) -> Result<Self, RejectionReason> {
        if anchor_block.state_root != hash_tree_root(state) {
            return Err(RejectionReason::AnchorStateRootMismatch);
        }

        let anchor_root = hash_tree_root(anchor_block);
        let anchor_checkpoint = Checkpoint {
            root: anchor_root,
            slot: anchor_block.slot,
        };

        Ok(Self {
            time: intervals_at_slot_start(anchor_block.slot),
            config: state.config,
            head: anchor_root,
            safe_target: anchor_root,
            latest_justified: anchor_checkpoint,
            latest_finalized: anchor_checkpoint,
            blocks: HashMap::from([(anchor_root, anchor_block.clone())]),
            states: HashMap::from([(anchor_root, state.clone())]),
            validator_index,
            attestation_signatures: HashMap::new(),
            latest_new_aggregated_payloads: HashMap::new(),
            latest_known_aggregated_payloads: HashMap::new(),
        })
    }

    /// The slot the store's interval clock currently sits in.
    #[must_use = "this reads the clock; ticking it is `on_tick`'s job"]
    pub const fn current_slot(&self) -> verity_types::Slot {
        verity_types::Slot(self.time.0 / verity_types::config::INTERVALS_PER_SLOT)
    }

    /// Whether one checkpoint lies on the other's chain of ancestors.
    ///
    /// The walk climbs parent links from `descendant` and stops the moment it leaves the
    /// known tree, so an unknown branch answers `false` rather than looping. Landing above
    /// the ancestor's slot without hitting it means that slot held no block on this chain,
    /// which puts the ancestor off it.
    #[must_use = "this answers the question; it neither records nor enforces the relation"]
    pub fn is_ancestor(&self, ancestor: Checkpoint, descendant: Checkpoint) -> bool {
        if ancestor.slot > descendant.slot {
            return false;
        }

        let mut current_root = descendant.root;
        while let Some(current_block) = self.blocks.get(&current_root) {
            match current_block.slot.cmp(&ancestor.slot) {
                Ordering::Equal => return current_root == ancestor.root,
                Ordering::Less => return false,
                Ordering::Greater => current_root = current_block.parent_root,
            }
        }
        false
    }

    /// The ancestor of `root` at `slot`, when the chain above it is fully known.
    ///
    /// Returns `None` where the walk leaves the known tree, or where the chain skips the
    /// slot entirely — a checkpoint-sync anchor leaves exactly that hole below itself.
    #[must_use = "this locates the ancestor; it does not move any checkpoint onto it"]
    pub fn ancestor_at_slot(&self, root: Bytes32, slot: verity_types::Slot) -> Option<Bytes32> {
        let mut current_root = root;
        loop {
            let current_block = self.blocks.get(&current_root)?;
            match current_block.slot.cmp(&slot) {
                Ordering::Equal => return Some(current_root),
                Ordering::Less => return None,
                Ordering::Greater => current_root = current_block.parent_root,
            }
        }
    }
}
