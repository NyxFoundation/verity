//! Validator Metrics: the duties this node runs, and the ones it skipped.

use prometheus::{Histogram, IntCounterVec, IntGauge, Registry};

use crate::labels::SkipReason;
use crate::register::{counter_vec, gauge, histogram, seed};

/// Producing a slot's attestation, in seconds.
const PRODUCTION_BUCKETS: &[f64] = &[0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0];

/// Gauges, counters and timings of validator duties.
#[derive(Debug)]
pub struct ValidatorMetrics {
    /// `lean_validators_count`: validators this node runs.
    pub validators_count: IntGauge,
    /// `lean_is_aggregator`: 1 when this node aggregates.
    pub is_aggregator: IntGauge,
    /// `lean_attestations_production_time_seconds`.
    pub attestations_production_time_seconds: Histogram,
    /// `lean_aggregator_skipped_total`, labelled `reason`.
    pub aggregator_skipped_total: IntCounterVec,
}

impl ValidatorMetrics {
    pub(crate) fn register(registry: &Registry) -> prometheus::Result<Self> {
        let aggregator_skipped_total = counter_vec(
            registry,
            "lean_aggregator_skipped_total",
            "Total number of aggregation submissions skipped, labeled by reason",
            &["reason"],
        )?;
        seed(&aggregator_skipped_total, &SkipReason::labels());

        Ok(Self {
            validators_count: gauge(
                registry,
                "lean_validators_count",
                "Number of validators managed by a node",
            )?,
            is_aggregator: gauge(
                registry,
                "lean_is_aggregator",
                "Validator's is_aggregator status. True=1, False=0",
            )?,
            attestations_production_time_seconds: histogram(
                registry,
                "lean_attestations_production_time_seconds",
                "Time taken to produce attestation",
                PRODUCTION_BUCKETS,
            )?,
            aggregator_skipped_total,
        })
    }

    /// Records one aggregation cycle that produced nothing, and why.
    pub fn record_aggregation_skipped(&self, reason: SkipReason) {
        self.aggregator_skipped_total
            .with_label_values(&[reason.label()])
            .inc();
    }
}
