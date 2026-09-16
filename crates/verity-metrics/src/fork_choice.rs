//! Fork-Choice Metrics: the head, the pools behind it, and what moving it costs.

use prometheus::{Histogram, IntCounter, IntGauge, IntGaugeVec, Registry};

use crate::labels::SyncStatus;
use crate::register::{counter, gauge, gauge_vec, histogram};

/// Importing a block into the store, in seconds.
const BLOCK_PROCESSING_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 1.0, 1.25, 1.5, 2.0, 4.0];

/// Validating one vote, in seconds.
const ATTESTATION_VALIDATION_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 1.0];

/// Blocks reverted by a reorg.
const REORG_DEPTH_BUCKETS: &[f64] = &[1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 20.0, 30.0, 50.0, 100.0];

/// One aggregation round, in seconds.
const AGGREGATION_ROUND_BUCKETS: &[f64] = &[0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 2.0, 3.0, 4.0];

/// Time between clock ticks, in seconds, dense around the 0.8 s interval.
const TICK_INTERVAL_BUCKETS: &[f64] = &[
    0.4, 0.6, 0.75, 0.8, 0.805, 0.81, 0.815, 0.82, 0.825, 0.85, 0.9, 1.0, 1.2, 1.6,
];

/// Gauges, counters and timings of fork choice.
#[derive(Debug)]
pub struct ForkChoiceMetrics {
    /// `lean_head_slot`: the slot of the fork-choice head.
    pub head_slot: IntGauge,
    /// `lean_current_slot`: the slot the clock sits in.
    pub current_slot: IntGauge,
    /// `lean_safe_target_slot`: the slot of the safe target.
    pub safe_target_slot: IntGauge,
    /// `lean_fork_choice_block_processing_time_seconds`.
    pub block_processing_time_seconds: Histogram,
    /// `lean_attestations_valid_total`: votes that passed gossip validation.
    pub attestations_valid_total: IntCounter,
    /// `lean_attestations_invalid_total`: votes that did not.
    pub attestations_invalid_total: IntCounter,
    /// `lean_attestation_validation_time_seconds`.
    pub attestation_validation_time_seconds: Histogram,
    /// `lean_fork_choice_reorgs_total`.
    pub reorgs_total: IntCounter,
    /// `lean_fork_choice_reorg_depth`, in blocks.
    pub reorg_depth: Histogram,
    /// `lean_gossip_signatures`: per-validator signatures held in the store.
    pub gossip_signatures: IntGauge,
    /// `lean_latest_new_aggregated_payloads`: proofs gathered this slot, not yet counted.
    pub latest_new_aggregated_payloads: IntGauge,
    /// `lean_latest_known_aggregated_payloads`: proofs counting toward weight.
    pub latest_known_aggregated_payloads: IntGauge,
    /// `lean_committee_signatures_aggregation_time_seconds`: one aggregation round.
    pub committee_signatures_aggregation_time_seconds: Histogram,
    /// `lean_node_sync_status`, labelled `status`. Exactly one label value is 1.
    pub node_sync_status: IntGaugeVec,
    /// `lean_tick_interval_duration_seconds`.
    pub tick_interval_duration_seconds: Histogram,
}

impl ForkChoiceMetrics {
    pub(crate) fn register(registry: &Registry) -> prometheus::Result<Self> {
        let metrics = Self {
            head_slot: gauge(registry, "lean_head_slot", "Latest slot of the lean chain")?,
            current_slot: gauge(
                registry,
                "lean_current_slot",
                "Current slot of the lean chain",
            )?,
            safe_target_slot: gauge(registry, "lean_safe_target_slot", "Safe target slot")?,
            block_processing_time_seconds: histogram(
                registry,
                "lean_fork_choice_block_processing_time_seconds",
                "Time taken to process block",
                BLOCK_PROCESSING_BUCKETS,
            )?,
            attestations_valid_total: counter(
                registry,
                "lean_attestations_valid_total",
                "Total number of valid attestations",
            )?,
            attestations_invalid_total: counter(
                registry,
                "lean_attestations_invalid_total",
                "Total number of invalid attestations",
            )?,
            attestation_validation_time_seconds: histogram(
                registry,
                "lean_attestation_validation_time_seconds",
                "Time taken to validate attestation",
                ATTESTATION_VALIDATION_BUCKETS,
            )?,
            reorgs_total: counter(
                registry,
                "lean_fork_choice_reorgs_total",
                "Total number of fork choice reorgs",
            )?,
            reorg_depth: histogram(
                registry,
                "lean_fork_choice_reorg_depth",
                "Depth of fork choice reorgs (in blocks)",
                REORG_DEPTH_BUCKETS,
            )?,
            gossip_signatures: gauge(
                registry,
                "lean_gossip_signatures",
                "Number of gossip signatures in fork-choice store",
            )?,
            latest_new_aggregated_payloads: gauge(
                registry,
                "lean_latest_new_aggregated_payloads",
                "Number of new aggregated payload items",
            )?,
            latest_known_aggregated_payloads: gauge(
                registry,
                "lean_latest_known_aggregated_payloads",
                "Number of known aggregated payload items",
            )?,
            committee_signatures_aggregation_time_seconds: histogram(
                registry,
                "lean_committee_signatures_aggregation_time_seconds",
                "Time taken to aggregate committee signatures",
                AGGREGATION_ROUND_BUCKETS,
            )?,
            node_sync_status: gauge_vec(
                registry,
                "lean_node_sync_status",
                "Node sync status",
                &["status"],
            )?,
            tick_interval_duration_seconds: histogram(
                registry,
                "lean_tick_interval_duration_seconds",
                "Elapsed time between clock ticks in seconds",
                TICK_INTERVAL_BUCKETS,
            )?,
        };
        // A node that has met nobody is idle; setting it here is what puts all three status
        // series in the first scrape.
        metrics.set_sync_status(SyncStatus::Idle);
        Ok(metrics)
    }

    /// Sets the sync status: the named label to 1, every other to 0.
    ///
    /// All three series are always present, so a dashboard's `max by (status)` reads the
    /// same shape from every client whether or not a status has ever been active.
    pub fn set_sync_status(&self, status: SyncStatus) {
        for candidate in SyncStatus::ALL {
            let value = i64::from(*candidate == status);
            self.node_sync_status
                .with_label_values(&[candidate.label()])
                .set(value);
        }
    }

    /// Records a reorg of `depth` blocks.
    pub fn record_reorg(&self, depth: u64) {
        self.reorgs_total.inc();
        // Depth is a block count; the lossy cast only matters past 2^53 blocks.
        self.reorg_depth.observe(depth as f64);
    }
}
