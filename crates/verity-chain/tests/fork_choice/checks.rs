//! The generator's own per-step assertions, on top of the snapshot.

use std::collections::{HashMap, HashSet};

use crate::common::compare;
use crate::fork_choice::labels::Labels;
use crate::fork_choice::shapes::Step;
use serde::Deserialize;
use verity_chain::fork_choice::duties::attestation_target;
use verity_chain::{Store, block_weights, hash_tree_root};
use verity_types::{AttestationData, Bytes32, SingleMessageAggregate, ValidatorIndex};

// ---------------------------------------------------------------------------------------
// The `checks` block
// ---------------------------------------------------------------------------------------

/// The generator's own per-step assertions, on top of the snapshot.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ChecksJson {
    pub time: Option<u64>,
    pub head_slot: Option<u64>,
    pub head_root_label: Option<String>,
    pub safe_target_slot: Option<u64>,
    pub safe_target_root_label: Option<String>,
    pub latest_justified_slot: Option<u64>,
    pub latest_justified_root_label: Option<String>,
    pub latest_finalized_slot: Option<u64>,
    pub latest_finalized_root_label: Option<String>,
    pub attestation_target_slot: Option<u64>,
    pub attestation_target_root_label: Option<String>,
    pub attestation_signature_target_slots: Option<Vec<u64>>,
    pub latest_new_aggregated_target_slots: Option<Vec<u64>>,
    pub latest_known_aggregated_target_slots: Option<Vec<u64>>,
    pub new_pool_proof_participants: Option<HashMap<String, Vec<u64>>>,
    pub block_attestation_count: Option<usize>,
    pub block_attestations: Option<Vec<BlockAttestationCheck>>,
    pub attestation_checks: Option<Vec<AttestationCheck>>,
    pub labels_in_store: Option<Vec<String>>,
    pub lexicographic_head_among: Option<Vec<String>>,
    pub canonical_equivocation_head_among: Option<Vec<String>>,
    pub filled_block_root_label: Option<String>,
    pub reorg_depth: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct BlockAttestationCheck {
    pub participants: Vec<u64>,
    pub attestation_slot: Option<u64>,
    pub target_slot: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct AttestationCheck {
    pub validator: u64,
    pub attestation_slot: Option<u64>,
    pub head_slot: Option<u64>,
    pub source_slot: Option<u64>,
    pub source_root_label: Option<String>,
    pub target_slot: Option<u64>,
    pub location: String,
}

impl ChecksJson {
    pub fn check(
        &self,
        failures: &mut Vec<String>,
        store: &Store,
        step: &Step,
        labels: &Labels,
        previous_head: Bytes32,
    ) -> Result<(), String> {
        compare(failures, "time", self.time, Some(store.time.0));
        self.check_checkpoints(failures, store, labels)?;
        self.check_target(failures, store, labels)?;
        self.check_pools(failures, store);
        self.check_block_body(failures, step, labels)?;
        self.check_ties(failures, store, labels)?;

        if let Some(expected) = self.reorg_depth {
            compare(
                failures,
                "reorgDepth",
                Some(expected),
                Some(reorg_depth(store, previous_head)),
            );
        }
        if let Some(names) = &self.labels_in_store {
            for name in names {
                let root = labels.root(name)?;
                if !store.blocks.contains_key(&root) {
                    failures.push(format!("labelsInStore: {name} is not in the store"));
                }
            }
        }
        Ok(())
    }

    /// Head, safe target, and the two checkpoints, by slot and by symbolic name.
    fn check_checkpoints(
        &self,
        failures: &mut Vec<String>,
        store: &Store,
        labels: &Labels,
    ) -> Result<(), String> {
        let slot_of = |root: Bytes32| store.blocks.get(&root).map(|block| block.slot.0);

        compare(failures, "headSlot", self.head_slot, slot_of(store.head));
        compare(
            failures,
            "safeTargetSlot",
            self.safe_target_slot,
            slot_of(store.safe_target),
        );
        compare(
            failures,
            "latestJustifiedSlot",
            self.latest_justified_slot,
            Some(store.latest_justified.slot.0),
        );
        compare(
            failures,
            "latestFinalizedSlot",
            self.latest_finalized_slot,
            Some(store.latest_finalized.slot.0),
        );

        let named = [
            ("headRootLabel", &self.head_root_label, store.head),
            (
                "safeTargetRootLabel",
                &self.safe_target_root_label,
                store.safe_target,
            ),
            (
                "latestJustifiedRootLabel",
                &self.latest_justified_root_label,
                store.latest_justified.root,
            ),
            (
                "latestFinalizedRootLabel",
                &self.latest_finalized_root_label,
                store.latest_finalized.root,
            ),
        ];
        for (name, expected, actual) in named {
            let Some(label) = expected else { continue };
            labels.check(failures, name, label, actual)?;
        }
        Ok(())
    }

    /// The checkpoint a validator would name as its attestation target.
    fn check_target(
        &self,
        failures: &mut Vec<String>,
        store: &Store,
        labels: &Labels,
    ) -> Result<(), String> {
        if self.attestation_target_slot.is_none() && self.attestation_target_root_label.is_none() {
            return Ok(());
        }
        let target = attestation_target(store);
        compare(
            failures,
            "attestationTargetSlot",
            self.attestation_target_slot,
            Some(target.slot.0),
        );
        if let Some(label) = &self.attestation_target_root_label {
            labels.check(failures, "attestationTargetRootLabel", label, target.root)?;
        }
        Ok(())
    }

    /// Which target slots each pool holds, and the pending pool's participant union.
    fn check_pools(&self, failures: &mut Vec<String>, store: &Store) {
        let target_slots = |pool: &HashMap<AttestationData, HashSet<SingleMessageAggregate>>| {
            let mut slots: Vec<u64> = pool.keys().map(|data| data.target.slot.0).collect();
            slots.sort_unstable();
            slots.dedup();
            slots
        };

        let mut signature_slots: Vec<u64> = store
            .attestation_signatures
            .keys()
            .map(|data| data.target.slot.0)
            .collect();
        signature_slots.sort_unstable();
        signature_slots.dedup();
        compare(
            failures,
            "attestationSignatureTargetSlots",
            self.attestation_signature_target_slots.clone(),
            Some(signature_slots),
        );
        compare(
            failures,
            "latestNewAggregatedTargetSlots",
            self.latest_new_aggregated_target_slots.clone(),
            Some(target_slots(&store.latest_new_aggregated_payloads)),
        );
        compare(
            failures,
            "latestKnownAggregatedTargetSlots",
            self.latest_known_aggregated_target_slots.clone(),
            Some(target_slots(&store.latest_known_aggregated_payloads)),
        );

        if let Some(expected) = &self.new_pool_proof_participants {
            for (slot, wanted) in expected {
                let mut union: Vec<u64> = store
                    .latest_new_aggregated_payloads
                    .iter()
                    .filter(|(data, _)| data.target.slot.0.to_string() == *slot)
                    .flat_map(|(_, proofs)| proofs)
                    .flat_map(|proof| verity_chain::fork_choice::participants(&proof.participants))
                    .map(|index| index.0)
                    .collect();
                union.sort_unstable();
                union.dedup();
                let mut wanted = wanted.clone();
                wanted.sort_unstable();
                compare(
                    failures,
                    &format!("newPoolProofParticipants[{slot}]"),
                    Some(wanted),
                    Some(union),
                );
            }
        }
    }

    /// What the step's own block carries, and the label it was registered under.
    fn check_block_body(
        &self,
        failures: &mut Vec<String>,
        step: &Step,
        labels: &Labels,
    ) -> Result<(), String> {
        let needs_block = self.block_attestation_count.is_some()
            || self.block_attestations.is_some()
            || self.filled_block_root_label.is_some();
        if !needs_block {
            return Ok(());
        }
        let block = step
            .block
            .as_ref()
            .ok_or("a block check on a step with no block")?
            .build()?;

        compare(
            failures,
            "blockAttestationCount",
            self.block_attestation_count,
            Some(block.body.attestations.len()),
        );
        if let Some(expected) = &self.block_attestations {
            if expected.len() != block.body.attestations.len() {
                failures.push(format!(
                    "blockAttestations: got {} entries, expected {}",
                    block.body.attestations.len(),
                    expected.len()
                ));
            }
            for (check, attestation) in expected.iter().zip(block.body.attestations.iter()) {
                let actual: Vec<u64> =
                    verity_chain::fork_choice::participants(&attestation.aggregation_bits)
                        .map(|index| index.0)
                        .collect();
                compare(
                    failures,
                    "blockAttestations.participants",
                    Some(check.participants.clone()),
                    Some(actual),
                );
                compare(
                    failures,
                    "blockAttestations.attestationSlot",
                    check.attestation_slot,
                    Some(attestation.data.slot.0),
                );
                compare(
                    failures,
                    "blockAttestations.targetSlot",
                    check.target_slot,
                    Some(attestation.data.target.slot.0),
                );
            }
        }
        if let Some(label) = &self.filled_block_root_label {
            compare(
                failures,
                "filledBlockRootLabel",
                Some(labels.root(label)?),
                Some(hash_tree_root(&block)),
            );
        }
        Ok(())
    }

    /// The two tiebreak assertions, plus the per-validator pool content checks.
    fn check_ties(
        &self,
        failures: &mut Vec<String>,
        store: &Store,
        labels: &Labels,
    ) -> Result<(), String> {
        if let Some(names) = &self.lexicographic_head_among {
            let weights = block_weights(store);
            let mut roots = Vec::new();
            for name in names {
                let root = labels.root(name)?;
                roots.push((weights.get(&root).copied().unwrap_or_default(), root));
            }
            let tied = roots.iter().all(|(weight, _)| *weight == roots[0].0);
            if !tied {
                failures.push(format!("lexicographicHeadAmong: weights differ: {roots:?}"));
            }
            let winner = roots.iter().map(|(_, root)| *root).max();
            compare(
                failures,
                "lexicographicHeadAmong",
                winner.map(|root| labels.name(root)),
                Some(labels.name(store.head)),
            );
        }

        if let Some(names) = &self.canonical_equivocation_head_among {
            let mut best: Option<(Bytes32, Bytes32)> = None;
            for name in names {
                let root = labels.root(name)?;
                let Some(vote) = store
                    .latest_known_aggregated_payloads
                    .keys()
                    .filter(|data| data.head.root == root)
                    .map(hash_tree_root)
                    .max()
                else {
                    failures.push(format!("canonicalEquivocationHeadAmong: {name} unattested"));
                    continue;
                };
                if best.is_none_or(|(seen, _)| vote > seen) {
                    best = Some((vote, root));
                }
            }
            compare(
                failures,
                "canonicalEquivocationHeadAmong",
                best.map(|(_, root)| labels.name(root)),
                Some(labels.name(store.head)),
            );
        }

        if let Some(checks) = &self.attestation_checks {
            for check in checks {
                check.check(failures, store, labels)?;
            }
        }
        Ok(())
    }
}

impl AttestationCheck {
    /// The vote a named pool records for one validator, by canonical precedence.
    pub fn check(
        &self,
        failures: &mut Vec<String>,
        store: &Store,
        labels: &Labels,
    ) -> Result<(), String> {
        let voter = ValidatorIndex(self.validator);
        let winner = match self.location.as_str() {
            "signatures" => best_vote(store.attestation_signatures.iter().filter_map(
                |(data, entries)| {
                    entries
                        .iter()
                        .any(|entry| entry.validator_index == voter)
                        .then_some(*data)
                },
            )),
            "new" => best_vote(votes_for(&store.latest_new_aggregated_payloads, voter)),
            "known" => best_vote(votes_for(&store.latest_known_aggregated_payloads, voter)),
            other => return Err(format!("unknown attestation check location {other}")),
        };
        let Some(vote) = winner else {
            failures.push(format!(
                "attestationChecks: validator {} not in the {} pool",
                self.validator, self.location
            ));
            return Ok(());
        };

        compare(
            failures,
            "attestationChecks.attestationSlot",
            self.attestation_slot,
            Some(vote.slot.0),
        );
        compare(
            failures,
            "attestationChecks.headSlot",
            self.head_slot,
            Some(vote.head.slot.0),
        );
        compare(
            failures,
            "attestationChecks.sourceSlot",
            self.source_slot,
            Some(vote.source.slot.0),
        );
        compare(
            failures,
            "attestationChecks.targetSlot",
            self.target_slot,
            Some(vote.target.slot.0),
        );
        if let Some(label) = &self.source_root_label {
            compare(
                failures,
                "attestationChecks.sourceRootLabel",
                Some(labels.root(label)?),
                Some(vote.source.root),
            );
        }
        Ok(())
    }
}

/// Every vote in a pool that names `voter` as a participant.
fn votes_for(
    pool: &HashMap<AttestationData, HashSet<SingleMessageAggregate>>,
    voter: ValidatorIndex,
) -> impl Iterator<Item = AttestationData> + '_ {
    pool.iter().filter_map(move |(data, proofs)| {
        proofs
            .iter()
            .any(|proof| {
                verity_chain::fork_choice::participants(&proof.participants).any(|i| i == voter)
            })
            .then_some(*data)
    })
}

/// The winner by canonical precedence: highest slot, then largest attestation-data root.
fn best_vote(votes: impl Iterator<Item = AttestationData>) -> Option<AttestationData> {
    votes.max_by_key(|data| (data.slot, hash_tree_root(data)))
}

/// How many blocks the old head sat above its common ancestor with the new one.
fn reorg_depth(store: &Store, previous_head: Bytes32) -> usize {
    let mut on_new_chain = HashSet::new();
    let mut cursor = store.head;
    while let Some(block) = store.blocks.get(&cursor) {
        if !on_new_chain.insert(cursor) {
            break;
        }
        cursor = block.parent_root;
    }

    let mut depth = 0;
    let mut cursor = previous_head;
    while let Some(block) = store.blocks.get(&cursor) {
        if on_new_chain.contains(&cursor) {
            break;
        }
        depth += 1;
        cursor = block.parent_root;
    }
    depth
}
