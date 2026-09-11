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
//! The gauges and counters a node can sample from its chain view and its peer set: node
//! info, the fork-choice and state-transition slots, the sync status, the validator and
//! aggregator facts, the peer count, and the committee layout. The histograms — timings of
//! signing, verification, block building, state transition — are not registered yet: each
//! needs a probe at the point of work, and an unregistered histogram is a gap a dashboard
//! shows as "no data", where a wrongly placed probe would show a wrong number.
//!
//! # Where the samples come from
//!
//! This crate registers and renders; it does not observe. leanMetrics names a "sample
//! collection event" for each metric, and the crate that owns that event records into the
//! handle it holds. Gauges marked "on scrape" are refreshed by the scrape endpoint itself,
//! from the chain view current at that moment.

use std::fmt;

use prometheus::{Encoder, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder};

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

/// The concrete gauge and counter types, for a caller that names one in a signature.
pub mod prometheus_gauge {
    pub use prometheus::{IntCounter, IntGauge};
}

/// The Prometheus text exposition media type, as leanSpec serves it.
pub const TEXT_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// The `status` label of `lean_node_sync_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    /// Not syncing and not synced: the node has not met the network yet.
    Idle,
    /// Behind the network and fetching.
    Syncing,
    /// Caught up.
    Synced,
}

impl SyncStatus {
    const ALL: [Self; 3] = [Self::Idle, Self::Syncing, Self::Synced];

    /// The label value leanMetrics fixes for this status.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Syncing => "syncing",
            Self::Synced => "synced",
        }
    }
}

/// Every registered metric, and the registry that renders them.
///
/// One per process, shared by handle. Fields are the metric families; recording into one
/// is a plain Prometheus operation on it.
#[derive(Debug)]
pub struct Metrics {
    registry: Registry,

    /// `lean_node_info`, labelled `name`, `version`. Always 1.
    pub node_info: IntGaugeVec,
    /// `lean_node_start_time_seconds`: the Unix time the node started.
    pub node_start_time_seconds: IntGauge,

    /// `lean_head_slot`: the slot of the fork-choice head.
    pub head_slot: IntGauge,
    /// `lean_current_slot`: the slot the clock sits in.
    pub current_slot: IntGauge,
    /// `lean_safe_target_slot`: the slot of the safe target.
    pub safe_target_slot: IntGauge,
    /// `lean_gossip_signatures`: per-validator signatures held in the store.
    pub gossip_signatures: IntGauge,
    /// `lean_latest_new_aggregated_payloads`: proofs gathered this slot, not yet counted.
    pub latest_new_aggregated_payloads: IntGauge,
    /// `lean_latest_known_aggregated_payloads`: proofs counting toward weight.
    pub latest_known_aggregated_payloads: IntGauge,
    /// `lean_node_sync_status`, labelled `status`. Exactly one label value is 1.
    pub node_sync_status: IntGaugeVec,

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

    /// `lean_validators_count`: validators this node runs.
    pub validators_count: IntGauge,
    /// `lean_is_aggregator`: 1 when this node aggregates.
    pub is_aggregator: IntGauge,

    /// `lean_connected_peers`, labelled `client` (`<name>_<N>` or `unknown`).
    pub connected_peers: IntGaugeVec,
    /// `lean_attestation_committee_subnet`: the subnet this node's votes go on.
    pub attestation_committee_subnet: IntGauge,
    /// `lean_attestation_committee_count`: `ATTESTATION_COMMITTEE_COUNT`.
    pub attestation_committee_count: IntGauge,
}

