//! The one place the hash tree root implementation is chosen.
//!
//! The default remains `libssz`. Enabling `lean-ssz` routes supported consensus containers
//! through the implementation vendored from `ethereum/ssz-specs` PR #132 when Lean was
//! available at build time. An unavailable build falls back to `libssz`; once linked, a Lean
//! rejection is fatal because silently changing consensus implementations would hide divergence.
//! Parity tests and the benchmark call the fallible Lean entry point directly. The FFI receives
//! canonical SSZ bytes because the PR exposes a data-driven `Desc`/`Value` API rather than
//! Rust-layout bindings.

use libssz_merkle::{HashTreeRoot, Sha2Hasher};
use verity_types::Bytes32;

#[cfg(feature = "lean-ssz")]
use std::any::Any;

#[cfg(feature = "lean-ssz")]
use libssz::SszEncode;
#[cfg(feature = "lean-ssz")]
use verity_consensus_sys::{HashTreeRootError, SszType};
#[cfg(feature = "lean-ssz")]
use verity_types::{AttestationData, Block, BlockBody, BlockHeader, State};

/// The native Rust implementation used in production and as the benchmark baseline.
#[must_use = "this computes the root; it does not store or commit to it"]
pub fn native_hash_tree_root<T: HashTreeRoot>(value: &T) -> Bytes32 {
    value.hash_tree_root(&Sha2Hasher)
}

/// The SSZ hash tree root of a consensus value.
#[cfg(not(feature = "lean-ssz"))]
#[must_use = "this computes the root; it does not store or commit to it"]
pub fn hash_tree_root<T: HashTreeRoot + 'static>(value: &T) -> Bytes32 {
    native_hash_tree_root(value)
}

/// The SSZ hash tree root of a consensus value.
///
/// Types outside the five experimental consensus containers continue to use `libssz`.
#[cfg(feature = "lean-ssz")]
#[must_use = "this computes the root; it does not store or commit to it"]
pub fn hash_tree_root<T: HashTreeRoot + 'static>(value: &T) -> Bytes32 {
    if !verity_consensus_sys::is_available() {
        return native_hash_tree_root(value);
    }
    let Some(result) = selected_lean_hash_tree_root(value) else {
        return native_hash_tree_root(value);
    };
    result.unwrap_or_else(|error| {
        panic!("canonical Verity value was rejected by the Lean SSZ adapter: {error}")
    })
}

#[cfg(feature = "lean-ssz")]
fn selected_lean_hash_tree_root<T: 'static>(
    value: &T,
) -> Option<Result<Bytes32, HashTreeRootError>> {
    let value = value as &dyn Any;
    macro_rules! select {
        ($type:ty) => {
            if let Some(value) = value.downcast_ref::<$type>() {
                return Some(lean_hash_tree_root(value));
            }
        };
    }
    select!(State);
    select!(Block);
    select!(BlockBody);
    select!(BlockHeader);
    select!(AttestationData);
    None
}

/// Computes the same root through the PR #132 implementation, including its pure Lean SHA-256.
#[cfg(feature = "lean-ssz")]
pub fn lean_hash_tree_root<T: LeanSszType>(value: &T) -> Result<Bytes32, HashTreeRootError> {
    verity_consensus_sys::hash_tree_root(T::SSZ_TYPE, &value.to_ssz())
}

/// Associates a supported Rust consensus container with its data-driven Lean declaration.
///
/// This trait is sealed so downstream code cannot accidentally pair another encoding with one of
/// the five Lean descriptors.
#[cfg(feature = "lean-ssz")]
pub trait LeanSszType: SszEncode + sealed::Sealed {
    const SSZ_TYPE: SszType;
}

#[cfg(feature = "lean-ssz")]
mod sealed {
    pub trait Sealed {}
}

#[cfg(feature = "lean-ssz")]
macro_rules! lean_ssz_types {
    ($($rust:ty => $lean:ident),+ $(,)?) => {
        $(
            impl sealed::Sealed for $rust {}

            impl LeanSszType for $rust {
                const SSZ_TYPE: SszType = SszType::$lean;
            }
        )+
    };
}

