//! Gossip Arrival Metrics: how far from its due interval each gossiped item landed.

use prometheus::{Histogram, IntCounterVec, Registry};

use crate::labels::ArrivalPosition;
use crate::register::{counter_vec, histogram, seed};

/// Absolute delay from the due interval, in seconds.
const ARRIVAL_DELAY_BUCKETS: &[f64] = &[0.05, 0.1, 0.2, 0.4, 0.8, 1.2, 1.6, 2.4, 4.0, 8.0, 16.0];

/// Which gossip channel an arrival is recorded against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrivalKind {
    /// A block, due at interval 0 of its slot.
    Block,
    /// A vote, due at interval 1 of its slot.
    Attestation,
    /// An aggregate, measured from the most recent interval-2 boundary.
    Aggregation,
}

/// Delays and positions of gossip arrivals.
#[derive(Debug)]
pub struct GossipArrivalMetrics {
    /// `lean_gossip_block_arrival_delay_seconds`.
    pub block_arrival_delay_seconds: Histogram,
    /// `lean_gossip_attestation_arrival_delay_seconds`.
    pub attestation_arrival_delay_seconds: Histogram,
    /// `lean_gossip_aggregation_arrival_delay_seconds`.
    pub aggregation_arrival_delay_seconds: Histogram,
    /// `lean_gossip_block_arrival_total`, labelled `position`.
    pub block_arrival_total: IntCounterVec,
    /// `lean_gossip_attestation_arrival_total`, labelled `position`.
    pub attestation_arrival_total: IntCounterVec,
    /// `lean_gossip_aggregation_arrival_total`, labelled `position` (`inside`, `after`).
    pub aggregation_arrival_total: IntCounterVec,
}

impl GossipArrivalMetrics {
    pub(crate) fn register(registry: &Registry) -> prometheus::Result<Self> {
        let block_arrival_total = counter_vec(
            registry,
            "lean_gossip_block_arrival_total",
            "Gossip blocks by arrival position relative to the interval they were due in",
            &["position"],
        )?;
        seed(&block_arrival_total, &ArrivalPosition::labels());
        let attestation_arrival_total = counter_vec(
            registry,
            "lean_gossip_attestation_arrival_total",
            "Gossip attestations by arrival position relative to the interval they were due in",
            &["position"],
        )?;
        seed(&attestation_arrival_total, &ArrivalPosition::labels());
        let aggregation_arrival_total = counter_vec(
            registry,
            "lean_gossip_aggregation_arrival_total",
            "Gossip aggregates by arrival position relative to the most recent \
             aggregation-interval boundary",
            &["position"],
        )?;
        let non_negative: Vec<&str> = ArrivalPosition::NON_NEGATIVE
            .iter()
            .map(|position| position.label())
            .collect();
        seed(&aggregation_arrival_total, &non_negative);

        Ok(Self {
            block_arrival_delay_seconds: histogram(
                registry,
                "lean_gossip_block_arrival_delay_seconds",
                "Absolute delay between a gossip block's arrival and the start of the interval \
                 it was due in",
                ARRIVAL_DELAY_BUCKETS,
            )?,
            attestation_arrival_delay_seconds: histogram(
                registry,
                "lean_gossip_attestation_arrival_delay_seconds",
                "Absolute delay between a gossip attestation's arrival and the start of the \
                 interval it was due in",
                ARRIVAL_DELAY_BUCKETS,
            )?,
            aggregation_arrival_delay_seconds: histogram(
                registry,
                "lean_gossip_aggregation_arrival_delay_seconds",
                "Absolute delay between a gossip aggregate's arrival and the most recent \
                 aggregation-interval boundary at or before it",
                ARRIVAL_DELAY_BUCKETS,
            )?,
            block_arrival_total,
            attestation_arrival_total,
            aggregation_arrival_total,
        })
    }

    /// Records one arrival: its absolute delay in seconds, and where it landed.
    pub fn record(&self, kind: ArrivalKind, delay_seconds: f64, position: ArrivalPosition) {
        let (delay, total) = match kind {
            ArrivalKind::Block => (&self.block_arrival_delay_seconds, &self.block_arrival_total),
            ArrivalKind::Attestation => (
                &self.attestation_arrival_delay_seconds,
                &self.attestation_arrival_total,
            ),
            ArrivalKind::Aggregation => (
                &self.aggregation_arrival_delay_seconds,
                &self.aggregation_arrival_total,
            ),
        };
        delay.observe(delay_seconds);
        total.with_label_values(&[position.label()]).inc();
    }
}
