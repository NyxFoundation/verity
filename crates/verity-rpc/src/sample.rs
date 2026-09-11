//! The metrics whose leanMetrics collection event is "on scrape", or that the view already
//! answers for: refreshed from the current snapshot each time `/metrics` is read.
//!
//! Reading the snapshot at scrape time is what keeps the chain task out of the metrics path
//! entirely — it publishes views; it does not know a scrape happened.

use verity_chain::ChainView;
use verity_metrics::{Metrics, SyncStatus};

use crate::ApiContext;

/// Refreshes every view-derived gauge from the snapshot current now.
pub fn on_scrape(context: &ApiContext) {
    let view = context.view.borrow().clone();
    let synced = *context.synced.borrow();
    sample_view(&context.metrics, &view);
    context.metrics.set_sync_status(if synced {
        SyncStatus::Synced
    } else {
        SyncStatus::Syncing
    });
}

fn sample_view(metrics: &Metrics, view: &ChainView) {
    let justified = view.latest_justified().slot.0;
    let finalized = view.latest_finalized().slot.0;
    let safe_target_slot = view
        .block(view.safe_target())
        .map_or(0, |block| block.slot.0);

    metrics
        .head_slot
        .set(gauge_value(view.head_checkpoint().slot.0));
    metrics.current_slot.set(gauge_value(view.slot().0));
    metrics.safe_target_slot.set(gauge_value(safe_target_slot));
    metrics.latest_justified_slot.set(gauge_value(justified));
    metrics.justified_slot.set(gauge_value(justified));
    metrics.latest_finalized_slot.set(gauge_value(finalized));
    metrics.finalized_slot.set(gauge_value(finalized));
    metrics
        .gossip_signatures
        .set(count_value(view.attestation_signature_count()));
    metrics
        .latest_new_aggregated_payloads
        .set(count_value(view.new_aggregated_payload_count()));
    metrics
        .latest_known_aggregated_payloads
        .set(count_value(view.known_aggregated_payload_count()));
}

/// A slot as a gauge value. Slots never approach `i64::MAX`; saturating keeps the cast
/// honest without a panic path.
fn gauge_value(slot: u64) -> i64 {
    i64::try_from(slot).unwrap_or(i64::MAX)
}

fn count_value(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}
