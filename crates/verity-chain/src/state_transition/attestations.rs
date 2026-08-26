//! Applying a block's attestations, and the justification and finalization that follow.
//!
//! This is the 3SF-mini accounting. The state stores votes as one flat bitlist segmented by
//! tracked root; this module unpacks that layout into per-root vote flags, applies the block's
//! votes, and packs it back.
//!
//! ```text
//!     roots:  [root_0,  root_1,  ...]
//!     votes:  [<--N-->][<--N-->] ...        N = validator count
//! ```
//!
//! Every capacity failure here surfaces as [`RejectionReason::JustifiedSlotOutOfRange`]
//! rather than a panic. All of them are unreachable: the tracked roots are bounded by the
//! chain view's own SSZ limit and the votes per root by the registry's, so their product is
//! exactly the flat bitlist's capacity. They exist so these functions are total.

use std::collections::{BTreeMap, HashMap, HashSet};

use libssz_types::SszBitlist;
use verity_types::config::MAX_ATTESTATIONS_DATA;
use verity_types::primitives::{Bytes32, Slot, ZERO_HASH};
use verity_types::{
    AggregatedAttestation, AggregationBits, AttestationData, Checkpoint, HistoricalBlockHashes,
    JustificationRoots, JustificationValidators, JustifiedSlots, State,
};

use crate::error::RejectionReason;
use crate::justification::{
    advance_checkpoint, is_justifiable_after, is_slot_justified, justified_index_after,
};

/// Applies `attestations` to `state`, moving the justified and finalized checkpoints.
///
/// # Errors
///
/// - [`RejectionReason::TooManyAttestationData`] when the block carries more distinct
///   attestation data entries than [`MAX_ATTESTATIONS_DATA`].
/// - [`RejectionReason::EmptyValidatorRegistry`] when there is no segment width to unpack by.
/// - [`RejectionReason::JustificationVotesLengthMismatch`] when the flat vote list is not the
///   tracked-root count times the validator count.
/// - [`RejectionReason::ZeroHashJustificationRoot`] when a tracked root marks an empty slot.
/// - [`RejectionReason::EmptyAggregationBits`] when an accepted vote names no validator.
/// - [`RejectionReason::ValidatorIndexOutOfRange`] when a set bit is outside the registry.
/// - [`RejectionReason::JustifiedSlotOutOfRange`] when a vote reaches past the tracked window.
#[must_use = "this returns the state after the votes; the argument keeps its old checkpoints"]
pub fn process_attestations(
    state: &State,
    attestations: &[AggregatedAttestation],
) -> Result<State, RejectionReason> {
    // Each distinct data builds a per-root vote table sized to the validator set, so the
    // distinct count is what drives work. Aggregates split over one target share their data
    // and count once.
    let distinct: HashSet<&AttestationData> = attestations
        .iter()
        .map(|attestation| &attestation.data)
        .collect();
    if distinct.len() > MAX_ATTESTATIONS_DATA as usize {
        return Err(RejectionReason::TooManyAttestationData);
    }

    let validator_count = state.validators.len();
    if validator_count == 0 {
        return Err(RejectionReason::EmptyValidatorRegistry);
    }

    let mut justification_state = JustificationState::unpack(state, validator_count)?;
    let root_to_slot = index_chain_by_slot(state);

    for attestation in attestations {
        justification_state.apply(attestation, state, &root_to_slot, validator_count)?;
    }

    justification_state.repack(state, validator_count)
}

/// Every unfinalized root in the chain view, mapped to the slot it sits at.
///
/// Used only to prune tracked roots once finalization moves; a root absent from it is off-chain.
fn index_chain_by_slot(state: &State) -> HashMap<Bytes32, Slot> {
    let start = state.latest_finalized.slot.0.saturating_add(1) as usize;
    state
        .historical_block_hashes
        .iter()
        .enumerate()
        .skip(start)
        .map(|(slot, root)| (*root, Slot(slot as u64)))
        .collect()
}

/// The justification and finalization accounting carried across a block's votes,
/// applied to the state in one step at the end.
struct JustificationState {
    /// Per-root vote flags. Ordered by root, which is what makes the repack canonical.
    justifications: BTreeMap<Bytes32, Vec<bool>>,
    justified_slots: JustifiedSlots,
    latest_justified: Checkpoint,
    latest_finalized: Checkpoint,
}

