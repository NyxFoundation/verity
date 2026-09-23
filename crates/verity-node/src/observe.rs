//! The node's probes into leanMetrics: the samples that need a clock, a store, or the wire.
//!
//! `verity-metrics` registers and renders; `verity-chain` reports stage boundaries without
//! keeping time; `verity-p2p` reports connection facts without naming a metric. What is left
//! — reading the clock at each boundary, walking the block tree to size a reorg, placing an
//! arrival against the interval it was due in, and translating wire enums into label enums —
//! is here, because this crate is the one that holds all three.

use std::collections::HashMap;
use std::time::Instant;

use verity_chain::{SlotClock, TransitionEvent, TransitionObserver};
use verity_metrics::{
    ArrivalKind, ArrivalPosition, ConnectionResult, Direction, DisconnectReason,
    FinalizationResult, Metrics, observe_elapsed,
};
use verity_types::config::{
    GOSSIP_DISPARITY_INTERVALS, INTERVALS_PER_SLOT, MILLISECONDS_PER_INTERVAL,
    MILLISECONDS_PER_SLOT,
};
use verity_types::{Block, Bytes32, Interval, Slot};

use crate::verification::VerificationFailure;

/// Longest walk a reorg measurement makes before giving up on an exact depth.
///
/// A reorg deeper than this is a network event worth a log line, not a histogram bucket:
/// the largest bucket the contract fixes is 100.
const MAX_REORG_WALK: u64 = 128;

/// Times the state transition's stages as the chain reports their boundaries.
///
/// The chain emits `Begin` and `End` events and nothing else; the instants are taken here,
/// which keeps the transition free of any clock (`verity_chain::state_transition::observe`).
pub struct TransitionRecorder<'a> {
    metrics: &'a Metrics,
    transition: Option<Instant>,
    slots: Option<Instant>,
    block: Option<Instant>,
    attestations: Option<Instant>,
}

impl<'a> TransitionRecorder<'a> {
    /// A recorder for one transition.
    #[must_use]
    pub const fn new(metrics: &'a Metrics) -> Self {
        Self {
            metrics,
            transition: None,
            slots: None,
            block: None,
            attestations: None,
        }
    }
}

impl TransitionObserver for TransitionRecorder<'_> {
    fn observe(&mut self, event: TransitionEvent) {
        let transition = &self.metrics.transition;
        match event {
            TransitionEvent::TransitionBegin => self.transition = Some(Instant::now()),
            TransitionEvent::SlotsBegin => self.slots = Some(Instant::now()),
            TransitionEvent::SlotsEnd { processed } => {
                transition.slots_processed_total.inc_by(processed);
                if let Some(started) = self.slots.take() {
                    observe_elapsed(&transition.slots_processing_time_seconds, started);
                }
            }
            TransitionEvent::BlockBegin => self.block = Some(Instant::now()),
            TransitionEvent::AttestationsBegin => self.attestations = Some(Instant::now()),
            TransitionEvent::AttestationsEnd { processed } => {
                transition
                    .attestations_processed_total
                    .inc_by(processed as u64);
                if let Some(started) = self.attestations.take() {
                    observe_elapsed(&transition.attestations_processing_time_seconds, started);
                }
            }
            TransitionEvent::BlockEnd => {
                if let Some(started) = self.block.take() {
                    observe_elapsed(&transition.block_processing_time_seconds, started);
                }
            }
            TransitionEvent::TransitionEnd => {
                if let Some(started) = self.transition.take() {
                    observe_elapsed(&transition.time_seconds, started);
                }
            }
            TransitionEvent::FinalizationAttempt { advanced } => {
                transition.record_finalization(if advanced {
                    FinalizationResult::Success
                } else {
                    FinalizationResult::Error
                });
            }
        }
    }
}

