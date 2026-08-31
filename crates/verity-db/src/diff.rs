//! The per-block state delta, and the snapshot rule that bounds how many of them a replay
//! ever walks.
//!
//! A full state per block is not affordable. `process_slots` appends to
//! `historical_block_hashes` every slot and the lstar state never trims it, so a state is
//! roughly `300 B + 32 B × slot`: about 691 KB at one day of slots and 8.4 MB at
//! `HISTORICAL_ROOTS_LIMIT`. Writing one per block would grow quadratically — around 7.5 GB
//! over the first day alone.
//!
//! So a full snapshot is written at anchors and at 1,024-slot boundaries, and every other
//! processed block writes a [`StateDiff`]. The diff carries only the fields a block can move
//! that nothing else can rederive: the snapshot supplies `config` and the validator registry,
//! and `reconstruct` derives the latest header and the historical roots from the stored
//! blocks and the parent link.
//!
//! Transcribed from `docs/design/storage.md`, "State storage: snapshots and diffs".

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use verity_types::primitives::{Bytes32, Slot};
use verity_types::state::{JustificationRoots, JustificationValidators, JustifiedSlots};
use verity_types::{Checkpoint, State};

/// How often a full snapshot is written, in slots.
///
/// A block writes a snapshot when its edge from its parent crosses a multiple of this. The
/// rule is stated on the *edge*, not on the block's own slot, so a run of empty slots cannot
/// skip a boundary and lengthen a replay past 1,023 diffs.
pub const SNAPSHOT_INTERVAL_SLOTS: u64 = 1_024;

/// The fields of a state that a block moves and nothing else can rederive.
///
/// Field order is the stored format. It follows `docs/design/storage.md` exactly; reordering
/// it makes every existing `state_diffs` row decode into a different state, which is why the
/// stored-type manifest in [`crate::schema`] hashes this shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct StateDiff {
    /// Root of the block this diff is applied on top of — the child's parent.
    pub base_block_root: Bytes32,
    /// The slot of the state this diff produces.
    pub slot: Slot,
    /// The highest justified checkpoint after the block.
    pub latest_justified: Checkpoint,
    /// The highest finalized checkpoint after the block.
    pub latest_finalized: Checkpoint,
    /// Justification status per tracked slot after the block.
    pub justified_slots: JustifiedSlots,
    /// Block roots of the slots with justification votes in flight.
    pub justifications_roots: JustificationRoots,
    /// Justification votes, flattened over slots and validators.
    pub justifications_validators: JustificationValidators,
}

impl StateDiff {
    /// The diff that reproduces `post` from the state of its parent block.
    ///
    /// This drops most of a state rather than converting one, which is why it is a named
    /// constructor and not a `From` implementation: `config`, the validator registry, the
    /// latest header, and the historical roots are all left behind for the snapshot and for
    /// `reconstruct` to supply.
    ///
    /// The base is taken from `post.latest_block_header.parent_root` rather than being passed
    /// in, so a diff cannot be built claiming a base the state does not actually descend from.
    #[must_use = "this builds the row; it does not store it"]
    pub fn from_post_state(post: &State) -> Self {
        Self {
            base_block_root: post.latest_block_header.parent_root,
            slot: post.slot,
            latest_justified: post.latest_justified,
            latest_finalized: post.latest_finalized,
            justified_slots: post.justified_slots.clone(),
            justifications_roots: post.justifications_roots.clone(),
            justifications_validators: post.justifications_validators.clone(),
        }
    }
}

/// Whether the edge from `parent_slot` to `block_slot` crosses a snapshot boundary.
///
/// # Panics
///
/// Never. A `block_slot` at or below `parent_slot` cannot cross a boundary and returns
/// `false`; the state transition has already refused such a block long before this is asked.
#[must_use]
pub fn crosses_snapshot_boundary(parent_slot: Slot, block_slot: Slot) -> bool {
    parent_slot.0 / SNAPSHOT_INTERVAL_SLOTS < block_slot.0 / SNAPSHOT_INTERVAL_SLOTS
}

#[cfg(test)]
mod tests {
    use libssz::{SszDecode, SszEncode};
    use verity_types::primitives::Slot;

    use super::{SNAPSHOT_INTERVAL_SLOTS, StateDiff, crosses_snapshot_boundary};

    #[test]
    fn should_take_a_snapshot_when_the_edge_crosses_a_boundary() {
        assert!(crosses_snapshot_boundary(
            Slot(SNAPSHOT_INTERVAL_SLOTS - 1),
            Slot(SNAPSHOT_INTERVAL_SLOTS)
        ));
    }

    #[test]
    fn should_not_take_a_snapshot_inside_one_interval() {
        assert!(!crosses_snapshot_boundary(Slot(1), Slot(1_023)));
    }

    #[test]
    fn should_take_a_snapshot_when_empty_slots_jump_over_a_boundary() {
        assert!(
            crosses_snapshot_boundary(Slot(1_000), Slot(3_000)),
            "the rule is on the edge, so a gap cannot skip a boundary"
        );
    }

    #[test]
    fn should_round_trip_a_diff_through_its_stored_encoding() {
        let diff = StateDiff {
            base_block_root: [7u8; 32],
            slot: Slot(9),
            ..StateDiff::default()
        };
        assert_eq!(StateDiff::from_ssz_bytes(&diff.to_ssz()).unwrap(), diff);
    }
}
