//! The interval clock: what the store does at each fixed point inside a slot.
//!
//! A slot is five intervals, and consensus work is pinned to positions within it rather than
//! to arrival times, so every node acts on the same schedule relative to a block landing.
//!
//! One of leanSpec's five actions is absent. At interval 2 an aggregator folds the slot's
//! pooled signatures into proofs, which is a cryptographic operation and belongs to
//! `verity-crypto` (see the crate docs); the composed tick lands there. The four actions
//! below are decisions over the store alone, and they are complete.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/timeline.py` and
//! `fork_choice.py`, read at commit `0588c2d215a955a516378677a92db2a5666802f3`.

use verity_types::Interval;
use verity_types::config::INTERVALS_PER_SLOT;

use crate::fork_choice::block::update_head;
use crate::fork_choice::store::Store;
use crate::fork_choice::weights::{latest_votes, lmd_ghost_head};

/// Advances the store's clock to `target`, running every interval it passes through.
///
/// Stepping one interval at a time is what keeps an action from being skipped when a node
/// falls behind and catches up across several intervals at once.
///
/// `has_proposal` says the node expects this slot's block to have landed; it is signalled
/// only on the final step, since it describes the interval being arrived at rather than the
/// ones passed on the way. A `target` at or behind the current time does nothing.
pub fn on_tick(store: &mut Store, target: Interval, has_proposal: bool) {
    while store.time.0 < target.0 {
        let next = Interval(store.time.0 + 1);
        tick_interval(store, has_proposal && next.0 == target.0);
    }
}

/// Advances one interval and runs whatever that position in the slot calls for.
///
/// - Interval 0 — ingest the slot's pending votes, once the proposal has landed.
/// - Interval 3 — advance the safe target, after aggregates for this slot exist.
/// - Interval 4 — ingest the votes that accumulated through the rest of the slot.
///
/// Interval 1 has no action, and interval 2 is the aggregator's, which is not this crate's
/// (see the module docs).
fn tick_interval(store: &mut Store, has_proposal: bool) {
    store.time = Interval(store.time.0 + 1);

    match store.time.0 % INTERVALS_PER_SLOT {
        0 if has_proposal => accept_new_attestations(store),
        3 => update_safe_target(store),
        4 => accept_new_attestations(store),
        _ => {}
    }
}

/// Promotes the pending proofs into the counted pool and recomputes the head.
///
/// Proofs gathered during a slot carry no weight while they sit pending. This is the point
/// at which they begin to count, and the pending pool is emptied behind them.
pub fn accept_new_attestations(store: &mut Store) {
    let pending = core::mem::take(&mut store.latest_new_aggregated_payloads);
    for (data, proofs) in pending {
        store
            .latest_known_aggregated_payloads
            .entry(data)
            .or_default()
            .extend(proofs);
    }
    update_head(store);
}

/// Advances the safe target: the deepest block a supermajority of this slot's voters back.
///
/// This is the block a validator can attest to without risking that it later disappears, so
/// the threshold is a strict supermajority and is rounded up — 100 validators need 67, not
/// 66. Children below it are pruned before the walk, which is why the target can stop
/// shallower than the head.
///
/// It is weighed from the *pending* pool, not the counted one: the safe target is about what
/// this slot's voters are doing now, and the counted pool is last slot's picture.
pub fn update_safe_target(store: &mut Store) {
    let validator_count = store
        .states
        .get(&store.head)
        .map_or(0, |state| state.validators.len() as u64);
    let min_target_score = (validator_count * 2).div_ceil(3);

    let votes = latest_votes(
        &store.latest_new_aggregated_payloads,
        store.latest_finalized.slot,
    );
    store.safe_target = lmd_ghost_head(
        store,
        store.latest_justified.root,
        &votes,
        Some(min_target_score),
    );
}
