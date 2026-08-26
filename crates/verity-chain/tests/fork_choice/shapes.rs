//! The `fork_choice_test` vector, mirrored field for field.

use crate::common::{AttestationDataJson, BlockJson, DataList, StateJson};
use crate::fork_choice::checks::ChecksJson;
use crate::fork_choice::snapshot::StoreSnapshotJson;
use serde::Deserialize;

// ---------------------------------------------------------------------------------------
// Fixture shapes
// ---------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Case {
    #[allow(dead_code)]
    pub network: String,
    #[allow(dead_code)]
    pub lean_env: String,
    #[allow(dead_code)]
    pub proof_setting: u8,
    pub anchor_state: StateJson,
    pub anchor_block: BlockJson,
    pub steps: Vec<Step>,
    /// Set when building the store from the anchor is itself the thing expected to fail.
    #[serde(default)]
    pub rejection_reason: Option<String>,
    #[allow(dead_code)]
    pub max_slot: u64,
    #[serde(rename = "_info")]
    #[allow(dead_code)]
    pub info: serde_json::Value,
}

/// One event, in the flat shape the generator emits for every step type.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Step {
    pub step_type: String,
    pub valid: bool,
    pub store_snapshot: StoreSnapshotJson,
    pub checks: Option<ChecksJson>,
    pub rejection_reason: Option<String>,
    pub block: Option<BlockJson>,
    pub tick_to_slot: bool,
    pub time: Option<u64>,
    pub interval: Option<u64>,
    pub has_proposal: bool,
    pub attestation: Option<AttestationJson>,
    pub is_aggregator: bool,
}

impl Default for Step {
    fn default() -> Self {
        Self {
            step_type: String::new(),
            // Every emitted step carries `valid`; this only satisfies `default` above.
            valid: true,
            store_snapshot: StoreSnapshotJson::default(),
            checks: None,
            rejection_reason: None,
            block: None,
            tick_to_slot: false,
            time: None,
            interval: None,
            attestation: None,
            has_proposal: false,
            is_aggregator: false,
        }
    }
}

/// Both gossip shapes in one: a per-validator vote, and an aggregate over many.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct AttestationJson {
    pub data: AttestationDataJson,
    pub validator_index: Option<u64>,
    pub signature: Option<String>,
    pub proof: Option<ProofJson>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofJson {
    pub participants: DataList<bool>,
    pub proof: ProofBytesJson,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofBytesJson {
    pub data: String,
}
