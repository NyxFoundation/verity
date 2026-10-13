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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use verity_types::{
        AggregationBits, AttestationData, Checkpoint, SingleMessageAggregate, Slot, ValidatorIndex,
    };

    use crate::fork_choice::testing::{anchored_on_genesis, import_at};
    use crate::fork_choice::{AttestationSignature, AttestationSignatureEntry};
    use crate::merkle::hash_tree_root;

    use super::prune_stale_attestation_data;

    fn vote(source: Checkpoint, target: Checkpoint, head: Checkpoint) -> AttestationData {
        AttestationData {
            slot: head.slot,
            source,
            target,
            head,
        }
    }

    #[test]
    fn should_remove_finalized_and_orphaned_votes_from_every_pool() {
        let (mut store, genesis) = anchored_on_genesis(2);
        let anchor = store.latest_finalized;
        let first_block = import_at(&mut store, &genesis, 1);
        let first = Checkpoint {
            root: hash_tree_root(&first_block),
            slot: Slot(1),
        };
        let first_state = store.states[&first.root].clone();
        let canonical_block = import_at(&mut store, &first_state, 2);
        let canonical = Checkpoint {
            root: hash_tree_root(&canonical_block),
            slot: Slot(2),
        };
        let sibling_block = import_at(&mut store, &genesis, 3);
        let sibling = Checkpoint {
            root: hash_tree_root(&sibling_block),
            slot: Slot(3),
        };

        store.latest_finalized = first;
        let finalized = vote(anchor, first, first);
        let orphaned = vote(anchor, anchor, sibling);
        let survivor = vote(anchor, first, canonical);
        let signature = HashSet::from([AttestationSignatureEntry {
            validator_index: ValidatorIndex(0),
            signature: AttestationSignature(vec![1]),
        }]);
        let proof = HashSet::from([SingleMessageAggregate {
            participants: AggregationBits::try_from(vec![true, false]).expect("small bitfield"),
            ..SingleMessageAggregate::default()
        }]);

        for data in [finalized, orphaned, survivor] {
            store.attestation_signatures.insert(data, signature.clone());
            store
                .latest_new_aggregated_payloads
                .insert(data, proof.clone());
            store
                .latest_known_aggregated_payloads
                .insert(data, proof.clone());
        }

        prune_stale_attestation_data(&mut store);

        assert_eq!(
            store
                .attestation_signatures
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![survivor]
        );
        assert_eq!(
            store
                .latest_new_aggregated_payloads
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![survivor]
        );
        assert_eq!(
            store
                .latest_known_aggregated_payloads
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![survivor]
        );
    }
}
