//! Node Info Metrics: who this process is, and when it started.

use prometheus::{IntGauge, IntGaugeVec, Registry};

use crate::register::{gauge, gauge_vec};

/// The two "on node start" facts.
#[derive(Debug)]
pub struct NodeInfoMetrics {
    /// `lean_node_info`, labelled `name`, `version`. Always 1.
    pub info: IntGaugeVec,
    /// `lean_node_start_time_seconds`: the Unix time the node started.
    pub start_time_seconds: IntGauge,
}

impl NodeInfoMetrics {
    pub(crate) fn register(registry: &Registry) -> prometheus::Result<Self> {
        Ok(Self {
            info: gauge_vec(
                registry,
                "lean_node_info",
                "Node information (always 1)",
                &["name", "version"],
            )?,
            start_time_seconds: gauge(registry, "lean_node_start_time_seconds", "Start timestamp")?,
        })
    }
}
