//! The leanMetrics contract, as a registry.
//!
//! [leanMetrics](https://github.com/leanEthereum/leanMetrics) fixes what every lean client
//! exposes: the metric name strings, their Prometheus types, their label names and enum
//! values, and their histogram buckets — so that one Grafana dashboard reads every client.
//! Transcribed from `metrics.md` at commit `69f97227`. What is governed is the registered
//! string, type, labels and buckets; the Rust identifiers below are free, and are named for
//! what they measure rather than for the string.
//!
//! # What is implemented
//!
//! Every metric the contract defines, grouped as the contract groups them: one struct per
//! category, each a field of [`Metrics`]. Counters whose label is a closed enum expose every
//! value at zero from the first scrape, so a dashboard reads the same series shape from this
//! client as from any other whether or not a value has occurred yet.
//!
//! # Where the samples come from
//!
//! This crate registers and renders; it does not observe. leanMetrics names a "sample
//! collection event" for each metric, and the crate that owns that event records into the
//! handle it holds — the validator client times its own signing, the verification stage its
//! own proof checks, the chain task its own imports. Gauges marked "on scrape" are refreshed
//! by the scrape endpoint itself, from the chain view current at that moment.

use std::fmt;

use prometheus::{Encoder, Registry, TextEncoder};

mod block_production;
mod fork_choice;
mod gossip_arrival;
mod labels;
mod network;
mod node_info;
mod register;
mod signature;
mod state_transition;
mod timing;
mod validator;

pub use block_production::BlockProductionMetrics;
pub use fork_choice::ForkChoiceMetrics;
pub use gossip_arrival::{ArrivalKind, GossipArrivalMetrics};
pub use labels::{
    ArrivalPosition, ConnectionResult, Direction, DisconnectReason, FinalizationResult, SkipReason,
    SyncStatus,
};
pub use network::{NetworkMetrics, PEER_CLIENT_UNKNOWN};
pub use node_info::NodeInfoMetrics;
pub use signature::SignatureMetrics;
pub use state_transition::StateTransitionMetrics;
pub use timing::{count_value, gauge_value, observe_duration, observe_elapsed};
pub use validator::ValidatorMetrics;

/// A registry that could not be built.
///
/// Only a name collision produces one, and the fixed names below make that impossible; it is
/// a type rather than a panic so that the contract's construction stays visible to callers.
#[derive(Debug)]
pub struct MetricsError(String);

impl fmt::Display for MetricsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot build the metric registry: {}", self.0)
    }
}

impl std::error::Error for MetricsError {}

impl From<prometheus::Error> for MetricsError {
    fn from(error: prometheus::Error) -> Self {
        Self(error.to_string())
    }
}

/// The concrete metric types, for a caller that names one in a signature.
pub mod prometheus_types {
    pub use prometheus::{Histogram, IntCounter, IntCounterVec, IntGauge, IntGaugeVec};
}

/// The Prometheus text exposition media type, as leanSpec serves it.
pub const TEXT_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Every registered metric, and the registry that renders them.
///
/// One per process, shared by handle. Fields are the contract's categories; recording into a
/// metric is a plain Prometheus operation on it.
#[derive(Debug)]
pub struct Metrics {
    registry: Registry,
    /// Node Info Metrics.
    pub node: NodeInfoMetrics,
    /// PQ Signature Metrics.
    pub signature: SignatureMetrics,
    /// Block Production Metrics.
    pub block: BlockProductionMetrics,
    /// Fork-Choice Metrics.
    pub fork_choice: ForkChoiceMetrics,
    /// State Transition Metrics.
    pub transition: StateTransitionMetrics,
    /// Validator Metrics.
    pub validator: ValidatorMetrics,
    /// Network Metrics.
    pub network: NetworkMetrics,
    /// Gossip Arrival Metrics.
    pub arrival: GossipArrivalMetrics,
}

impl Metrics {
    /// Registers every metric on a fresh registry.
    ///
    /// # Errors
    ///
    /// [`MetricsError`] only if two metrics collide, which the fixed names make impossible;
    /// it is propagated rather than unwrapped so the contract stays visible.
    pub fn new() -> Result<Self, MetricsError> {
        let registry = Registry::new();
        Ok(Self {
            node: NodeInfoMetrics::register(&registry)?,
            signature: SignatureMetrics::register(&registry)?,
            block: BlockProductionMetrics::register(&registry)?,
            fork_choice: ForkChoiceMetrics::register(&registry)?,
            transition: StateTransitionMetrics::register(&registry)?,
            validator: ValidatorMetrics::register(&registry)?,
            network: NetworkMetrics::register(&registry)?,
            arrival: GossipArrivalMetrics::register(&registry)?,
            registry,
        })
    }

