//! Block Production Metrics: what a proposal costs, and whether it shipped.

use prometheus::{Histogram, IntCounter, Registry};

use crate::register::{counter, histogram};

/// Distinct aggregated payloads in a block body.
const PAYLOAD_COUNT_BUCKETS: &[f64] = &[1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];

/// Folding the body's proofs, in seconds.
const PAYLOAD_AGGREGATION_BUCKETS: &[f64] = &[0.1, 0.25, 0.5, 0.75, 1.0, 2.0, 3.0, 4.0];

/// Selecting and assembling the block, in seconds.
const BUILDING_BUCKETS: &[f64] = &[0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0];

/// Counters and timings of block production.
#[derive(Debug)]
pub struct BlockProductionMetrics {
    /// `lean_block_aggregated_payloads`: aggregated payloads in a produced block.
    pub aggregated_payloads: Histogram,
    /// `lean_block_building_payload_aggregation_time_seconds`.
    pub building_payload_aggregation_time_seconds: Histogram,
    /// `lean_block_building_time_seconds`.
    pub building_time_seconds: Histogram,
    /// `lean_block_building_success_total`.
    pub building_success_total: IntCounter,
    /// `lean_block_building_failures_total`.
    pub building_failures_total: IntCounter,
}

impl BlockProductionMetrics {
    pub(crate) fn register(registry: &Registry) -> prometheus::Result<Self> {
        Ok(Self {
            aggregated_payloads: histogram(
                registry,
                "lean_block_aggregated_payloads",
                "Number of aggregated_payloads in a block",
                PAYLOAD_COUNT_BUCKETS,
            )?,
            building_payload_aggregation_time_seconds: histogram(
                registry,
                "lean_block_building_payload_aggregation_time_seconds",
                "Time taken to build aggregated_payloads during block building",
                PAYLOAD_AGGREGATION_BUCKETS,
            )?,
            building_time_seconds: histogram(
                registry,
                "lean_block_building_time_seconds",
                "Time taken to build a block",
                BUILDING_BUCKETS,
            )?,
            building_success_total: counter(
                registry,
                "lean_block_building_success_total",
                "Successful block builds",
            )?,
            building_failures_total: counter(
                registry,
                "lean_block_building_failures_total",
                "Failed block builds (exception in build_block)",
            )?,
        })
    }
}
