//! Proposer-side block building: which votes a block carries, and the post-state it commits to.
//!
//! # Why selection is circular
//!
//! A vote may only build from an already-justified source, and including votes is the act
//! that justifies sources. The eligible set therefore grows as votes are added, so the
//! proposer selects in rounds and stops once a pass adds nothing.
//!
//! # Where the cryptography went
//!
//! leanSpec collapses several proofs over one vote into a single proof by re-aggregating
//! them, which is a `verity-crypto` operation and this crate has none (see the crate docs).
//! [`build_block`] therefore computes the collapsed *body* — a block carries one attestation
//! per vote, over the union of the voters — and hands back the proofs that union covers, in
//! body order, for the caller to fold. Merging changes no voter, so the post-state returned
//! here is the one the folded block produces.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/block_production.py` and
//! `aggregation.py`, read at commit `8603fa63`.

use std::collections::{HashMap, HashSet};

use verity_types::config::MAX_ATTESTATIONS_DATA;
use verity_types::primitives::ZERO_HASH;
use verity_types::state::JustifiedSlots;
use verity_types::{
    AggregatedAttestation, AggregatedAttestations, AggregationBits, AttestationData, Block,
    BlockBody, Bytes32, Checkpoint, SingleMessageAggregate, Slot, State, ValidatorIndex,
};

use libssz::SszEncode;

use crate::error::RejectionReason;
use crate::fork_choice::weights::participants;
use crate::justification::{extend_justified_slots_to, is_slot_justified};
use crate::merkle::hash_tree_root;
use crate::state_transition::attestations::is_on_chain;
use crate::state_transition::{process_block, process_slots};

/// A block a proposer may sign, with everything the signer needs to finish it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltBlock {
    /// The block, already committing to the post-state below.
    pub block: Block,
    /// The state the block produces.
    pub post_state: State,
    /// For each attestation in the body, in order, the proofs its bits are the union of.
    ///
    /// A single-element entry is already the proof the block needs. A longer one must be
    /// folded into one proof covering the same voters before the block can carry it — the
    /// bits in the body are already the union, so folding changes the proof and nothing else.
    pub components: Vec<Vec<SingleMessageAggregate>>,
}

/// Builds a block on `state`, filling it with as many eligible votes as it can justify.
///
/// `known_block_roots` is the set of blocks the proposer has seen: a vote whose head it
/// cannot resolve is skipped rather than propagated. `aggregated_payloads` is the pool the
/// votes are drawn from, grouped by the vote they attest to.
///
/// # Errors
///
/// [`RejectionReason::TooManyAttestationData`] when the selected votes overflow the body's
/// SSZ limit, [`RejectionReason::JustifiedSlotOutOfRange`] when the justification window
/// cannot address a candidate's slot, and any [`RejectionReason`] the state transition
/// produces for a trial block.
#[must_use = "this builds the block; signing and broadcasting it are the validator's job"]
pub fn build_block(
    state: &State,
    slot: Slot,
    proposer_index: ValidatorIndex,
    parent_root: Bytes32,
    known_block_roots: &HashSet<Bytes32>,
    aggregated_payloads: &HashMap<AttestationData, HashSet<SingleMessageAggregate>>,
) -> Result<BuiltBlock, RejectionReason> {
    let advanced_state = process_slots(state, slot)?;

    let selected = if aggregated_payloads.is_empty() {
        Vec::new()
    } else {
        select_votes(
            state,
            &advanced_state,
            slot,
            proposer_index,
            parent_root,
            known_block_roots,
            aggregated_payloads,
        )?
    };

    let attestations: Vec<AggregatedAttestation> = selected
        .iter()
        .map(|(data, proofs)| {
            Ok(AggregatedAttestation {
                aggregation_bits: union_of(proofs)?,
                data: *data,
            })
        })
        .collect::<Result<_, RejectionReason>>()?;

    let mut block = Block {
        slot,
        proposer_index,
        parent_root,
        state_root: ZERO_HASH,
        body: BlockBody {
            attestations: to_body_list(attestations)?,
        },
    };

    // Folding each group into one proof keeps the same voters, so the post-state of the
    // collapsed body is the post-state of the block that will actually be signed.
    let post_state = process_block(&advanced_state, &block)?;
    block.state_root = hash_tree_root(&post_state);

    Ok(BuiltBlock {
        block,
        post_state,
        components: selected.into_iter().map(|(_, proofs)| proofs).collect(),
    })
}

