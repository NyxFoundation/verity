//! Turning a pool of votes into a head: the LMD view, the weights, and the GHOST walk.
//!
//! Nothing here reads or writes the store's pools. Each function takes the pool it should
//! reason about, which is what lets the same code weigh the counted pool for the head and
//! the pending pool for the safe target.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/fork_choice.py`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`.

use std::collections::HashMap;
use std::collections::HashSet;

use verity_types::{
    AggregationBits, AttestationData, Bytes32, SingleMessageAggregate, Slot, ValidatorIndex,
};

use crate::fork_choice::store::Store;
use crate::merkle::hash_tree_root;

/// The pools the store keys by vote: proofs filed under the attestation data they cover.
pub type AggregatedPayloads = HashMap<AttestationData, HashSet<SingleMessageAggregate>>;

/// Each validator mapped to the latest vote it cast — the LMD view fork choice runs on.
pub type LatestVotes = HashMap<ValidatorIndex, AttestationData>;

/// The validator indices a bitfield names.
#[must_use = "this reads the bitfield; it does not modify it"]
pub fn participants(bits: &AggregationBits) -> impl Iterator<Item = ValidatorIndex> + '_ {
    (0..bits.len())
        .filter(|index| bits.get(*index).unwrap_or(false))
        .map(|index| ValidatorIndex(index as u64))
}

/// Reduces a pool of proofs to each validator's latest still-relevant vote.
///
/// Votes are visited newest-first, so the first one seen for a validator is the one that
/// counts. An equivocator casting two votes in one slot is settled by the larger canonical
/// attestation-data root, the same tiebreak the block walk applies to block roots — which is
/// what makes the result independent of arrival order.
///
/// A vote whose head sits at or below the finalized slot can credit no block the walk
/// reaches, so it is skipped here and callers need not pre-filter their pool.
#[must_use = "this derives the LMD view; the pool it reads is left untouched"]
pub fn latest_votes(payloads: &AggregatedPayloads, latest_finalized_slot: Slot) -> LatestVotes {
    let mut by_precedence: Vec<(&AttestationData, &HashSet<SingleMessageAggregate>)> =
        payloads.iter().collect();
    // `hash_tree_root` runs once per distinct vote here, never once per validator below.
    by_precedence
        .sort_unstable_by_key(|(data, _)| core::cmp::Reverse((data.slot, hash_tree_root(*data))));

    let mut latest = LatestVotes::new();
    for (data, proofs) in by_precedence {
        if data.head.slot.0 <= latest_finalized_slot.0 {
            continue;
        }
        // Every proof filed here covers this same vote, so which one the loop visits cannot
        // change what gets recorded. Set order is therefore non-consensus.
        for proof in proofs {
            for validator_index in participants(&proof.participants) {
                latest.entry(validator_index).or_insert(*data);
            }
        }
    }
    latest
}

/// Tallies how many of those latest votes credit each block.
///
/// A vote credits its head and every ancestor above `start_slot`. The climb stops at that
/// slot, or where the chain leaves the known tree.
#[must_use = "this returns the tally; it stores nothing on the store it reads"]
pub fn ancestor_weights(
    store: &Store,
    attestations: &LatestVotes,
    start_slot: Slot,
) -> HashMap<Bytes32, u64> {
    let mut weights: HashMap<Bytes32, u64> = HashMap::new();

    for data in attestations.values() {
        let mut current_root = data.head.root;
        while let Some(current_block) = store.blocks.get(&current_root) {
            if current_block.slot.0 <= start_slot.0 {
                break;
            }
            *weights.entry(current_root).or_default() += 1;
            current_root = current_block.parent_root;
        }
    }
    weights
}

/// Weighs every block by the latest votes landing on it or on its descendants.
///
/// The anchor is the finalized slot: fork choice never reconsiders anything at or below it,
/// so nothing there is weighed.
#[must_use = "this returns the weights; it caches nothing on the store"]
pub fn block_weights(store: &Store) -> HashMap<Bytes32, u64> {
    let votes = latest_votes(
        &store.latest_known_aggregated_payloads,
        store.latest_finalized.slot,
    );
    ancestor_weights(store, &votes, store.latest_finalized.slot)
}

/// Walks the block tree by the LMD-GHOST rule and returns the leaf it lands on.
///
/// From `start_root`, each step takes the heaviest child, breaking an equal-weight tie toward
/// the lexicographically larger root. `min_score` prunes children below a threshold before
/// the walk, which is how the safe target stops shallower than the head.
///
/// An unknown `start_root` has no children and is returned unchanged; the store's own
/// invariant is that the justified root is always present.
#[must_use = "this selects the head; setting it on the store is `update_head`'s job"]
pub fn lmd_ghost_head(
    store: &Store,
    start_root: Bytes32,
    attestations: &LatestVotes,
    min_score: Option<u64>,
) -> Bytes32 {
    let start_slot = store
        .blocks
        .get(&start_root)
        .map_or(Slot(0), |block| block.slot);
    let weights = ancestor_weights(store, attestations, start_slot);
    let weight_of = |root: &Bytes32| weights.get(root).copied().unwrap_or_default();

    let mut children: HashMap<Bytes32, Vec<Bytes32>> = HashMap::new();
    for (root, block) in &store.blocks {
        // An anchor block naming itself as parent would make the walk below loop forever.
        // Nothing imported can reach that shape — `on_block` requires a known parent state —
        // but a caller-supplied anchor is not checked anywhere else.
        if block.parent_root == *root {
            continue;
        }
        if min_score.is_some_and(|threshold| weight_of(root) < threshold) {
            continue;
        }
        children.entry(block.parent_root).or_default().push(*root);
    }

    let mut head = start_root;
    while let Some(candidates) = children.get(&head) {
        let Some(best) = candidates
            .iter()
            .max_by_key(|child| (weight_of(child), **child))
        else {
            break;
        };
        head = *best;
    }
    head
}
