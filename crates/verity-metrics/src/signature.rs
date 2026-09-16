//! PQ Signature Metrics: XMSS signing and verification, and proof aggregation.

use prometheus::{Histogram, IntCounter, Registry};

use crate::register::{counter, histogram};

/// Buckets for a single XMSS operation, in seconds.
const XMSS_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 1.0];

/// Buckets for building or verifying an aggregate proof, in seconds.
const AGGREGATE_BUCKETS: &[f64] = &[0.1, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0];

/// Counters and timings of the signature scheme.
#[derive(Debug)]
pub struct SignatureMetrics {
    /// `lean_pq_sig_attestation_signatures_total`: votes this node signed.
    pub attestation_signatures_total: IntCounter,
    /// `lean_pq_sig_attestation_signatures_valid_total`: per-validator signatures that held.
    pub attestation_signatures_valid_total: IntCounter,
    /// `lean_pq_sig_attestation_signatures_invalid_total`: per-validator signatures that did not.
    pub attestation_signatures_invalid_total: IntCounter,
    /// `lean_pq_sig_attestation_signing_time_seconds`.
    pub attestation_signing_time_seconds: Histogram,
    /// `lean_pq_sig_attestation_verification_time_seconds`.
    pub attestation_verification_time_seconds: Histogram,
    /// `lean_pq_sig_aggregated_signatures_total`: proofs this node built.
    pub aggregated_signatures_total: IntCounter,
    /// `lean_pq_sig_aggregated_signatures_valid_total`: proofs that verified.
    pub aggregated_signatures_valid_total: IntCounter,
    /// `lean_pq_sig_aggregated_signatures_invalid_total`: proofs that did not.
    pub aggregated_signatures_invalid_total: IntCounter,
    /// `lean_pq_sig_attestations_in_aggregated_signatures_total`: votes folded into proofs.
    pub attestations_in_aggregated_signatures_total: IntCounter,
    /// `lean_pq_sig_aggregated_signatures_building_time_seconds`.
    pub aggregated_signatures_building_time_seconds: Histogram,
    /// `lean_pq_sig_aggregated_signatures_verification_time_seconds`.
    pub aggregated_signatures_verification_time_seconds: Histogram,
}

impl SignatureMetrics {
    pub(crate) fn register(registry: &Registry) -> prometheus::Result<Self> {
        Ok(Self {
            attestation_signatures_total: counter(
                registry,
                "lean_pq_sig_attestation_signatures_total",
                "Total number of individual attestation signatures",
            )?,
            attestation_signatures_valid_total: counter(
                registry,
                "lean_pq_sig_attestation_signatures_valid_total",
                "Total number of valid individual attestation signatures",
            )?,
            attestation_signatures_invalid_total: counter(
                registry,
                "lean_pq_sig_attestation_signatures_invalid_total",
                "Total number of invalid individual attestation signatures",
            )?,
            attestation_signing_time_seconds: histogram(
                registry,
                "lean_pq_sig_attestation_signing_time_seconds",
                "Time taken to sign an attestation",
                XMSS_BUCKETS,
            )?,
            attestation_verification_time_seconds: histogram(
                registry,
                "lean_pq_sig_attestation_verification_time_seconds",
                "Time taken to verify an attestation signature",
                XMSS_BUCKETS,
            )?,
            aggregated_signatures_total: counter(
                registry,
                "lean_pq_sig_aggregated_signatures_total",
                "Total number of aggregated signatures",
            )?,
            aggregated_signatures_valid_total: counter(
                registry,
                "lean_pq_sig_aggregated_signatures_valid_total",
                "Total number of valid aggregated signatures",
            )?,
            aggregated_signatures_invalid_total: counter(
                registry,
                "lean_pq_sig_aggregated_signatures_invalid_total",
                "Total number of invalid aggregated signatures",
            )?,
            attestations_in_aggregated_signatures_total: counter(
                registry,
                "lean_pq_sig_attestations_in_aggregated_signatures_total",
                "Total number of attestations included into aggregated signatures",
            )?,
            aggregated_signatures_building_time_seconds: histogram(
                registry,
                "lean_pq_sig_aggregated_signatures_building_time_seconds",
                "Time taken to build an aggregated attestation signature",
                AGGREGATE_BUCKETS,
            )?,
            aggregated_signatures_verification_time_seconds: histogram(
                registry,
                "lean_pq_sig_aggregated_signatures_verification_time_seconds",
                "Time taken to verify an aggregated attestation signature",
                AGGREGATE_BUCKETS,
            )?,
        })
    }

    /// Records one proof built, and the votes it covers.
    pub fn record_aggregate_built(&self, participants: usize) {
        self.aggregated_signatures_total.inc();
        self.attestations_in_aggregated_signatures_total
            .inc_by(participants as u64);
    }
}