/// The fixed-point selection: rounds of eligible votes until a pass adds nothing.
///
/// Returns one entry per distinct vote, in the order the votes were accepted, each carrying
/// the proofs chosen for it across every pass.
fn select_votes(
    state: &State,
    advanced_state: &State,
    slot: Slot,
    proposer_index: ValidatorIndex,
    parent_root: Bytes32,
    known_block_roots: &HashSet<Bytes32>,
    aggregated_payloads: &HashMap<AttestationData, HashSet<SingleMessageAggregate>>,
) -> Result<Vec<(AttestationData, Vec<SingleMessageAggregate>)>, RejectionReason> {
    // On genesis the parent is justified at slot 0 by header processing, so anchoring there
    // is what makes the first block's eligible sources match the votes that exist.
    let mut justified = if state.latest_block_header.slot.0 == 0 {
        Checkpoint {
            root: parent_root,
            slot: Slot(0),
        }
    } else {
        state.latest_justified
    };
    let mut finalized_slot = state.latest_finalized.slot;
    let mut justified_slots = extend_justified_slots_to(
        &state.justified_slots,
        finalized_slot,
        Slot(slot.0.saturating_sub(1)),
    )?;

    let chain_view = extended_chain_view(state, slot, parent_root);
    let candidates = in_target_slot_order(aggregated_payloads);

    // Insertion-ordered accumulation: `order` fixes the body's attestation order, `groups`
    // holds every proof chosen for a vote across the passes that reached it.
    let mut order: Vec<AttestationData> = Vec::new();
    let mut groups: HashMap<AttestationData, Vec<SingleMessageAggregate>> = HashMap::new();

    loop {
        let mut found_new_entries = false;

        for (data, proofs) in &candidates {
            if groups.contains_key(data) {
                continue;
            }
            // A proposer-side budget on distinct votes, not a consensus rule.
            if order.len() >= MAX_ATTESTATIONS_DATA as usize {
                break;
            }
            if !is_eligible(
                data,
                known_block_roots,
                &chain_view,
                justified,
                &justified_slots,
                finalized_slot,
            )? {
                continue;
            }

            found_new_entries = true;
            order.push(*data);
            groups.insert(*data, select_proofs_for_coverage(Some(proofs), None).0);
        }

        if !found_new_entries {
            break;
        }

        // A trial block's post-state is what reveals whether this pass moved justification.
        let trial = Block {
            slot,
            proposer_index,
            parent_root,
            state_root: ZERO_HASH,
            body: BlockBody {
                attestations: trial_body(&order, &groups)?,
            },
        };
        let post_state = process_block(advanced_state, &trial)?;

        // Both advance monotonically, so re-anchoring cannot loop forever. A finalization
        // step slides the justified window forward, which can make targets eligible that
        // were out of range before.
        if post_state.latest_justified == justified
            && post_state.latest_finalized.slot == finalized_slot
        {
            break;
        }
        justified = post_state.latest_justified;
        justified_slots = post_state.justified_slots;
        finalized_slot = post_state.latest_finalized.slot;
    }

    Ok(order
        .into_iter()
        .map(|data| {
            let proofs = groups
                .remove(&data)
                .expect("every accepted vote has a group");
            (data, proofs)
        })
        .collect())
}

/// Whether a candidate vote may enter the block being built.
fn is_eligible(
    data: &AttestationData,
    known_block_roots: &HashSet<Bytes32>,
    chain_view: &[Bytes32],
    justified: Checkpoint,
    justified_slots: &JustifiedSlots,
    finalized_slot: Slot,
) -> Result<bool, RejectionReason> {
    // A vote whose head the proposer has not seen is one it cannot stand behind.
    if !known_block_roots.contains(&data.head.root) {
        return Ok(false);
    }
    // A vote may only build from the checkpoint this chain currently treats as justified.
    if data.source.slot != justified.slot {
        return Ok(false);
    }
    // Off-chain votes are rejected here rather than by the transition, which also keeps the
    // justification lookups below inside the window.
    if !is_on_chain(data, chain_view) {
        return Ok(false);
    }
    if !is_slot_justified(justified_slots, finalized_slot, data.source.slot)? {
        return Ok(false);
    }

    // A genesis self-vote justifies nothing and the transition drops it, but it carries head
    // weight and including it propagates it. Slot 0 counts as justified, so without this
    // exemption the target check below would drop it.
    let is_genesis_self_vote = data.source.slot.0 == 0 && data.target.slot.0 == 0;
    if is_genesis_self_vote {
        return Ok(true);
    }

    // An already-justified target gains nothing from more votes.
    Ok(!is_slot_justified(
        justified_slots,
        finalized_slot,
        data.target.slot,
    )?)
}