#[cfg(feature = "lean-ssz")]
lean_ssz_types! {
    State => State,
    Block => Block,
    BlockBody => BlockBody,
    BlockHeader => BlockHeader,
    AttestationData => AttestationData,
}

#[cfg(all(test, feature = "lean-ssz"))]
mod tests {
    use super::{hash_tree_root, lean_hash_tree_root, native_hash_tree_root};
    use verity_types::{
        AggregatedAttestation, AggregatedAttestations, AggregationBits, AttestationData, Block,
        BlockBody, BlockHeader, Checkpoint, GenesisConfig, HistoricalBlockHashes,
        JustificationRoots, JustificationValidators, JustifiedSlots, Slot, State, Validator,
        ValidatorIndex, Validators,
    };

    #[test]
    fn should_match_native_roots_for_empty_values_when_lean_backend_is_available() {
        if !verity_consensus_sys::is_available() {
            return;
        }
        assert_parity("BlockHeader", &BlockHeader::default());
        assert_parity("AttestationData", &AttestationData::default());
        assert_parity("BlockBody", &BlockBody::default());
        assert_parity("Block", &Block::default());
        assert_parity("State", &State::default());
    }

    #[test]
    fn should_keep_using_native_roots_for_types_outside_the_experiment() {
        let value = checkpoint(8, 6);
        assert_eq!(hash_tree_root(&value), native_hash_tree_root(&value));
    }

    #[test]
    fn should_match_native_roots_for_populated_values_when_lean_backend_is_available() {
        if !verity_consensus_sys::is_available() {
            return;
        }
        let header = populated_header();
        let data = populated_attestation_data();
        let body = populated_body(data);
        let block = Block {
            slot: header.slot,
            proposer_index: header.proposer_index,
            parent_root: header.parent_root,
            state_root: header.state_root,
            body: body.clone(),
        };
        assert_parity("BlockHeader", &header);
        assert_parity("AttestationData", &data);
        assert_parity("BlockBody", &body);
        assert_parity("Block", &block);
        assert_parity("State", &populated_state(header));
    }

    fn populated_header() -> BlockHeader {
        BlockHeader {
            slot: Slot(19),
            proposer_index: ValidatorIndex(7),
            parent_root: [1; 32],
            state_root: [2; 32],
            body_root: [3; 32],
        }
    }

    fn populated_attestation_data() -> AttestationData {
        AttestationData {
            slot: Slot(17),
            head: checkpoint(16, 4),
            target: checkpoint(12, 5),
            source: checkpoint(8, 6),
        }
    }

    fn populated_body(data: AttestationData) -> BlockBody {
        BlockBody {
            attestations: AggregatedAttestations::try_from(vec![AggregatedAttestation {
                aggregation_bits: AggregationBits::try_from(vec![true, false, true]).unwrap(),
                data,
            }])
            .unwrap(),
        }
    }

    fn populated_state(header: BlockHeader) -> State {
        State {
            config: GenesisConfig { genesis_time: 42 },
            slot: Slot(20),
            latest_block_header: header,
            latest_justified: checkpoint(16, 7),
            latest_finalized: checkpoint(12, 8),
            historical_block_hashes: HistoricalBlockHashes::try_from(vec![[9; 32], [10; 32]])
                .unwrap(),
            justified_slots: JustifiedSlots::try_from(vec![true, false, true]).unwrap(),
            validators: Validators::try_from(vec![Validator {
                attestation_public_key: [11; 52],
                proposal_public_key: [12; 52],
                index: ValidatorIndex(7),
            }])
            .unwrap(),
            justifications_roots: JustificationRoots::try_from(vec![[13; 32], [14; 32]]).unwrap(),
            justifications_validators: JustificationValidators::try_from(vec![false, true, true])
                .unwrap(),
        }
    }

    fn checkpoint(slot: u64, byte: u8) -> Checkpoint {
        Checkpoint {
            root: [byte; 32],
            slot: Slot(slot),
        }
    }

    fn assert_parity<T>(name: &str, value: &T)
    where
        T: super::LeanSszType + libssz_merkle::HashTreeRoot,
    {
        assert_eq!(
            lean_hash_tree_root(value)
                .unwrap_or_else(|error| panic!("Lean should accept canonical {name}: {error}")),
            native_hash_tree_root(value)
        );
    }
}