/// How many blocks a head move reverted, or `None` when the new head extends the old one.
///
/// The depth is the number of blocks on the old head's chain above the two heads' common
/// ancestor — the blocks that stopped being canonical. Both walks descend by slot until the
/// roots meet; two heads in one store always meet at or above the anchor, and a walk that
/// leaves the known tree anyway answers `None` rather than a number nothing supports.
#[must_use]
pub fn reorg_depth(
    blocks: &HashMap<Bytes32, Block>,
    old_head: Bytes32,
    new_head: Bytes32,
) -> Option<u64> {
    let (mut old, mut new) = (old_head, new_head);
    let mut depth = 0;
    let mut walked = 0;
    while old != new {
        if walked >= MAX_REORG_WALK {
            return Some(depth);
        }
        walked += 1;
        let old_block = blocks.get(&old)?;
        let new_block = blocks.get(&new)?;
        // The deeper head steps first; at equal slots both step, since neither can be the
        // other's ancestor.
        if old_block.slot >= new_block.slot {
            old = old_block.parent_root;
            depth += 1;
        }
        if new_block.slot >= old_block.slot {
            new = new_block.parent_root;
        }
    }
    (depth > 0).then_some(depth)
}

/// Times one aggregate-proof check and counts which way it went.
///
/// # Errors
///
/// [`VerificationFailure::Invalid`] when the proof does not hold.
pub fn check_aggregate_proof(
    metrics: &Metrics,
    check: impl FnOnce() -> Result<(), verity_crypto::AggregationError>,
) -> Result<(), VerificationFailure> {
    let signature = &metrics.signature;
    let started = Instant::now();
    let outcome = check();
    observe_elapsed(
        &signature.aggregated_signatures_verification_time_seconds,
        started,
    );
    match outcome {
        Ok(()) => {
            signature.aggregated_signatures_valid_total.inc();
            Ok(())
        }
        Err(_) => {
            signature.aggregated_signatures_invalid_total.inc();
            Err(VerificationFailure::Invalid)
        }
    }
}

/// Whether an arrival at `slot` is worth measuring against the local clock.
///
/// An item from a slot that has not started here, beyond the gossip disparity margin, is
/// measured against a boundary this node's clock has not reached; the sample would say more
/// about clock skew than about propagation. The same guard ethlambda applies.
#[must_use]
pub fn is_arrival_observable(view_time: Interval, slot: Slot) -> bool {
    slot.0.saturating_mul(INTERVALS_PER_SLOT)
        <= view_time.0.saturating_add(GOSSIP_DISPARITY_INTERVALS)
}

/// The interval within a slot at which each gossip channel's item is due.
const fn due_interval(kind: ArrivalKind) -> u64 {
    match kind {
        ArrivalKind::Block => 0,
        ArrivalKind::Attestation => 1,
        ArrivalKind::Aggregation => 2,
    }
}

/// Records where an item arriving at `now_milliseconds` landed relative to its due interval.
///
/// Blocks and attestations are anchored at their own slot's due interval, so an early arrival
/// is `before`. An aggregate is anchored at the most recent aggregation boundary at or before
/// its arrival — its data slot may lag the slot it was published in — so its delay is never
/// negative and `before` is unreachable.
pub fn record_arrival(
    metrics: &Metrics,
    clock: &SlotClock,
    kind: ArrivalKind,
    slot: Slot,
    now_milliseconds: u64,
) {
    let delta = match kind {
        ArrivalKind::Aggregation => {
            latest_boundary_delta(clock, now_milliseconds, due_interval(kind))
        }
        ArrivalKind::Block | ArrivalKind::Attestation => {
            anchored_delta(clock, now_milliseconds, slot, due_interval(kind))
        }
    };
    let position = if delta < 0 {
        ArrivalPosition::Before
    } else if delta < MILLISECONDS_PER_INTERVAL as i64 {
        ArrivalPosition::Inside
    } else {
        ArrivalPosition::After
    };
    // Milliseconds to seconds; the precision loss is far below the smallest bucket.
    let delay_seconds = delta.unsigned_abs() as f64 / 1000.0;
    metrics.arrival.record(kind, delay_seconds, position);
}

/// Signed milliseconds from the start of `interval` in `slot` to `now`.
fn anchored_delta(clock: &SlotClock, now: u64, slot: Slot, interval: u64) -> i64 {
    let expected = clock
        .genesis_time()
        .saturating_mul(1000)
        .saturating_add(slot.0.saturating_mul(MILLISECONDS_PER_SLOT))
        .saturating_add(interval.saturating_mul(MILLISECONDS_PER_INTERVAL));
    saturating_i64(now) - saturating_i64(expected)
}

