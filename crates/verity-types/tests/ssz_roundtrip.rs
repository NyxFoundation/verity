//! Property tests over the SSZ codec for every consensus container.
//!
//! Two properties are checked per container:
//!
//! - **Round trip** — `decode(encode(x)) == x`. This is what pins the field order and the
//!   variable-length offsets; a container whose fields are transposed still encodes and
//!   decodes, but not back to the same value once the field types differ.
//! - **Length agreement** — `encoded_len(x) == encode(x).len()`. `encoded_len` is what
//!   callers size buffers with and what enclosing containers use to place offsets, so a
//!   disagreement corrupts the parent's encoding rather than this one's.
//!
//! Conformance against leanSpec's own vectors is a separate concern and lands with the
//! fixture harness; these properties hold with no network access and cover shapes the
//! fixtures happen not to contain, such as empty lists and single-bit bitlists.

use libssz::{SszDecode, SszEncode};
use libssz_types::{SszBitlist, SszList};
use proptest::prelude::*;
use verity_types::aggregation::{MultiMessageAggregate, SingleMessageAggregate};
use verity_types::attestation::{
    AggregatedAttestation, AggregatedAttestations, Attestation, SignedAggregatedAttestation,
};
use verity_types::block::{Block, BlockBody, BlockHeader, SignedBlock};
use verity_types::checkpoint::{AttestationData, Checkpoint};
use verity_types::primitives::{Bytes32, Bytes52, Interval, Slot, SubnetId, ValidatorIndex};
use verity_types::state::{
    GenesisConfig, HistoricalBlockHashes, JustificationRoots, JustificationValidators,
    JustifiedSlots, State,
};
use verity_types::validator::{Validator, Validators};

/// Upper bound on generated collection lengths.
///
/// The real limits are up to 2^30 entries, which no test can materialize. Coverage of the
/// limit itself belongs to the merkleization check, not to the codec: the codec treats an
/// element the same at position 3 and at position 2^29.
const MAX_ELEMENTS: usize = 4;

fn arb_bytes<const N: usize>() -> impl Strategy<Value = [u8; N]> {
    proptest::collection::vec(any::<u8>(), N).prop_map(|bytes| {
        <[u8; N]>::try_from(bytes.as_slice()).expect("vector was generated at length N")
    })
}

fn arb_bytes32() -> impl Strategy<Value = Bytes32> {
    arb_bytes::<32>()
}

fn arb_bytes52() -> impl Strategy<Value = Bytes52> {
    arb_bytes::<52>()
}

fn arb_slot() -> impl Strategy<Value = Slot> {
    any::<u64>().prop_map(Slot)
}

fn arb_validator_index() -> impl Strategy<Value = ValidatorIndex> {
    any::<u64>().prop_map(ValidatorIndex)
}

fn arb_bitlist<const N: usize>() -> impl Strategy<Value = SszBitlist<N>> {
    proptest::collection::vec(any::<bool>(), 0..=64)
        .prop_map(|bits| SszBitlist::try_from(bits).expect("64 bits fit every consensus limit"))
}

fn arb_byte_list<const N: usize>() -> impl Strategy<Value = SszList<u8, N>> {
    proptest::collection::vec(any::<u8>(), 0..=128)
        .prop_map(|bytes| SszList::try_from(bytes).expect("128 bytes fit every consensus limit"))
}

fn arb_justified_slots() -> impl Strategy<Value = JustifiedSlots> {
    arb_bitlist()
}

fn arb_justification_validators() -> impl Strategy<Value = JustificationValidators> {
    arb_bitlist()
}

fn arb_checkpoint() -> impl Strategy<Value = Checkpoint> {
    (arb_bytes32(), arb_slot()).prop_map(|(root, slot)| Checkpoint { root, slot })
}

fn arb_attestation_data() -> impl Strategy<Value = AttestationData> {
    (
        arb_slot(),
        arb_checkpoint(),
        arb_checkpoint(),
        arb_checkpoint(),
    )
        .prop_map(|(slot, head, target, source)| AttestationData {
            slot,
            head,
            target,
            source,
        })
}

fn arb_single_message_aggregate() -> impl Strategy<Value = SingleMessageAggregate> {
    (arb_bitlist(), arb_byte_list()).prop_map(|(participants, proof)| SingleMessageAggregate {
        participants,
        proof,
    })
}

fn arb_multi_message_aggregate() -> impl Strategy<Value = MultiMessageAggregate> {
    arb_byte_list().prop_map(|proof| MultiMessageAggregate { proof })
}

fn arb_aggregated_attestation() -> impl Strategy<Value = AggregatedAttestation> {
    (arb_bitlist(), arb_attestation_data()).prop_map(|(aggregation_bits, data)| {
        AggregatedAttestation {
            aggregation_bits,
            data,
        }
    })
}

fn arb_validator() -> impl Strategy<Value = Validator> {
    (arb_bytes52(), arb_bytes52(), arb_validator_index()).prop_map(
        |(attestation_public_key, proposal_public_key, index)| Validator {
            attestation_public_key,
            proposal_public_key,
            index,
        },
    )
}

