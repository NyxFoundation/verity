//! The ten observables leanSpec records after every step, and how they are compared.

use std::collections::{HashMap, HashSet};

use crate::common::{self, compare, hex};
use serde::Deserialize;
use verity_chain::{Store, block_weights, hash_tree_root};
use verity_types::{AttestationData, SingleMessageAggregate};

// ---------------------------------------------------------------------------------------
// The store snapshot
// ---------------------------------------------------------------------------------------

/// Every observable leanSpec records after a step, accepted or rejected.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreSnapshotJson {
    pub time: u64,
    pub head_root: String,
    pub safe_target_root: String,
    pub latest_justified: common::CheckpointJson,
    pub latest_finalized: common::CheckpointJson,
    pub block_roots: Vec<String>,
    pub block_weights: Vec<BlockWeightJson>,
    pub attestation_signatures: Vec<SignaturePoolJson>,
    pub new_aggregated_payloads: Vec<AggregatedPoolJson>,
    pub known_aggregated_payloads: Vec<AggregatedPoolJson>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockWeightJson {
    pub root: String,
    pub weight: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignaturePoolJson {
    pub data_root: String,
    pub validator_indices: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AggregatedPoolJson {
    pub data_root: String,
    pub participant_sets: Vec<Vec<u64>>,
}

impl StoreSnapshotJson {
    /// Compares all ten observables against the store.
    pub fn check(&self, failures: &mut Vec<String>, store: &Store) {
        compare(failures, "time", Some(self.time), Some(store.time.0));
        compare(
            failures,
            "headRoot",
            Some(self.head_root.clone()),
            Some(hex(&store.head)),
        );
        compare(
            failures,
            "safeTargetRoot",
            Some(self.safe_target_root.clone()),
            Some(hex(&store.safe_target)),
        );
        compare(
            failures,
            "latestJustified",
            Some((
                self.latest_justified.root.clone(),
                self.latest_justified.slot,
            )),
            Some((
                hex(&store.latest_justified.root),
                store.latest_justified.slot.0,
            )),
        );
        compare(
            failures,
            "latestFinalized",
            Some((
                self.latest_finalized.root.clone(),
                self.latest_finalized.slot,
            )),
            Some((
                hex(&store.latest_finalized.root),
                store.latest_finalized.slot.0,
            )),
        );

        let mut roots: Vec<String> = store.blocks.keys().map(|root| hex(root)).collect();
        roots.sort();
        compare(
            failures,
            "blockRoots",
            Some(self.block_roots.clone()),
            Some(roots),
        );

        self.check_weights(failures, store);
        self.check_pools(failures, store);
    }

    /// Weights are recorded for every block above the finalized slot, zero included.
    fn check_weights(&self, failures: &mut Vec<String>, store: &Store) {
        let weights = block_weights(store);
        let mut actual: Vec<(String, u64)> = store
            .blocks
            .iter()
            .filter(|(_, block)| block.slot.0 > store.latest_finalized.slot.0)
            .map(|(root, _)| (hex(root), weights.get(root).copied().unwrap_or_default()))
            .collect();
        actual.sort();

        let expected: Vec<(String, u64)> = self
            .block_weights
            .iter()
            .map(|entry| (entry.root.clone(), entry.weight))
            .collect();
        compare(failures, "blockWeights", Some(expected), Some(actual));
    }

    /// The three vote pools, as the coverage the snapshot records rather than as bytes.
    fn check_pools(&self, failures: &mut Vec<String>, store: &Store) {
        let mut signatures: Vec<(String, Vec<u64>)> = store
            .attestation_signatures
            .iter()
            .map(|(data, entries)| {
                let mut indices: Vec<u64> = entries
                    .iter()
                    .map(|entry| entry.validator_index.0)
                    .collect();
                indices.sort_unstable();
                (hex(&hash_tree_root(data)), indices)
            })
            .collect();
        signatures.sort();
        let expected: Vec<(String, Vec<u64>)> = self
            .attestation_signatures
            .iter()
            .map(|entry| (entry.data_root.clone(), entry.validator_indices.clone()))
            .collect();
        compare(
            failures,
            "attestationSignatures",
            Some(expected),
            Some(signatures),
        );

        compare(
            failures,
            "newAggregatedPayloads",
            Some(pool_entries(&self.new_aggregated_payloads)),
            Some(coverage(&store.latest_new_aggregated_payloads)),
        );
        compare(
            failures,
            "knownAggregatedPayloads",
            Some(pool_entries(&self.known_aggregated_payloads)),
            Some(coverage(&store.latest_known_aggregated_payloads)),
        );
    }
}

fn pool_entries(entries: &[AggregatedPoolJson]) -> Vec<(String, Vec<Vec<u64>>)> {
    entries
        .iter()
        .map(|entry| (entry.data_root.clone(), entry.participant_sets.clone()))
        .collect()
}

/// A pool as the snapshot sees it: participants per proof, sorted, proof bytes dropped.
fn coverage(
    pool: &HashMap<AttestationData, HashSet<SingleMessageAggregate>>,
) -> Vec<(String, Vec<Vec<u64>>)> {
    let mut out: Vec<(String, Vec<Vec<u64>>)> = pool
        .iter()
        .map(|(data, proofs)| {
            let mut sets: Vec<Vec<u64>> = proofs
                .iter()
                .map(|proof| {
                    verity_chain::fork_choice::participants(&proof.participants)
                        .map(|index| index.0)
                        .collect()
                })
                .collect();
            sets.sort();
            (hex(&hash_tree_root(data)), sets)
        })
        .collect();
    out.sort();
    out
}
