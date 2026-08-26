//! Dropping the votes finalization has put out of reach.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/fork_choice.py`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`.

use std::collections::HashSet;

use verity_types::AttestationData;

use crate::fork_choice::store::Store;

/// Drops every vote whose head can no longer influence fork choice.
///
/// A vote is out of reach when its head sits at or below the finalized slot, or when that
/// head is on a branch the finalized block orphaned. Neither head lies under the finalized
/// block, and fork choice only ever descends from there, so neither can credit a block the
/// walk reaches — dropping them cannot change the chosen chain.
///
/// This is sound only because [`super::block::update_head`] re-derives the finalized
/// checkpoint from the head. Pruning against a checkpoint that had drifted onto a different
/// branch would discard votes that still matter.
///
/// All three pools share the vote as their key, so one staleness test filters them together.
pub fn prune_stale_attestation_data(store: &mut Store) {
    let finalized = store.latest_finalized;

    let survivors: HashSet<AttestationData> = store
        .attestation_signatures
        .keys()
        .chain(store.latest_new_aggregated_payloads.keys())
        .chain(store.latest_known_aggregated_payloads.keys())
        .filter(|data| {
            data.head.slot.0 > finalized.slot.0 && store.is_ancestor(finalized, data.head)
        })
        .copied()
        .collect();

    store
        .attestation_signatures
        .retain(|data, _| survivors.contains(data));
    store
        .latest_new_aggregated_payloads
        .retain(|data, _| survivors.contains(data));
    store
        .latest_known_aggregated_payloads
        .retain(|data, _| survivors.contains(data));
}