/// The chain as it will look once this block is applied: history, the parent at its own
/// slot, then a zero hash for each slot skipped before this one.
fn extended_chain_view(state: &State, slot: Slot, parent_root: Bytes32) -> Vec<Bytes32> {
    let empty_slots = slot
        .0
        .saturating_sub(state.latest_block_header.slot.0)
        .saturating_sub(1);

    let mut view: Vec<Bytes32> = state.historical_block_hashes.to_vec();
    view.push(parent_root);
    view.extend(std::iter::repeat_n(ZERO_HASH, empty_slots as usize));
    view
}

/// Candidates ordered by target slot, ties broken by the vote's own root.
///
/// The tie-break is content-derived rather than arrival-ordered, which is what makes every
/// node truncate the same candidate at the distinct-vote budget.
fn in_target_slot_order(
    aggregated_payloads: &HashMap<AttestationData, HashSet<SingleMessageAggregate>>,
) -> Vec<(AttestationData, &HashSet<SingleMessageAggregate>)> {
    let mut candidates: Vec<(AttestationData, &HashSet<SingleMessageAggregate>)> =
        aggregated_payloads
            .iter()
            .map(|(data, proofs)| (*data, proofs))
            .collect();
    candidates.sort_by_key(|(data, _)| (data.target.slot.0, hash_tree_root(data)));
    candidates
}

/// The body of a trial block: every proof chosen so far, one attestation per proof.
///
/// Trial bodies are not collapsed. The passes are only asking what the transition justifies,
/// and the transition weighs voters rather than the shape they arrived in.
fn trial_body(
    order: &[AttestationData],
    groups: &HashMap<AttestationData, Vec<SingleMessageAggregate>>,
) -> Result<AggregatedAttestations, RejectionReason> {
    let attestations: Vec<AggregatedAttestation> = order
        .iter()
        .flat_map(|data| {
            groups
                .get(data)
                .into_iter()
                .flatten()
                .map(move |proof| AggregatedAttestation {
                    aggregation_bits: proof.participants.clone(),
                    data: *data,
                })
        })
        .collect();
    to_body_list(attestations)
}

fn to_body_list(
    attestations: Vec<AggregatedAttestation>,
) -> Result<AggregatedAttestations, RejectionReason> {
    AggregatedAttestations::try_from(attestations)
        .map_err(|_| RejectionReason::TooManyAttestationData)
}

/// The bitfield naming every validator any of `proofs` covers.
fn union_of(proofs: &[SingleMessageAggregate]) -> Result<AggregationBits, RejectionReason> {
    let width = proofs
        .iter()
        .map(|proof| proof.participants.len())
        .max()
        .unwrap_or(0);

    let mut bits = vec![false; width];
    for proof in proofs {
        for index in participants(&proof.participants) {
            let index = index.0 as usize;
            if index < width {
                bits[index] = true;
            }
        }
    }

    AggregationBits::try_from(bits).map_err(|_| RejectionReason::EmptyAggregationBits)
}