impl Metrics {
    /// Registers every metric on a fresh registry.
    ///
    /// # Errors
    ///
    /// [`MetricsError`] only if two metrics collide, which the fixed names below make
    /// impossible; it is propagated rather than unwrapped so the contract stays visible.
    pub fn new() -> Result<Self, MetricsError> {
        let registry = Registry::new();
        let metrics = Self {
            node_info: gauge_vec(
                &registry,
                "lean_node_info",
                "Node information (always 1)",
                &["name", "version"],
            )?,
            node_start_time_seconds: gauge(
                &registry,
                "lean_node_start_time_seconds",
                "Start timestamp",
            )?,
            head_slot: gauge(&registry, "lean_head_slot", "Latest slot of the lean chain")?,
            current_slot: gauge(
                &registry,
                "lean_current_slot",
                "Current slot of the lean chain",
            )?,
            safe_target_slot: gauge(&registry, "lean_safe_target_slot", "Safe target slot")?,
            gossip_signatures: gauge(
                &registry,
                "lean_gossip_signatures",
                "Number of gossip signatures in fork-choice store",
            )?,
            latest_new_aggregated_payloads: gauge(
                &registry,
                "lean_latest_new_aggregated_payloads",
                "Number of new aggregated payload items",
            )?,
            latest_known_aggregated_payloads: gauge(
                &registry,
                "lean_latest_known_aggregated_payloads",
                "Number of known aggregated payload items",
            )?,
            node_sync_status: gauge_vec(
                &registry,
                "lean_node_sync_status",
                "Node sync status",
                &["status"],
            )?,
            latest_justified_slot: gauge(
                &registry,
                "lean_latest_justified_slot",
                "Latest justified slot",
            )?,
            latest_finalized_slot: gauge(
                &registry,
                "lean_latest_finalized_slot",
                "Latest finalized slot",
            )?,
            justified_slot: gauge(&registry, "lean_justified_slot", "Current justified slot")?,
            finalized_slot: gauge(&registry, "lean_finalized_slot", "Current finalized slot")?,
            finalizations_total: counter_vec(
                &registry,
                "lean_finalizations_total",
                "Total number of finalization attempts",
                &["result"],
            )?,
            validators_count: gauge(
                &registry,
                "lean_validators_count",
                "Number of validators managed by a node",
            )?,
            is_aggregator: gauge(
                &registry,
                "lean_is_aggregator",
                "Validator's is_aggregator status. True=1, False=0",
            )?,
            connected_peers: gauge_vec(
                &registry,
                "lean_connected_peers",
                "Number of connected peers",
                &["client"],
            )?,
            attestation_committee_subnet: gauge(
                &registry,
                "lean_attestation_committee_subnet",
                "Node's attestation committee subnet",
            )?,
            attestation_committee_count: gauge(
                &registry,
                "lean_attestation_committee_count",
                "Number of attestation committees (ATTESTATION_COMMITTEE_COUNT)",
            )?,
            registry,
        };
        Ok(metrics)
    }

    /// Records the node's identity and start time, once.
    pub fn record_start(&self, name: &str, version: &str, start_time_seconds: i64) {
        self.node_info.with_label_values(&[name, version]).set(1);
        self.node_start_time_seconds.set(start_time_seconds);
    }

    /// Sets the sync status: the named label to 1, every other to 0.
    ///
    /// All three series are always present, so a dashboard's `max by (status)` reads the
    /// same shape from every client whether or not a status has ever been active.
    pub fn set_sync_status(&self, status: SyncStatus) {
        for candidate in SyncStatus::ALL {
            let value = i64::from(candidate == status);
            self.node_sync_status
                .with_label_values(&[candidate.label()])
                .set(value);
        }
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

fn gauge(registry: &Registry, name: &str, help: &str) -> Result<IntGauge, prometheus::Error> {
    let gauge = IntGauge::with_opts(Opts::new(name, help))?;
    registry.register(Box::new(gauge.clone()))?;
    Ok(gauge)
}

fn gauge_vec(
    registry: &Registry,
    name: &str,
    help: &str,
    labels: &[&str],
) -> Result<IntGaugeVec, prometheus::Error> {
    let gauge = IntGaugeVec::new(Opts::new(name, help), labels)?;
    registry.register(Box::new(gauge.clone()))?;
    Ok(gauge)
}

fn counter_vec(
    registry: &Registry,
    name: &str,
    help: &str,
    labels: &[&str],
) -> Result<IntCounterVec, prometheus::Error> {
    let counter = IntCounterVec::new(Opts::new(name, help), labels)?;
    registry.register(Box::new(counter.clone()))?;
    Ok(counter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_register_every_contract_name_exactly_once() {
        let metrics = Metrics::new().expect("fresh registry");
        metrics.record_start("verity", "0.0.0", 1);
        metrics.set_sync_status(SyncStatus::Synced);
        metrics
            .finalizations_total
            .with_label_values(&["success"])
            .inc();
        metrics
            .connected_peers
            .with_label_values(&["unknown"])
            .set(3);
        let text = metrics.render();

        for name in [
            "lean_node_info{name=\"verity\",version=\"0.0.0\"} 1",
            "lean_node_start_time_seconds 1",
            "lean_head_slot 0",
            "lean_current_slot 0",
            "lean_safe_target_slot 0",
            "lean_gossip_signatures 0",
            "lean_latest_new_aggregated_payloads 0",
            "lean_latest_known_aggregated_payloads 0",
            "lean_node_sync_status{status=\"idle\"} 0",
            "lean_node_sync_status{status=\"syncing\"} 0",
            "lean_node_sync_status{status=\"synced\"} 1",
            "lean_latest_justified_slot 0",
            "lean_latest_finalized_slot 0",
            "lean_justified_slot 0",
            "lean_finalized_slot 0",
            "lean_finalizations_total{result=\"success\"} 1",
            "lean_validators_count 0",
            "lean_is_aggregator 0",
            "lean_connected_peers{client=\"unknown\"} 3",
            "lean_attestation_committee_subnet 0",
            "lean_attestation_committee_count 0",
        ] {
            assert!(text.contains(name), "missing `{name}` in:\n{text}");
        }
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
    fn should_declare_a_second_registry_independently() {
        // Two nodes in one test process must not share a registry.
        let a = Metrics::new().expect("first");
        let b = Metrics::new().expect("second");
        a.head_slot.set(7);
        assert!(b.render().contains("lean_head_slot 0"));
    }
}