impl JustificationState {
    /// Recovers the per-root vote flags from the state's flat segmented layout.
    fn unpack(state: &State, validator_count: usize) -> Result<Self, RejectionReason> {
        let expected = state.justifications_roots.len() * validator_count;
        if state.justifications_validators.len() != expected {
            return Err(RejectionReason::JustificationVotesLengthMismatch);
        }
        // The zero hash marks a skipped slot, never a block, so it cannot carry votes.
        if state.justifications_roots.contains(&ZERO_HASH) {
            return Err(RejectionReason::ZeroHashJustificationRoot);
        }

        let flat = to_bits(&state.justifications_validators);
        Ok(Self {
            justifications: state
                .justifications_roots
                .iter()
                .zip(flat.chunks_exact(validator_count))
                .map(|(root, segment)| (*root, segment.to_vec()))
                .collect(),
            justified_slots: state.justified_slots.clone(),
            latest_justified: state.latest_justified,
            latest_finalized: state.latest_finalized,
        })
    }

    /// Applies one aggregate, ignoring it when any vote filter drops it.
    fn apply(
        &mut self,
        attestation: &AggregatedAttestation,
        state: &State,
        root_to_slot: &HashMap<Bytes32, Slot>,
        validator_count: usize,
    ) -> Result<(), RejectionReason> {
        let (source, target) = (attestation.data.source, attestation.data.target);
        if !self.counts(&attestation.data, state)? {
            return Ok(());
        }

        let voters = voting_validator_indices(&attestation.aggregation_bits)?;
        // A bit outside the registry has no flag in the per-root vote table. This guards the
        // unsigned path, where no signature stage catches it first.
        if voters.iter().any(|index| *index >= validator_count) {
            return Err(RejectionReason::ValidatorIndexOutOfRange);
        }

        let votes = self
            .justifications
            .entry(target.root)
            .or_insert_with(|| vec![false; validator_count]);
        // Re-marking a voter is idempotent, so no guard is needed.
        for index in voters {
            votes[index] = true;
        }

        // Justified once two thirds of validators vote for the target. Compared as integers
        // to keep floating point out of a consensus decision.
        let count = votes.iter().filter(|voted| **voted).count();
        if 3 * count >= 2 * validator_count {
            self.justify(source, target, root_to_slot)?;
        }
        Ok(())
    }

    /// Whether a vote survives every filter and should be counted.
    ///
    /// The order is leanSpec's: the two justification lookups can reject a vote outright, so
    /// they must run before the cheaper chain and distance checks.
    fn counts(&self, data: &AttestationData, state: &State) -> Result<bool, RejectionReason> {
        let (source, target) = (data.source, data.target);
        let finalized = self.latest_finalized.slot;

        // A vote may only anchor on an already-justified source, and an already-justified
        // target gains nothing from more votes.
        if !is_slot_justified(&self.justified_slots, finalized, source.slot)? {
            return Ok(false);
        }
        if is_slot_justified(&self.justified_slots, finalized, target.slot)? {
            return Ok(false);
        }
        // Both roots must match the canonical chain; this also rejects zero-hash roots.
        if !lies_on_chain(data, &state.historical_block_hashes) {
            return Ok(false);
        }
        if target.slot.0 <= source.slot.0 {
            return Ok(false);
        }
        // 3SF-mini admits a target only at a justifiable distance from the boundary.
        Ok(is_justifiable_after(target.slot, finalized))
    }

    /// Records `target` as justified, then finalizes `source` when nothing sits between them.
    fn justify(
        &mut self,
        source: Checkpoint,
        target: Checkpoint,
        root_to_slot: &HashMap<Bytes32, Slot>,
    ) -> Result<(), RejectionReason> {
        let finalized = self.latest_finalized.slot;

        // Targets within one block can resolve out of order, so an earlier target seen after
        // a later one must not drag the checkpoint back.
        self.latest_justified = advance_checkpoint(self.latest_justified, target);

        // In range: `apply` already dropped every target at or behind the boundary, and the
        // header stage grew the bitfield to cover the block's slot.
        let index = justified_index_after(target.slot, finalized)
            .ok_or(RejectionReason::JustifiedSlotOutOfRange)?;
        self.justified_slots
            .set(index, true)
            .map_err(|_| RejectionReason::JustifiedSlotOutOfRange)?;

        // The target is justified; its individual votes no longer matter.
        self.justifications.remove(&target.root);

        // Finalize the source when no justifiable slot sits strictly between it and the
        // target. A source at or behind the boundary is already final.
        let nothing_between = ((source.slot.0 + 1)..target.slot.0)
            .all(|slot| !is_justifiable_after(Slot(slot), finalized));
        if source.slot.0 > finalized.0 && nothing_between {
            self.rebase_onto(source, finalized, root_to_slot)?;
        }
        Ok(())
    }

