//! Chain constants, transcribed from leanSpec `src/lean_spec/spec/forks/lstar/config.py`.
//!
//! The collection limits below are `usize` because they instantiate const-generic SSZ
//! containers, where the limit is part of the type and therefore part of the hash tree root.
//! Changing one changes merkle depth, hence the state root — they are consensus-critical.

/// Number of intervals a slot is divided into.
pub const INTERVALS_PER_SLOT: u64 = 5;

/// Future-slot tolerance for gossip attestations, in intervals.
///
/// Bounds the clock skew the time check absorbs when admitting a vote whose slot has not yet
/// started locally.
pub const GOSSIP_DISPARITY_INTERVALS: u64 = 1;

/// Wall-clock duration of one slot, in seconds.
pub const SECONDS_PER_SLOT: u64 = 4;

/// Wall-clock duration of one slot, in milliseconds.
pub const MILLISECONDS_PER_SLOT: u64 = SECONDS_PER_SLOT * 1000;

/// Wall-clock duration of one interval, in milliseconds.
pub const MILLISECONDS_PER_INTERVAL: u64 = MILLISECONDS_PER_SLOT / INTERVALS_PER_SLOT;

/// How far back justification is tracked, in slots.
pub const JUSTIFICATION_LOOKBACK_SLOTS: u64 = 3;

/// Maximum number of historical block roots held in the state.
///
/// At a 4-second slot this is roughly 12.1 days of history.
pub const HISTORICAL_ROOTS_LIMIT: usize = 1 << 18;

/// Number of attestation committees.
pub const ATTESTATION_COMMITTEE_COUNT: u64 = 1;

/// Maximum number of validators in the registry.
pub const VALIDATOR_REGISTRY_LIMIT: usize = 1 << 12;

/// Maximum number of distinct attestation data entries.
pub const MAX_ATTESTATIONS_DATA: u8 = 8;

/// Capacity of the flattened per-slot, per-validator justification bitlist.
///
/// The state stores justification votes as one flat bitlist rather than a list of lists, so
/// its limit is the product of the two dimensions it flattens.
pub const JUSTIFICATION_VALIDATORS_LIMIT: usize = HISTORICAL_ROOTS_LIMIT * VALIDATOR_REGISTRY_LIMIT;

/// Maximum length in bytes of a serialized aggregation proof.
pub const BYTE_LIST_512_KIB_LIMIT: usize = 512 * 1024;
