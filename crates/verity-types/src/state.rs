//! The consensus state and the bounded collections it holds.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::{SszBitlist, SszList};

use crate::block::BlockHeader;
use crate::checkpoint::Checkpoint;
use crate::config::{HISTORICAL_ROOTS_LIMIT, JUSTIFICATION_VALIDATORS_LIMIT};
use crate::primitives::{Bytes32, Slot};
use crate::validator::Validators;

/// Chain view indexed by slot. A slot with no block carries [`crate::primitives::ZERO_HASH`].
pub type HistoricalBlockHashes = SszList<Bytes32, HISTORICAL_ROOTS_LIMIT>;

/// Block roots of the slots with justification votes in flight.
pub type JustificationRoots = SszList<Bytes32, HISTORICAL_ROOTS_LIMIT>;

/// Per-slot justification status, indexed relative to the finalized boundary.
pub type JustifiedSlots = SszBitlist<HISTORICAL_ROOTS_LIMIT>;

/// Per-slot, per-validator justification votes, flattened into one bitlist for SSZ.
pub type JustificationValidators = SszBitlist<JUSTIFICATION_VALIDATORS_LIMIT>;

/// Chain configuration committed into the consensus state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct GenesisConfig {
    /// The timestamp of the genesis block.
    pub genesis_time: u64,
}

/// The consensus state.
///
/// Field order is consensus-critical: it determines the hash tree root, and therefore the
/// `state_root` every block commits to. It matches leanSpec exactly and must never be
/// reordered for readability.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct State {
    /// Chain configuration.
    pub config: GenesisConfig,
    /// The slot this state is at.
    pub slot: Slot,
    /// Header of the most recent block applied.
    pub latest_block_header: BlockHeader,
    /// The highest justified checkpoint.
    pub latest_justified: Checkpoint,
    /// The highest finalized checkpoint.
    pub latest_finalized: Checkpoint,
    /// Chain view indexed by slot.
    pub historical_block_hashes: HistoricalBlockHashes,
    /// Justification status per tracked slot.
    pub justified_slots: JustifiedSlots,
    /// The validator registry.
    pub validators: Validators,
    /// Block roots of the slots with justification votes in flight.
    pub justifications_roots: JustificationRoots,
    /// Justification votes, flattened over slots and validators.
    pub justifications_validators: JustificationValidators,
}