    /// Moves the finalized boundary to `source` and drops what it leaves behind.
    fn rebase_onto(
        &mut self,
        source: Checkpoint,
        old_finalized: Slot,
        root_to_slot: &HashMap<Bytes32, Slot>,
    ) -> Result<(), RejectionReason> {
        self.latest_finalized = source;
        let finalized = source.slot;

        // Flags start one past the finalized slot, so advancing the boundary drops that many
        // from the front.
        let delta = (finalized.0 - old_finalized.0) as usize;
        if delta > 0 {
            let mut bits = to_bits(&self.justified_slots);
            bits.drain(..delta.min(bits.len()));
            self.justified_slots = from_bits(bits)?;

            // A root absent from the chain view is off-chain and can never justify. Drop such
            // a per-root vote table rather than tracking or rejecting it.
            self.justifications.retain(|root, _| {
                root_to_slot
                    .get(root)
                    .is_some_and(|slot| slot.0 > finalized.0)
            });
        }
        Ok(())
    }

    /// Flattens the per-root vote flags back into the state's segmented layout, roots first.
    fn repack(self, state: &State, validator_count: usize) -> Result<State, RejectionReason> {
        let mut roots = JustificationRoots::default();
        let mut votes: Vec<bool> = Vec::with_capacity(self.justifications.len() * validator_count);
        for (root, segment) in &self.justifications {
            roots
                .push(*root)
                .map_err(|_| RejectionReason::JustifiedSlotOutOfRange)?;
            votes.extend_from_slice(segment);
        }
        let justifications_validators: JustificationValidators = from_bits(votes)?;

        Ok(State {
            justifications_roots: roots,
            justifications_validators,
            justified_slots: self.justified_slots,
            latest_justified: self.latest_justified,
            latest_finalized: self.latest_finalized,
            ..state.clone()
        })
    }
}

/// Whether every checkpoint in the data points at the chain view's block for its slot.
fn lies_on_chain(data: &AttestationData, chain: &HistoricalBlockHashes) -> bool {
    // Empty slots carry the zero hash, so a vote recording one is meaningless.
    if data.source.root == ZERO_HASH || data.target.root == ZERO_HASH || data.head.root == ZERO_HASH
    {
        return false;
    }
    let length = chain.len() as u64;
    if data.source.slot.0 >= length || data.target.slot.0 >= length || data.head.slot.0 >= length {
        return false;
    }
    chain[data.source.slot.0 as usize] == data.source.root
        && chain[data.target.slot.0 as usize] == data.target.root
        && chain[data.head.slot.0 as usize] == data.head.root
}

/// The validator indices an aggregate names, ascending.
///
/// # Errors
///
/// [`RejectionReason::EmptyAggregationBits`] when the aggregate names nobody.
fn voting_validator_indices(bits: &AggregationBits) -> Result<Vec<usize>, RejectionReason> {
    let indices: Vec<usize> = (0..bits.len())
        .filter(|index| bits.get(*index) == Some(true))
        .collect();
    if indices.is_empty() {
        return Err(RejectionReason::EmptyAggregationBits);
    }
    Ok(indices)
}

/// Reads a bitlist out as plain flags, for the operations SSZ bitlists do not offer.
fn to_bits<const N: usize>(bitlist: &SszBitlist<N>) -> Vec<bool> {
    (0..bitlist.len())
        .map(|index| bitlist.get(index).unwrap_or(false))
        .collect()
}

/// Rebuilds a bitlist from plain flags.
fn from_bits<const N: usize>(bits: Vec<bool>) -> Result<SszBitlist<N>, RejectionReason> {
    SszBitlist::try_from(bits).map_err(|_| RejectionReason::JustifiedSlotOutOfRange)
}

#[cfg(test)]
mod tests {
    use verity_types::{AttestationData, Checkpoint};

    use crate::state_transition::testing::genesis_with;