/// Greedily picks proofs covering as many distinct validators as possible.
///
/// The priority pool is consulted before the fallback pool, so uncommitted work is reused
/// before proofs already counted. Ties on new coverage fall to the largest canonical
/// encoding: without a content-derived tie-break, two nodes with the same pool could pick
/// differently and produce different blocks.
///
/// Returns the chosen proofs and the union of the validators they cover.
#[must_use = "this chooses the proofs; aggregating them is the caller's job"]
pub fn select_proofs_for_coverage(
    priority_pool: Option<&HashSet<SingleMessageAggregate>>,
    fallback_pool: Option<&HashSet<SingleMessageAggregate>>,
) -> (Vec<SingleMessageAggregate>, HashSet<ValidatorIndex>) {
    let mut selected: Vec<SingleMessageAggregate> = Vec::new();
    let mut covered: HashSet<ValidatorIndex> = HashSet::new();

    for pool in [priority_pool, fallback_pool].into_iter().flatten() {
        // Each proof's validator set is materialized once: otherwise every comparison in the
        // loop below would re-walk the bitfield.
        let mut candidates: Vec<(&SingleMessageAggregate, HashSet<ValidatorIndex>)> = pool
            .iter()
            .map(|proof| (proof, participants(&proof.participants).collect()))
            .collect();

        while !candidates.is_empty() {
            let best = candidates
                .iter()
                .enumerate()
                .max_by_key(|(_, (proof, voters))| {
                    (voters.difference(&covered).count(), proof.to_ssz())
                })
                .map(|(position, _)| position)
                .expect("the list is not empty");

            let (proof, voters) = candidates.swap_remove(best);
            // The best pick adds the most, so if it adds nothing then nothing does.
            if voters.is_subset(&covered) {
                break;
            }
            covered.extend(voters);
            selected.push(proof.clone());
        }
    }

    (selected, covered)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use verity_types::{AttestationData, Checkpoint, SingleMessageAggregate, Slot, ValidatorIndex};

    use crate::fork_choice::testing::anchored_on_genesis;
    use crate::merkle::hash_tree_root;

    use super::{AggregationBits, build_block, select_proofs_for_coverage, union_of};

    fn proof(bits: &[bool]) -> SingleMessageAggregate {
        SingleMessageAggregate {
            participants: AggregationBits::try_from(bits.to_vec()).unwrap(),
            proof: Default::default(),
        }
    }

    fn vote(source_slot: u64, target_slot: u64, root: [u8; 32]) -> AttestationData {
        AttestationData {
            slot: Slot(target_slot + 1),
            head: Checkpoint {
                root,
                slot: Slot(target_slot),
            },
            target: Checkpoint {
                root,
                slot: Slot(target_slot),
            },
            source: Checkpoint {
                root,
                slot: Slot(source_slot),
            },
        }
    }

    #[test]
    fn should_stop_once_every_validator_is_covered() {
        let pool = HashSet::from([proof(&[true, true, false]), proof(&[false, true, false])]);
        let (selected, covered) = select_proofs_for_coverage(Some(&pool), None);

        assert_eq!(selected.len(), 1);
        assert_eq!(
            covered,
            HashSet::from([ValidatorIndex(0), ValidatorIndex(1)])
        );
    }

    #[test]
    fn should_take_both_when_neither_proof_contains_the_other() {
        let pool = HashSet::from([proof(&[true, false, false]), proof(&[false, false, true])]);
        let (selected, covered) = select_proofs_for_coverage(Some(&pool), None);

        assert_eq!(selected.len(), 2);
        assert_eq!(
            covered,
            HashSet::from([ValidatorIndex(0), ValidatorIndex(2)])
        );
    }

    #[test]
    fn should_consult_the_priority_pool_before_the_fallback() {
        let priority = HashSet::from([proof(&[true, true, false])]);
        let fallback = HashSet::from([proof(&[true, false, false])]);
        let (selected, _) = select_proofs_for_coverage(Some(&priority), Some(&fallback));

        // The fallback adds nobody the priority pick has not already covered.
        assert_eq!(selected, vec![proof(&[true, true, false])]);
    }

    #[test]
    fn should_name_every_voter_when_proofs_are_unioned() {
        let bits = union_of(&[proof(&[true, false, false]), proof(&[false, false, true])]).unwrap();
        assert_eq!(
            bits,
            AggregationBits::try_from(vec![true, false, true]).unwrap()
        );
    }

    #[test]
    fn should_build_an_empty_block_when_the_pool_holds_nothing() {
        let (store, genesis) = anchored_on_genesis(4);
        let parent_root = store.head;

        let built = build_block(
            &genesis,
            Slot(1),
            ValidatorIndex(1),
            parent_root,
            &HashSet::new(),
            &HashMap::new(),
        )
        .expect("an empty block on genesis");

        assert!(built.block.body.attestations.is_empty());
        assert!(built.components.is_empty());
        assert_eq!(built.block.state_root, hash_tree_root(&built.post_state));
    }

    #[test]
    fn should_skip_a_vote_whose_head_the_proposer_has_not_seen() {
        let (store, genesis) = anchored_on_genesis(4);
        let parent_root = store.head;
        let payloads = HashMap::from([(
            vote(0, 0, parent_root),
            HashSet::from([proof(&[true, false, false, false])]),
        )]);

        let built = build_block(
            &genesis,
            Slot(1),
            ValidatorIndex(1),
            parent_root,
            &HashSet::new(),
            &payloads,
        )
        .expect("block building does not fail on an unusable vote");

        assert!(built.block.body.attestations.is_empty());
    }

    #[test]
    fn should_carry_a_genesis_self_vote_whose_head_it_knows() {
        let (store, genesis) = anchored_on_genesis(4);
        let parent_root = store.head;
        let data = vote(0, 0, parent_root);
        let payloads =
            HashMap::from([(data, HashSet::from([proof(&[true, false, false, false])]))]);

        let built = build_block(
            &genesis,
            Slot(1),
            ValidatorIndex(1),
            parent_root,
            &HashSet::from([parent_root]),
            &payloads,
        )
        .expect("a block carrying one genesis self-vote");

        assert_eq!(built.block.body.attestations.len(), 1);
        assert_eq!(built.block.body.attestations[0].data, data);
        assert_eq!(built.components.len(), 1);
    }
}