fn arb_block_header() -> impl Strategy<Value = BlockHeader> {
    (
        arb_slot(),
        arb_validator_index(),
        arb_bytes32(),
        arb_bytes32(),
        arb_bytes32(),
    )
        .prop_map(
            |(slot, proposer_index, parent_root, state_root, body_root)| BlockHeader {
                slot,
                proposer_index,
                parent_root,
                state_root,
                body_root,
            },
        )
}

fn arb_block_body() -> impl Strategy<Value = BlockBody> {
    proptest::collection::vec(arb_aggregated_attestation(), 0..=MAX_ELEMENTS).prop_map(
        |attestations| BlockBody {
            attestations: AggregatedAttestations::try_from(attestations)
                .expect("generated length is below the registry limit"),
        },
    )
}

fn arb_block() -> impl Strategy<Value = Block> {
    (
        arb_slot(),
        arb_validator_index(),
        arb_bytes32(),
        arb_bytes32(),
        arb_block_body(),
    )
        .prop_map(
            |(slot, proposer_index, parent_root, state_root, body)| Block {
                slot,
                proposer_index,
                parent_root,
                state_root,
                body,
            },
        )
}

fn arb_state() -> impl Strategy<Value = State> {
    (
        any::<u64>(),
        arb_slot(),
        arb_block_header(),
        arb_checkpoint(),
        arb_checkpoint(),
        proptest::collection::vec(arb_bytes32(), 0..=MAX_ELEMENTS),
        arb_justified_slots(),
        proptest::collection::vec(arb_validator(), 0..=MAX_ELEMENTS),
        proptest::collection::vec(arb_bytes32(), 0..=MAX_ELEMENTS),
        arb_justification_validators(),
    )
        .prop_map(
            |(
                genesis_time,
                slot,
                latest_block_header,
                latest_justified,
                latest_finalized,
                historical_block_hashes,
                justified_slots,
                validators,
                justifications_roots,
                justifications_validators,
            )| State {
                config: GenesisConfig { genesis_time },
                slot,
                latest_block_header,
                latest_justified,
                latest_finalized,
                historical_block_hashes: HistoricalBlockHashes::try_from(historical_block_hashes)
                    .expect("generated length is below the historical roots limit"),
                justified_slots,
                validators: Validators::try_from(validators)
                    .expect("generated length is below the registry limit"),
                justifications_roots: JustificationRoots::try_from(justifications_roots)
                    .expect("generated length is below the historical roots limit"),
                justifications_validators,
            },
        )
}

/// Asserts both codec properties for one value.
fn assert_codec<T>(value: &T) -> Result<(), TestCaseError>
where
    T: SszEncode + SszDecode + PartialEq + core::fmt::Debug,
{
    let encoded = value.to_ssz();
    prop_assert_eq!(
        value.encoded_len(),
        encoded.len(),
        "encoded_len disagrees with the actual encoding"
    );
    prop_assert_eq!(&T::from_ssz_bytes(&encoded).unwrap(), value);
    Ok(())
}

proptest! {
    #[test]
    fn should_round_trip_when_a_slot_is_encoded(value in arb_slot()) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_validator_index_is_encoded(value in arb_validator_index()) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_subnet_id_is_encoded(value in any::<u64>().prop_map(SubnetId)) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_an_interval_is_encoded(value in any::<u64>().prop_map(Interval)) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_checkpoint_is_encoded(value in arb_checkpoint()) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_attestation_data_is_encoded(value in arb_attestation_data()) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_an_attestation_is_encoded(
        value in (arb_validator_index(), arb_attestation_data())
            .prop_map(|(validator_index, data)| Attestation { validator_index, data })
    ) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_an_aggregated_attestation_is_encoded(
        value in arb_aggregated_attestation()
    ) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_signed_aggregated_attestation_is_encoded(
        value in (arb_attestation_data(), arb_single_message_aggregate())
            .prop_map(|(data, proof)| SignedAggregatedAttestation { data, proof })
    ) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_single_message_aggregate_is_encoded(
        value in arb_single_message_aggregate()
    ) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_multi_message_aggregate_is_encoded(
        value in arb_multi_message_aggregate()
    ) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_validator_is_encoded(value in arb_validator()) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_block_header_is_encoded(value in arb_block_header()) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_block_body_is_encoded(value in arb_block_body()) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_block_is_encoded(value in arb_block()) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_signed_block_is_encoded(
        value in (arb_block(), arb_multi_message_aggregate())
            .prop_map(|(block, proof)| SignedBlock { block, proof })
    ) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_genesis_config_is_encoded(
        value in any::<u64>().prop_map(|genesis_time| GenesisConfig { genesis_time })
    ) {
        assert_codec(&value)?;
    }

    #[test]
    fn should_round_trip_when_a_state_is_encoded(value in arb_state()) {
        assert_codec(&value)?;
    }
}