    /// Records the node's identity and start time, once.
    pub fn record_start(&self, name: &str, version: &str, start_time_seconds: i64) {
        self.node.info.with_label_values(&[name, version]).set(1);
        self.node.start_time_seconds.set(start_time_seconds);
    }

    /// Sets the sync status: the named label to 1, every other to 0.
    pub fn set_sync_status(&self, status: SyncStatus) {
        self.fork_choice.set_sync_status(status);
    }

    /// Renders every metric in Prometheus text exposition format.
    #[must_use]
    pub fn render(&self) -> String {
        let mut buffer = Vec::new();
        // The text encoder writes into a `Vec`, which cannot fail; a metric family that
        // cannot be encoded is a bug in this crate, not a runtime condition to handle.
        if let Err(error) = TextEncoder::new().encode(&self.registry.gather(), &mut buffer) {
            return format!("# rendering failed: {error}\n");
        }
        String::from_utf8(buffer).unwrap_or_else(|_| "# rendering produced non-UTF-8\n".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row of leanMetrics `metrics.md` at commit `69f97227`: the name and the type.
    ///
    /// The list is the contract, transcribed; the test is that the registry exposes exactly
    /// these families, each once, under the right Prometheus type.
    const CONTRACT: &[(&str, &str)] = &[
        // Node Info
        ("lean_node_info", "gauge"),
        ("lean_node_start_time_seconds", "gauge"),
        // PQ Signature
        ("lean_pq_sig_attestation_signatures_total", "counter"),
        ("lean_pq_sig_attestation_signatures_valid_total", "counter"),
        (
            "lean_pq_sig_attestation_signatures_invalid_total",
            "counter",
        ),
        ("lean_pq_sig_attestation_signing_time_seconds", "histogram"),
        (
            "lean_pq_sig_attestation_verification_time_seconds",
            "histogram",
        ),
        ("lean_pq_sig_aggregated_signatures_total", "counter"),
        ("lean_pq_sig_aggregated_signatures_valid_total", "counter"),
        ("lean_pq_sig_aggregated_signatures_invalid_total", "counter"),
        (
            "lean_pq_sig_attestations_in_aggregated_signatures_total",
            "counter",
        ),
        (
            "lean_pq_sig_aggregated_signatures_building_time_seconds",
            "histogram",
        ),
        (
            "lean_pq_sig_aggregated_signatures_verification_time_seconds",
            "histogram",
        ),
        // Block Production
        ("lean_block_aggregated_payloads", "histogram"),
        (
            "lean_block_building_payload_aggregation_time_seconds",
            "histogram",
        ),
        ("lean_block_building_time_seconds", "histogram"),
        ("lean_block_building_success_total", "counter"),
        ("lean_block_building_failures_total", "counter"),
        // Fork-Choice
        ("lean_head_slot", "gauge"),
        ("lean_current_slot", "gauge"),
        ("lean_safe_target_slot", "gauge"),
        (
            "lean_fork_choice_block_processing_time_seconds",
            "histogram",
        ),
        ("lean_attestations_valid_total", "counter"),
        ("lean_attestations_invalid_total", "counter"),
        ("lean_attestation_validation_time_seconds", "histogram"),
        ("lean_fork_choice_reorgs_total", "counter"),
        ("lean_fork_choice_reorg_depth", "histogram"),
        ("lean_gossip_signatures", "gauge"),
        ("lean_latest_new_aggregated_payloads", "gauge"),
        ("lean_latest_known_aggregated_payloads", "gauge"),
        (
            "lean_committee_signatures_aggregation_time_seconds",
            "histogram",
        ),
        ("lean_node_sync_status", "gauge"),
        ("lean_tick_interval_duration_seconds", "histogram"),
        // State Transition
        ("lean_latest_justified_slot", "gauge"),
        ("lean_latest_finalized_slot", "gauge"),
        ("lean_justified_slot", "gauge"),
        ("lean_finalized_slot", "gauge"),
        ("lean_finalizations_total", "counter"),
        ("lean_state_transition_time_seconds", "histogram"),
        ("lean_state_transition_slots_processed_total", "counter"),
        (
            "lean_state_transition_slots_processing_time_seconds",
            "histogram",
        ),
        (
            "lean_state_transition_block_processing_time_seconds",
            "histogram",
        ),
        (
            "lean_state_transition_attestations_processed_total",
            "counter",
        ),
        (
            "lean_state_transition_attestations_processing_time_seconds",
            "histogram",
        ),
        // Validator
        ("lean_validators_count", "gauge"),
        ("lean_is_aggregator", "gauge"),
        ("lean_attestations_production_time_seconds", "histogram"),
        ("lean_aggregator_skipped_total", "counter"),
        // Network
        ("lean_connected_peers", "gauge"),
        ("lean_peer_connection_events_total", "counter"),
        ("lean_peer_disconnection_events_total", "counter"),
        ("lean_gossip_mesh_peers", "gauge"),
        ("lean_attestation_committee_subnet", "gauge"),
        ("lean_attestation_committee_count", "gauge"),
        ("lean_gossip_block_size_bytes", "histogram"),
        ("lean_gossip_attestation_size_bytes", "histogram"),
        ("lean_gossip_aggregation_size_bytes", "histogram"),
        // Gossip Arrival
        ("lean_gossip_block_arrival_delay_seconds", "histogram"),
        ("lean_gossip_attestation_arrival_delay_seconds", "histogram"),
        ("lean_gossip_aggregation_arrival_delay_seconds", "histogram"),
        ("lean_gossip_block_arrival_total", "counter"),
        ("lean_gossip_attestation_arrival_total", "counter"),
        ("lean_gossip_aggregation_arrival_total", "counter"),
    ];

    #[test]
    fn should_expose_every_contract_family_exactly_once_under_its_type() {
        let metrics = Metrics::new().expect("fresh registry");
        metrics.record_start("verity", "0.0.0", 1);
        let text = metrics.render();

        assert_eq!(
            CONTRACT.len(),
            63,
            "leanMetrics 69f97227 defines 63 metrics"
        );
        for (name, kind) in CONTRACT {
            let header = format!("# TYPE {name} {kind}\n");
            assert_eq!(
                text.matches(&header).count(),
                1,
                "expected `{}` once in:\n{text}",
                header.trim_end()
            );
        }

        let families = text
            .lines()
            .filter(|line| line.starts_with("# TYPE "))
            .count();
        assert_eq!(families, CONTRACT.len(), "no family outside the contract");
    }

    #[test]
    fn should_render_every_enum_label_series_from_the_first_scrape() {
        let metrics = Metrics::new().expect("fresh registry");
        let text = metrics.render();

        for line in [
            "lean_finalizations_total{result=\"success\"} 0",
            "lean_finalizations_total{result=\"error\"} 0",
            "lean_aggregator_skipped_total{reason=\"not_synced\"} 0",
            "lean_peer_connection_events_total{direction=\"inbound\",result=\"timeout\"} 0",
            "lean_peer_disconnection_events_total{direction=\"outbound\",reason=\"local_close\"} 0",
            "lean_gossip_block_arrival_total{position=\"before\"} 0",
            "lean_gossip_aggregation_arrival_total{position=\"inside\"} 0",
            "lean_connected_peers{client=\"unknown\"} 0",
            "lean_gossip_mesh_peers{client=\"unknown\"} 0",
        ] {
            assert!(text.contains(line), "missing `{line}` in:\n{text}");
        }
        assert!(
            !text.contains("lean_gossip_aggregation_arrival_total{position=\"before\"}"),
            "an aggregate can never arrive before a boundary at or before it"
        );
    }

    #[test]
    fn should_move_the_sync_status_to_exactly_one_label() {
        let metrics = Metrics::new().expect("fresh registry");
        metrics.set_sync_status(SyncStatus::Syncing);
        metrics.set_sync_status(SyncStatus::Idle);
        let text = metrics.render();
        assert!(text.contains("lean_node_sync_status{status=\"idle\"} 1"));
        assert!(text.contains("lean_node_sync_status{status=\"syncing\"} 0"));
        assert!(text.contains("lean_node_sync_status{status=\"synced\"} 0"));
    }

    #[test]
    fn should_record_arrivals_against_the_channel_they_came_on() {
        let metrics = Metrics::new().expect("fresh registry");
        metrics
            .arrival
            .record(ArrivalKind::Attestation, 0.3, ArrivalPosition::After);
        let text = metrics.render();
        assert!(text.contains("lean_gossip_attestation_arrival_total{position=\"after\"} 1"));
        assert!(text.contains("lean_gossip_attestation_arrival_delay_seconds_count 1"));
        assert!(text.contains("lean_gossip_block_arrival_delay_seconds_count 0"));
    }

    #[test]
    fn should_declare_a_second_registry_independently() {
        // Two nodes in one test process must not share a registry.
        let a = Metrics::new().expect("first");
        let b = Metrics::new().expect("second");
        a.fork_choice.head_slot.set(7);
        assert!(b.render().contains("lean_head_slot 0"));
    }
}
