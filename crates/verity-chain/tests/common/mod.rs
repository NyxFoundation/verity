//! Shared plumbing for the leanSpec fixture suites.
//!
//! Every suite is gated on `VERITY_FIXTURES` pointing at an extracted `fixtures-prod-scheme`
//! tree. The fast `cargo test` gate leaves it unset and the tests return; CI's fixtures job
//! always sets it, and each suite fails if no case matched.
//!
//! The JSON containers below are mirrored rather than derived on the `verity-types` shapes.
//! Consensus values travel as SSZ, never as JSON; the `{"data": [...]}` wrappers and
//! camelCase names are a test-generator convention, not part of any container's shape.
//! `deny_unknown_fields` is what keeps them honest — a field leanSpec adds fails the run
//! instead of being silently skipped.

// Each harness is its own test binary and compiles this module separately, so every one of
// them leaves some of it unused. The alternative is a per-item allow on almost every item.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use verity_types::{
    AggregatedAttestation, AggregatedAttestations, AggregationBits, AttestationData, Block,
    BlockBody, BlockHeader, Bytes32, Bytes52, Checkpoint, GenesisConfig, HistoricalBlockHashes,
    JustificationValidators, JustifiedSlots, Slot, State, Validator, ValidatorIndex, Validators,
};

/// The extracted fixture tree, when the environment points at one.
pub fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("VERITY_FIXTURES").map(PathBuf::from)
}

/// Every `*.json` under a directory named `suite`, anywhere in the tree.
///
/// Matching on the suite directory rather than a fixed depth keeps this working when leanSpec
/// moves a suite, which it has already done once.
pub fn collect_suite_json(root: &Path, suite: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, suite, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, suite: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, suite, out);
            continue;
        }
        let is_json = path.extension().is_some_and(|ext| ext == "json");
        let in_suite = path.components().any(|c| c.as_os_str() == suite);
        if is_json && in_suite {
            out.push(path);
        }
    }
}

/// Reads every case in a suite, keyed by the leanSpec test id that produced it.
pub fn read_cases<T: serde::de::DeserializeOwned>(paths: &[PathBuf]) -> Vec<(String, T)> {
    let mut cases = Vec::new();
    for path in paths {
        let text = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{}: read error: {error}", path.display()));
        let file: std::collections::BTreeMap<String, T> = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{}: json: {error}", path.display()));
        cases.extend(file);
    }
    cases
}

// ---------------------------------------------------------------------------------------
// Fixture shapes
// ---------------------------------------------------------------------------------------

