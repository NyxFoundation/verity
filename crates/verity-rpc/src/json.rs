//! The JSON bodies of the REST API, keyed exactly as leanSpec's `responses.py` keys them.
//!
//! Field names are snake_case and never aliased; roots are `0x`-prefixed lowercase hex,
//! slots and indices are plain integers, and an absent count is `null` rather than zero —
//! zero would read as "no validators" where the truth is "no state to count them in".

use serde::Serialize;
use verity_types::{Bytes32, Checkpoint};

/// A 32-byte root on the wire: `0x` followed by 64 lowercase hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hex32(pub Bytes32);

impl Serialize for Hex32 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("0x{}", hex::encode(self.0)))
    }
}

/// `GET /lean/v0/health`.
#[derive(Debug, Serialize)]
pub struct HealthBody {
    /// Always `healthy`: the process answered.
    pub status: &'static str,
    /// The service identifier leanSpec fixes.
    pub service: &'static str,
}

/// A checkpoint on the wire: its slot and block root.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CheckpointBody {
    /// The checkpoint's slot.
    pub slot: u64,
    /// The checkpoint's block root.
    pub root: Hex32,
}

impl From<Checkpoint> for CheckpointBody {
    fn from(checkpoint: Checkpoint) -> Self {
        Self {
            slot: checkpoint.slot.0,
            root: Hex32(checkpoint.root),
        }
    }
}

/// One block in the fork-choice tree.
#[derive(Debug, Serialize)]
pub struct ForkChoiceNode {
    /// The block's root.
    pub root: Hex32,
    /// The block's slot.
    pub slot: u64,
    /// The parent's root.
    pub parent_root: Hex32,
    /// Who proposed it.
    pub proposer_index: u64,
    /// The stake of the latest votes at or below it.
    pub weight: u64,
}

/// `GET /lean/v0/fork_choice`: a snapshot of the tree from the finalized slot up.
#[derive(Debug, Serialize)]
pub struct ForkChoiceBody {
    /// Every block at or above the finalized slot.
    pub nodes: Vec<ForkChoiceNode>,
    /// The head root.
    pub head: Hex32,
    /// The latest justified checkpoint.
    pub justified: CheckpointBody,
    /// The latest finalized checkpoint.
    pub finalized: CheckpointBody,
    /// The safe target root.
    pub safe_target: Hex32,
    /// The head state's registry size, or `null` when the head state is not in view.
    pub validator_count: Option<usize>,
}

/// `GET /lean/v0/admin/aggregator`.
#[derive(Debug, Serialize)]
pub struct AggregatorStatusBody {
    /// Whether the node runs the aggregation round.
    pub is_aggregator: bool,
}

/// `POST /lean/v0/admin/aggregator`: the new role and the one it replaced.
#[derive(Debug, Serialize)]
pub struct AggregatorToggleBody {
    /// The role now in effect.
    pub is_aggregator: bool,
    /// The role before this request.
    pub previous: bool,
}

#[cfg(test)]
mod tests {
    use verity_types::Slot;

    use super::*;

    #[test]
    fn should_render_roots_as_prefixed_lowercase_hex() {
        let body = CheckpointBody::from(Checkpoint {
            root: [0xab; 32],
            slot: Slot(7),
        });
        let text = serde_json::to_string(&body).expect("serializes");
        assert_eq!(
            text,
            "{\"slot\":7,\"root\":\"0xabababababababababababababababababababababababababababababababab\"}"
        );
    }

    #[test]
    fn should_render_an_absent_validator_count_as_null() {
        let body = ForkChoiceBody {
            nodes: Vec::new(),
            head: Hex32([0; 32]),
            justified: CheckpointBody::from(Checkpoint::default()),
            finalized: CheckpointBody::from(Checkpoint::default()),
            safe_target: Hex32([0; 32]),
            validator_count: None,
        };
        let text = serde_json::to_string(&body).expect("serializes");
        assert!(text.ends_with("\"validator_count\":null}"), "{text}");
    }
}