/// Milliseconds since the most recent `interval` boundary at or before `now`; always in
/// `[0, MILLISECONDS_PER_SLOT)`.
fn latest_boundary_delta(clock: &SlotClock, now: u64, interval: u64) -> i64 {
    let since_genesis = saturating_i64(clock.milliseconds_since_genesis(now));
    let offset = saturating_i64(interval.saturating_mul(MILLISECONDS_PER_INTERVAL));
    (since_genesis - offset).rem_euclid(MILLISECONDS_PER_SLOT as i64)
}

fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The wire's connection direction, as the contract labels it.
#[must_use]
pub const fn direction(direction: verity_p2p::Direction) -> Direction {
    match direction {
        verity_p2p::Direction::Inbound => Direction::Inbound,
        verity_p2p::Direction::Outbound => Direction::Outbound,
    }
}

/// The wire's connection failure, as the contract labels it.
#[must_use]
pub const fn connection_failure(failure: verity_p2p::ConnectFailure) -> ConnectionResult {
    match failure {
        verity_p2p::ConnectFailure::Timeout => ConnectionResult::Timeout,
        verity_p2p::ConnectFailure::Error => ConnectionResult::Error,
    }
}

/// The wire's disconnection reason, as the contract labels it.
#[must_use]
pub const fn disconnect_reason(reason: verity_p2p::DisconnectReason) -> DisconnectReason {
    match reason {
        verity_p2p::DisconnectReason::Timeout => DisconnectReason::Timeout,
        verity_p2p::DisconnectReason::RemoteClose => DisconnectReason::RemoteClose,
        verity_p2p::DisconnectReason::LocalClose => DisconnectReason::LocalClose,
        verity_p2p::DisconnectReason::Error => DisconnectReason::Error,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use verity_chain::SlotClock;
    use verity_metrics::{ArrivalKind, Metrics};
    use verity_types::config::{MILLISECONDS_PER_INTERVAL, MILLISECONDS_PER_SLOT};
    use verity_types::{Block, BlockBody, Bytes32, Interval, Slot, ValidatorIndex};

    use super::{is_arrival_observable, record_arrival, reorg_depth};

    fn block(slot: u64, parent: Bytes32) -> Block {
        Block {
            slot: Slot(slot),
            proposer_index: ValidatorIndex(0),
            parent_root: parent,
            state_root: [0; 32],
            body: BlockBody::default(),
        }
    }

    /// A tree: anchor `a` at slot 0, chain `a → b(1) → c(2)`, and a sibling `d(2)` under `b`.
    fn tree() -> HashMap<Bytes32, Block> {
        let (a, b, c, d) = ([1; 32], [2; 32], [3; 32], [4; 32]);
        HashMap::from([
            (a, block(0, [0; 32])),
            (b, block(1, a)),
            (c, block(2, b)),
            (d, block(2, b)),
        ])
    }

    #[test]
    fn should_report_no_reorg_when_the_head_extends() {
        assert_eq!(reorg_depth(&tree(), [2; 32], [3; 32]), None);
        assert_eq!(reorg_depth(&tree(), [3; 32], [3; 32]), None);
    }

    #[test]
    fn should_measure_a_switch_between_siblings_as_one_block() {
        assert_eq!(reorg_depth(&tree(), [3; 32], [4; 32]), Some(1));
    }

    #[test]
    fn should_measure_a_move_back_onto_an_ancestor_by_the_blocks_reverted() {
        // From `c` (slot 2) back to the anchor `a` drops `c` and `b`.
        assert_eq!(reorg_depth(&tree(), [3; 32], [1; 32]), Some(2));
    }

    #[test]
    fn should_answer_none_when_a_head_is_outside_the_tree() {
        assert_eq!(reorg_depth(&tree(), [9; 32], [3; 32]), None);
    }

    #[test]
    fn should_place_an_arrival_against_the_interval_it_was_due_in() {
        let metrics = Metrics::new().expect("registry");
        let clock = SlotClock::new(1_000);
        let genesis_ms = 1_000_000;
        // Slot 3's block is due at interval 0: 300 ms early, then 100 ms in, then late.
        let due = genesis_ms + 3 * MILLISECONDS_PER_SLOT;
        record_arrival(&metrics, &clock, ArrivalKind::Block, Slot(3), due - 300);
        record_arrival(&metrics, &clock, ArrivalKind::Block, Slot(3), due + 100);
        record_arrival(
            &metrics,
            &clock,
            ArrivalKind::Block,
            Slot(3),
            due + MILLISECONDS_PER_INTERVAL,
        );
        // An aggregate 50 ms after slot 3's interval 2, whatever its data slot says.
        let boundary = due + 2 * MILLISECONDS_PER_INTERVAL;
        record_arrival(
            &metrics,
            &clock,
            ArrivalKind::Aggregation,
            Slot(1),
            boundary + 50,
        );

        let text = metrics.render();
        for line in [
            "lean_gossip_block_arrival_total{position=\"before\"} 1",
            "lean_gossip_block_arrival_total{position=\"inside\"} 1",
            "lean_gossip_block_arrival_total{position=\"after\"} 1",
            "lean_gossip_block_arrival_delay_seconds_bucket{le=\"0.4\"} 2",
            "lean_gossip_aggregation_arrival_total{position=\"inside\"} 1",
            "lean_gossip_aggregation_arrival_delay_seconds_bucket{le=\"0.05\"} 1",
        ] {
            assert!(text.contains(line), "missing `{line}` in:\n{text}");
        }
    }

    #[test]
    fn should_not_observe_an_arrival_from_a_slot_the_clock_has_not_reached() {
        assert!(is_arrival_observable(Interval(10), Slot(2)));
        assert!(
            is_arrival_observable(Interval(14), Slot(3)),
            "one interval of disparity"
        );
        assert!(!is_arrival_observable(Interval(13), Slot(3)));
    }
}

#[cfg(kani)]
mod harnesses {
    use verity_chain::SlotClock;
    use verity_types::config::{
        GOSSIP_DISPARITY_INTERVALS, INTERVALS_PER_SLOT, MILLISECONDS_PER_INTERVAL,
        MILLISECONDS_PER_SLOT,
    };
    use verity_types::{Interval, Slot};

    use super::{anchored_delta, is_arrival_observable, latest_boundary_delta, saturating_i64};

    /// A genesis time whose millisecond rendering fits a `u64`.
    fn any_clock() -> SlotClock {
        let genesis_time: u64 = kani::any();
        kani::assume(genesis_time <= u64::MAX / 1000);
        SlotClock::new(genesis_time)
    }

    /// Observability is total and admits exactly the slots the clock has reached, give or
    /// take the gossip disparity margin.
    #[kani::proof]
    fn observability_is_total_and_bounded_by_the_clock() {
        let view_time: u64 = kani::any();
        let slot: u64 = kani::any();
        let observable = is_arrival_observable(Interval(view_time), Slot(slot));
        if slot <= u64::MAX / INTERVALS_PER_SLOT
            && view_time <= u64::MAX - GOSSIP_DISPARITY_INTERVALS
        {
            assert!(
                observable == (slot * INTERVALS_PER_SLOT <= view_time + GOSSIP_DISPARITY_INTERVALS)
            );
        }
    }

    /// The boundary delta always lands inside one slot, and the narrowing saturates.
    #[kani::proof]
    fn the_boundary_delta_stays_inside_the_slot() {
        let clock = any_clock();
        let now: u64 = kani::any();
        let interval: u64 = kani::any();
        kani::assume(interval < INTERVALS_PER_SLOT);
        let delta = latest_boundary_delta(&clock, now, interval);
        assert!(delta >= 0);
        assert!(delta < MILLISECONDS_PER_SLOT as i64);

        let value: u64 = kani::any();
        let narrowed = saturating_i64(value);
        assert!(narrowed >= 0);
        assert!(u64::try_from(narrowed).unwrap() <= value);
    }

    /// The anchored delta is the signed distance to the due instant, never a panic.
    #[kani::proof]
    fn the_anchored_delta_is_a_signed_distance() {
        let clock = any_clock();
        let now: u64 = kani::any();
        let slot: u64 = kani::any();
        let interval: u64 = kani::any();
        kani::assume(interval < INTERVALS_PER_SLOT);
        let delta = anchored_delta(&clock, now, Slot(slot), interval);
        let genesis_ms = clock.genesis_time() * 1000;
        if let Some(due) = slot
            .checked_mul(MILLISECONDS_PER_SLOT)
            .and_then(|at| at.checked_add(genesis_ms))
            .and_then(|at| at.checked_add(interval * MILLISECONDS_PER_INTERVAL))
            && due <= i64::MAX as u64
            && now <= i64::MAX as u64
        {
            assert!(delta == now as i64 - due as i64);
        }
    }
}
