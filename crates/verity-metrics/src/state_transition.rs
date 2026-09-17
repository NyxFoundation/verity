//! State Transition Metrics: the checkpoints a block moved, and what applying it cost.

use prometheus::{Histogram, IntCounter, IntCounterVec, IntGauge, Registry};

use crate::labels::FinalizationResult;
use crate::register::{counter, counter_vec, gauge, histogram, seed};

/// The whole transition, in seconds.
const TRANSITION_BUCKETS: &[f64] = &[0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0, 4.0];

/// One stage of the transition, in seconds.
const STAGE_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 1.0];

/// Gauges, counters and timings of the state transition.
#[derive(Debug)]
pub struct StateTransitionMetrics {
    /// `lean_latest_justified_slot`.
    pub latest_justified_slot: IntGauge,
    /// `lean_latest_finalized_slot`.
    pub latest_finalized_slot: IntGauge,
    /// `lean_justified_slot`.
    pub justified_slot: IntGauge,
    /// `lean_finalized_slot`.
    pub finalized_slot: IntGauge,
    /// `lean_finalizations_total`, labelled `result` (`success`, `error`).
    pub finalizations_total: IntCounterVec,
    /// `lean_state_transition_time_seconds`.
    pub time_seconds: Histogram,
    /// `lean_state_transition_slots_processed_total`.
    pub slots_processed_total: IntCounter,
    /// `lean_state_transition_slots_processing_time_seconds`.
    pub slots_processing_time_seconds: Histogram,
    /// `lean_state_transition_block_processing_time_seconds`.
    pub block_processing_time_seconds: Histogram,
    /// `lean_state_transition_attestations_processed_total`.
    pub attestations_processed_total: IntCounter,
    /// `lean_state_transition_attestations_processing_time_seconds`.
    pub attestations_processing_time_seconds: Histogram,
}

impl StateTransitionMetrics {
    pub(crate) fn register(registry: &Registry) -> prometheus::Result<Self> {
        let finalizations_total = counter_vec(
            registry,
            "lean_finalizations_total",
            "Total number of finalization attempts",
            &["result"],
        )?;
        seed(&finalizations_total, &FinalizationResult::labels());

        Ok(Self {
            latest_justified_slot: gauge(
                registry,
                "lean_latest_justified_slot",
                "Latest justified slot",
            )?,
            latest_finalized_slot: gauge(
                registry,
                "lean_latest_finalized_slot",
                "Latest finalized slot",
            )?,
            justified_slot: gauge(registry, "lean_justified_slot", "Current justified slot")?,
            finalized_slot: gauge(registry, "lean_finalized_slot", "Current finalized slot")?,
            finalizations_total,
            time_seconds: histogram(
                registry,
                "lean_state_transition_time_seconds",
                "Time to process state transition",
                TRANSITION_BUCKETS,
            )?,
            slots_processed_total: counter(
                registry,
                "lean_state_transition_slots_processed_total",
                "Total number of processed slots",
            )?,
            slots_processing_time_seconds: histogram(
                registry,
                "lean_state_transition_slots_processing_time_seconds",
                "Time taken to process slots",
                STAGE_BUCKETS,
            )?,
            block_processing_time_seconds: histogram(
                registry,
                "lean_state_transition_block_processing_time_seconds",
                "Time taken to process block",
                STAGE_BUCKETS,
            )?,
            attestations_processed_total: counter(
                registry,
                "lean_state_transition_attestations_processed_total",
                "Total number of processed attestations",
            )?,
            attestations_processing_time_seconds: histogram(
                registry,
                "lean_state_transition_attestations_processing_time_seconds",
                "Time taken to process attestations",
                STAGE_BUCKETS,
            )?,
        })
    }

    /// Records one finalization attempt.
    pub fn record_finalization(&self, result: FinalizationResult) {
        self.finalizations_total
            .with_label_values(&[result.label()])
            .inc();
    }
}
