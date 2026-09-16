//! Registration helpers: one call per metric family, each on the shared registry.
//!
//! The helpers exist so that every category module reads as a table of the contract — name,
//! help, labels, buckets — with no registry plumbing between the rows.

use prometheus::{
    Histogram, HistogramOpts, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
};

pub(crate) fn gauge(registry: &Registry, name: &str, help: &str) -> prometheus::Result<IntGauge> {
    let gauge = IntGauge::with_opts(Opts::new(name, help))?;
    registry.register(Box::new(gauge.clone()))?;
    Ok(gauge)
}

pub(crate) fn gauge_vec(
    registry: &Registry,
    name: &str,
    help: &str,
    labels: &[&str],
) -> prometheus::Result<IntGaugeVec> {
    let gauge = IntGaugeVec::new(Opts::new(name, help), labels)?;
    registry.register(Box::new(gauge.clone()))?;
    Ok(gauge)
}

pub(crate) fn counter(
    registry: &Registry,
    name: &str,
    help: &str,
) -> prometheus::Result<IntCounter> {
    let counter = IntCounter::with_opts(Opts::new(name, help))?;
    registry.register(Box::new(counter.clone()))?;
    Ok(counter)
}

pub(crate) fn counter_vec(
    registry: &Registry,
    name: &str,
    help: &str,
    labels: &[&str],
) -> prometheus::Result<IntCounterVec> {
    let counter = IntCounterVec::new(Opts::new(name, help), labels)?;
    registry.register(Box::new(counter.clone()))?;
    Ok(counter)
}

/// A histogram with the exact bucket boundaries leanMetrics fixes for it.
pub(crate) fn histogram(
    registry: &Registry,
    name: &str,
    help: &str,
    buckets: &[f64],
) -> prometheus::Result<Histogram> {
    let histogram = Histogram::with_opts(HistogramOpts::new(name, help).buckets(buckets.to_vec()))?;
    registry.register(Box::new(histogram.clone()))?;
    Ok(histogram)
}

/// Creates every series of a one-label counter at zero.
///
/// A counter whose label is a closed enum should expose every value from the first scrape:
/// a dashboard summing `rate()` over the enum then reads the same shape from every client,
/// and an absent series is otherwise indistinguishable from a client that lacks the metric.
pub(crate) fn seed(counter: &IntCounterVec, values: &[&str]) {
    for value in values {
        counter.with_label_values(&[value]).reset();
    }
}

/// Creates every series of a two-label counter at zero, over the product of the two enums.
pub(crate) fn seed_pairs(counter: &IntCounterVec, first: &[&str], second: &[&str]) {
    for a in first {
        for b in second {
            counter.with_label_values(&[a, b]).reset();
        }
    }
}