/// leanSpec wraps every SSZ collection in a `data` key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataList<T> {
    pub data: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateJson {
    pub config: GenesisConfigJson,
    pub slot: u64,
    pub latest_block_header: BlockHeaderJson,
    pub latest_justified: CheckpointJson,
    pub latest_finalized: CheckpointJson,
    pub historical_block_hashes: DataList<String>,
    pub justified_slots: DataList<bool>,
    pub validators: DataList<ValidatorJson>,
    pub justifications_roots: DataList<String>,
    pub justifications_validators: DataList<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenesisConfigJson {
    pub genesis_time: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlockHeaderJson {
    pub slot: u64,
    pub proposer_index: u64,
    pub parent_root: String,
    pub state_root: String,
    pub body_root: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointJson {
    pub root: String,
    pub slot: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidatorJson {
    pub attestation_public_key: String,
    pub proposal_public_key: String,
    pub index: u64,
}

/// A block as a step or a case carries it.
///
/// `blockRootLabel` is the generator's symbolic name for the block — `"block_2b"` — and is
/// what the fork-choice checks refer to instead of a root. It is absent from the
/// state-transition vectors, hence the default.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlockJson {
    pub slot: u64,
    pub proposer_index: u64,
    pub parent_root: String,
    pub state_root: String,
    pub body: BlockBodyJson,
    #[serde(default)]
    pub block_root_label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockBodyJson {
    pub attestations: DataList<AggregatedAttestationJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AggregatedAttestationJson {
    pub aggregation_bits: DataList<bool>,
    pub data: AttestationDataJson,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationDataJson {
    pub slot: u64,
    pub head: CheckpointJson,
    pub target: CheckpointJson,
    pub source: CheckpointJson,
}

impl StateJson {
    pub fn build(&self) -> Result<State, String> {
        Ok(State {
            config: GenesisConfig {
                genesis_time: self.config.genesis_time,
            },
            slot: Slot(self.slot),
            latest_block_header: self.latest_block_header.build()?,
            latest_justified: self.latest_justified.build()?,
            latest_finalized: self.latest_finalized.build()?,
            historical_block_hashes: roots(&self.historical_block_hashes)?,
            justified_slots: bitlist::<JustifiedSlots>(&self.justified_slots.data)?,
            validators: validators(&self.validators)?,
            justifications_roots: roots(&self.justifications_roots)?,
            justifications_validators: bitlist::<JustificationValidators>(
                &self.justifications_validators.data,
            )?,
        })
    }
}

impl BlockHeaderJson {
    pub fn build(&self) -> Result<BlockHeader, String> {
        Ok(BlockHeader {
            slot: Slot(self.slot),
            proposer_index: ValidatorIndex(self.proposer_index),
            parent_root: bytes32(&self.parent_root)?,
            state_root: bytes32(&self.state_root)?,
            body_root: bytes32(&self.body_root)?,
        })
    }
}

impl CheckpointJson {
    pub fn build(&self) -> Result<Checkpoint, String> {
        Ok(Checkpoint {
            root: bytes32(&self.root)?,
            slot: Slot(self.slot),
        })
    }
}

impl BlockJson {
    pub fn build(&self) -> Result<Block, String> {
        let mut attestations = AggregatedAttestations::default();
        for attestation in &self.body.attestations.data {
            attestations
                .push(attestation.build()?)
                .map_err(|error| format!("attestations: {error:?}"))?;
        }
        Ok(Block {
            slot: Slot(self.slot),
            proposer_index: ValidatorIndex(self.proposer_index),
            parent_root: bytes32(&self.parent_root)?,
            state_root: bytes32(&self.state_root)?,
            body: BlockBody { attestations },
        })
    }
}

impl AggregatedAttestationJson {
    pub fn build(&self) -> Result<AggregatedAttestation, String> {
        Ok(AggregatedAttestation {
            aggregation_bits: bitlist::<AggregationBits>(&self.aggregation_bits.data)?,
            data: self.data.build()?,
        })
    }
}

impl AttestationDataJson {
    pub fn build(&self) -> Result<AttestationData, String> {
        Ok(AttestationData {
            slot: Slot(self.slot),
            head: self.head.build()?,
            target: self.target.build()?,
            source: self.source.build()?,
        })
    }
}

// ---------------------------------------------------------------------------------------
// Primitive conversions
// ---------------------------------------------------------------------------------------

/// Records a mismatch when a case asserts a value and it disagrees.
pub fn compare<T: PartialEq + std::fmt::Debug>(
    failures: &mut Vec<String>,
    name: &str,
    expected: Option<T>,
    actual: Option<T>,
) {
    let (Some(expected), Some(actual)) = (expected, actual) else {
        return;
    };
    if expected != actual {
        failures.push(format!("{name}: got {actual:?}, expected {expected:?}"));
    }
}

pub fn flags(length: usize, read: impl Fn(usize) -> Option<bool>) -> Vec<bool> {
    (0..length)
        .map(|index| read(index).unwrap_or(false))
        .collect()
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn unhex(text: &str) -> Result<Vec<u8>, String> {
    let body = text
        .strip_prefix("0x")
        .ok_or_else(|| format!("{text}: missing 0x prefix"))?;
    (0..body.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&body[index..index + 2], 16)
                .map_err(|error| format!("{text}: {error}"))
        })
        .collect()
}

pub fn bytes32(text: &str) -> Result<Bytes32, String> {
    unhex(text)?
        .try_into()
        .map_err(|_| format!("{text}: not 32 bytes"))
}

pub fn bytes52(text: &str) -> Result<Bytes52, String> {
    unhex(text)?
        .try_into()
        .map_err(|_| format!("{text}: not 52 bytes"))
}

pub fn roots(list: &DataList<String>) -> Result<HistoricalBlockHashes, String> {
    let mut out = HistoricalBlockHashes::default();
    for text in &list.data {
        out.push(bytes32(text)?)
            .map_err(|error| format!("roots: {error:?}"))?;
    }
    Ok(out)
}

pub fn validators(list: &DataList<ValidatorJson>) -> Result<Validators, String> {
    let mut out = Validators::default();
    for entry in &list.data {
        out.push(Validator {
            attestation_public_key: bytes52(&entry.attestation_public_key)?,
            proposal_public_key: bytes52(&entry.proposal_public_key)?,
            index: ValidatorIndex(entry.index),
        })
        .map_err(|error| format!("validators: {error:?}"))?;
    }
    Ok(out)
}

pub fn bitlist<T: TryFrom<Vec<bool>>>(bits: &[bool]) -> Result<T, String> {
    T::try_from(bits.to_vec())
        .map_err(|_| format!("bitlist of {} bits exceeds its limit", bits.len()))
}