    use super::{
        AggregatedAttestation, AggregationBits, MAX_ATTESTATIONS_DATA, RejectionReason, Slot,
        State, ZERO_HASH, process_attestations,
    };

    /// Genesis with the justification bitfield already grown, as the header stage leaves it.
    ///
    /// Without it every lookup past the boundary is out of range, which is a state no block
    /// can reach: `process_block_header` grows the bitfield before attestations are applied.
    fn tracked_genesis(validators: u64) -> State {
        let mut state = genesis_with(validators);
        state.justified_slots = super::from_bits(vec![false; 8]).expect("eight bits fit");
        state
    }

    fn attestation(target_slot: u64, bits: &[bool]) -> AggregatedAttestation {
        AggregatedAttestation {
            aggregation_bits: AggregationBits::try_from(bits.to_vec())
                .expect("well under the registry limit"),
            data: AttestationData {
                slot: Slot(target_slot),
                head: Checkpoint {
                    root: [1u8; 32],
                    slot: Slot(target_slot),
                },
                target: Checkpoint {
                    root: [1u8; 32],
                    slot: Slot(target_slot),
                },
                source: Checkpoint {
                    root: [2u8; 32],
                    slot: Slot(0),
                },
            },
        }
    }

    #[test]
    fn should_reject_more_distinct_attestation_data_than_the_cap() {
        let state = tracked_genesis(4);
        let votes: Vec<_> = (1..=u64::from(MAX_ATTESTATIONS_DATA) + 1)
            .map(|slot| attestation(slot, &[true]))
            .collect();
        assert_eq!(
            process_attestations(&state, &votes),
            Err(RejectionReason::TooManyAttestationData)
        );
    }

    #[test]
    fn should_count_repeated_data_once_against_the_cap() {
        let state = tracked_genesis(4);
        let votes: Vec<_> = (0..u64::from(MAX_ATTESTATIONS_DATA) + 4)
            .map(|_| attestation(1, &[true]))
            .collect();
        assert!(
            process_attestations(&state, &votes).is_ok(),
            "split aggregates for one target share their data and count once"
        );
    }

    #[test]
    fn should_reject_when_the_registry_is_empty() {
        let state = genesis_with(0);
        assert_eq!(
            process_attestations(&state, &[]),
            Err(RejectionReason::EmptyValidatorRegistry)
        );
    }

    #[test]
    fn should_reject_a_vote_list_that_does_not_segment_by_the_registry() {
        let mut state = tracked_genesis(4);
        state.justifications_roots.push([3u8; 32]).unwrap();
        // One root and four validators need four flags; three cannot be segmented.
        state.justifications_validators =
            super::from_bits(vec![false, false, false]).expect("three bits fit");
        assert_eq!(
            process_attestations(&state, &[]),
            Err(RejectionReason::JustificationVotesLengthMismatch)
        );
    }

    #[test]
    fn should_reject_a_tracked_root_that_marks_an_empty_slot() {
        let mut state = tracked_genesis(4);
        state.justifications_roots.push(ZERO_HASH).unwrap();
        state.justifications_validators = super::from_bits(vec![false; 4]).expect("four bits fit");
        assert_eq!(
            process_attestations(&state, &[]),
            Err(RejectionReason::ZeroHashJustificationRoot)
        );
    }

    #[test]
    fn should_leave_the_state_alone_when_a_vote_names_no_block_on_the_chain() {
        let state: State = tracked_genesis(4);
        // The chain view is empty at genesis, so no checkpoint can be on it.
        let post = process_attestations(&state, &[attestation(1, &[true])]).unwrap();
        assert_eq!(post.latest_justified, state.latest_justified);
        assert_eq!(post.latest_finalized, state.latest_finalized);
        assert!(post.justifications_roots.is_empty());
    }

    #[test]
    fn should_reject_an_aggregate_that_names_nobody_before_it_reaches_the_vote_table() {
        assert_eq!(
            super::voting_validator_indices(
                &AggregationBits::try_from(vec![false, false]).unwrap()
            ),
            Err(RejectionReason::EmptyAggregationBits)
        );
    }

    #[test]
    fn should_list_every_set_bit_in_ascending_order() {
        assert_eq!(
            super::voting_validator_indices(
                &AggregationBits::try_from(vec![false, true, false, true]).unwrap()
            ),
            Ok(vec![1, 3])
        );
    }
}
