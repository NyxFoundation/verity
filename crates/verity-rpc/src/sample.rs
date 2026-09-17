//! The metrics whose leanMetrics collection event is "on scrape", or that the view already
//! answers for: refreshed from the current snapshot each time `/metrics` is read.
//!
//! Reading the snapshot at scrape time is what keeps the chain task out of the metrics path
//! entirely — it publishes views; it does not know a scrape happened.

use verity_chain::ChainView;
use verity_metrics::{Metrics, SyncStatus, count_value, gauge_value};

use crate::ApiContext;

/// Refreshes every view-derived gauge from the snapshot current now, then runs the node's
/// own samplers.
pub fn on_scrape(context: &ApiContext) {
    let view = context.view.borrow().clone();
    let synced = *context.synced.borrow();
    sample_view(&context.metrics, &view);
    context.metrics.set_sync_status(if synced {
        SyncStatus::Synced
    } else {
        SyncStatus::Syncing
    });
    for sampler in &context.samplers {
        sampler(&context.metrics);
    }
}

fn sample_view(metrics: &Metrics, view: &ChainView) {
    let justified = view.latest_justified().slot.0;
    let finalized = view.latest_finalized().slot.0;
    let safe_target_slot = view
        .block(view.safe_target())
        .map_or(0, |block| block.slot.0);

    let fork_choice = &metrics.fork_choice;
    fork_choice
        .head_slot
        .set(gauge_value(view.head_checkpoint().slot.0));
    fork_choice.current_slot.set(gauge_value(view.slot().0));
    fork_choice
        .safe_target_slot
        .set(gauge_value(safe_target_slot));
    fork_choice
        .gossip_signatures
        .set(count_value(view.attestation_signature_count()));
    fork_choice
        .latest_new_aggregated_payloads
        .set(count_value(view.new_aggregated_payload_count()));
    fork_choice
        .latest_known_aggregated_payloads
        .set(count_value(view.known_aggregated_payload_count()));

    let transition = &metrics.transition;
    transition.latest_justified_slot.set(gauge_value(justified));
    transition.justified_slot.set(gauge_value(justified));
    transition.latest_finalized_slot.set(gauge_value(finalized));
    transition.finalized_slot.set(gauge_value(finalized));
}
