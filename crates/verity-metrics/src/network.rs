//! Network Metrics: peers, the gossip mesh, the committee layout, and message sizes.

use prometheus::{Histogram, IntCounterVec, IntGauge, IntGaugeVec, Registry};

use crate::labels::{ConnectionResult, Direction, DisconnectReason};
use crate::register::{counter_vec, gauge, gauge_vec, histogram, seed_pairs};

/// A gossiped block, in bytes: a proof-bearing block is hundreds of kilobytes.
const BLOCK_SIZE_BUCKETS: &[f64] = &[
    10_000.0,
    50_000.0,
    100_000.0,
    250_000.0,
    500_000.0,
    1_000_000.0,
    2_000_000.0,
    5_000_000.0,
];

/// A gossiped vote, in bytes: one XMSS signature plus its data.
const ATTESTATION_SIZE_BUCKETS: &[f64] = &[512.0, 1024.0, 2048.0, 4096.0, 8192.0, 16384.0];

/// A gossiped aggregate, in bytes.
const AGGREGATION_SIZE_BUCKETS: &[f64] = &[
    1024.0,
    4096.0,
    16384.0,
    65536.0,
    131_072.0,
    262_144.0,
    524_288.0,
    1_048_576.0,
];

/// The `client` label value for a peer whose client is not announced.
///
/// The lean transport carries no client name, so every peer is `unknown`; the label exists
/// because the contract fixes it, and a dashboard that groups by it still sums correctly.
pub const PEER_CLIENT_UNKNOWN: &str = "unknown";

/// Gauges, counters and sizes of the network.
#[derive(Debug)]
pub struct NetworkMetrics {
    /// `lean_connected_peers`, labelled `client` (`<name>_<N>` or `unknown`).
    pub connected_peers: IntGaugeVec,
    /// `lean_peer_connection_events_total`, labelled `direction`, `result`.
    pub peer_connection_events_total: IntCounterVec,
    /// `lean_peer_disconnection_events_total`, labelled `direction`, `reason`.
    pub peer_disconnection_events_total: IntCounterVec,
    /// `lean_gossip_mesh_peers`, labelled `client`.
    pub gossip_mesh_peers: IntGaugeVec,
    /// `lean_attestation_committee_subnet`: the subnet this node's votes go on.
    pub attestation_committee_subnet: IntGauge,
    /// `lean_attestation_committee_count`: `ATTESTATION_COMMITTEE_COUNT`.
    pub attestation_committee_count: IntGauge,
    /// `lean_gossip_block_size_bytes`.
    pub gossip_block_size_bytes: Histogram,
    /// `lean_gossip_attestation_size_bytes`.
    pub gossip_attestation_size_bytes: Histogram,
    /// `lean_gossip_aggregation_size_bytes`.
    pub gossip_aggregation_size_bytes: Histogram,
}

impl NetworkMetrics {
    pub(crate) fn register(registry: &Registry) -> prometheus::Result<Self> {
        let peer_connection_events_total = counter_vec(
            registry,
            "lean_peer_connection_events_total",
            "Total number of peer connection events",
            &["direction", "result"],
        )?;
        seed_pairs(
            &peer_connection_events_total,
            &Direction::labels(),
            &ConnectionResult::labels(),
        );
        let peer_disconnection_events_total = counter_vec(
            registry,
            "lean_peer_disconnection_events_total",
            "Total number of peer disconnection events",
            &["direction", "reason"],
        )?;
        seed_pairs(
            &peer_disconnection_events_total,
            &Direction::labels(),
            &DisconnectReason::labels(),
        );

        let metrics = Self {
            connected_peers: gauge_vec(
                registry,
                "lean_connected_peers",
                "Number of connected peers",
                &["client"],
            )?,
            peer_connection_events_total,
            peer_disconnection_events_total,
            gossip_mesh_peers: gauge_vec(
                registry,
                "lean_gossip_mesh_peers",
                "Number of peers in the gossipsub mesh",
                &["client"],
            )?,
            attestation_committee_subnet: gauge(
                registry,
                "lean_attestation_committee_subnet",
                "Node's attestation committee subnet",
            )?,
            attestation_committee_count: gauge(
                registry,
                "lean_attestation_committee_count",
                "Number of attestation committees (ATTESTATION_COMMITTEE_COUNT)",
            )?,
            gossip_block_size_bytes: histogram(
                registry,
                "lean_gossip_block_size_bytes",
                "Bytes size of a gossip block message",
                BLOCK_SIZE_BUCKETS,
            )?,
            gossip_attestation_size_bytes: histogram(
                registry,
                "lean_gossip_attestation_size_bytes",
                "Bytes size of a gossip attestation message",
                ATTESTATION_SIZE_BUCKETS,
            )?,
            gossip_aggregation_size_bytes: histogram(
                registry,
                "lean_gossip_aggregation_size_bytes",
                "Bytes size of a gossip aggregated attestation message",
                AGGREGATION_SIZE_BUCKETS,
            )?,
        };
        // Both gauges are exposed from the first scrape, at zero, under the one client label
        // this node can name.
        metrics.connected_peers().set(0);
        metrics.mesh_peers().set(0);
        Ok(metrics)
    }

    /// The `lean_connected_peers` series for peers of unannounced client.
    #[must_use]
    pub fn connected_peers(&self) -> IntGauge {
        self.connected_peers
            .with_label_values(&[PEER_CLIENT_UNKNOWN])
    }

    /// The `lean_gossip_mesh_peers` series for peers of unannounced client.
    #[must_use]
    pub fn mesh_peers(&self) -> IntGauge {
        self.gossip_mesh_peers
            .with_label_values(&[PEER_CLIENT_UNKNOWN])
    }

    /// Records one connection attempt's outcome.
    pub fn record_connection(&self, direction: Direction, result: ConnectionResult) {
        self.peer_connection_events_total
            .with_label_values(&[direction.label(), result.label()])
            .inc();
    }

    /// Records one disconnection and why it happened.
    pub fn record_disconnection(&self, direction: Direction, reason: DisconnectReason) {
        self.peer_disconnection_events_total
            .with_label_values(&[direction.label(), reason.label()])
            .inc();
    }
}
