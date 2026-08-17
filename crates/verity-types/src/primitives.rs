//! Scalar identifiers and byte-array aliases shared by every consensus container.

use libssz::{DecodeError, SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Node, Sha256Hasher};

/// A 32-byte root or hash, as SSZ `Bytes32`.
pub type Bytes32 = [u8; 32];

/// A 52-byte XMSS public key, as SSZ `Bytes52`.
pub type Bytes52 = [u8; 52];

/// The all-zero root. In a chain view indexed by slot it marks a slot with no block.
pub const ZERO_HASH: Bytes32 = [0u8; 32];

/// Defines a `Uint64` newtype with its three SSZ impls.
///
/// These are written out rather than derived. `#[ssz(transparent)]` produces correct encode,
/// decode, and `hash_tree_root`, but it does not forward `is_basic_type`, which defaults to
/// `false`. A basic type that reports itself composite merkleizes one element per chunk
/// instead of packing, so any SSZ list of such a newtype would produce a wrong root. Since
/// the mistake would be silent, the flag is set here explicitly.
macro_rules! uint64_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(value)
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl SszEncode for $name {
            fn is_fixed_size() -> bool {
                true
            }

            fn fixed_size() -> usize {
                8
            }

            fn encoded_len(&self) -> usize {
                8
            }

            fn ssz_append(&self, buf: &mut Vec<u8>) {
                self.0.ssz_append(buf);
            }
        }

        impl SszDecode for $name {
            fn is_fixed_size() -> bool {
                true
            }

            fn fixed_size() -> usize {
                8
            }

            fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
                u64::from_ssz_bytes(bytes).map(Self)
            }
        }

        impl HashTreeRoot for $name {
            fn hash_tree_root(&self, hasher: &impl Sha256Hasher) -> Node {
                self.0.hash_tree_root(hasher)
            }

            fn is_basic_type() -> bool {
                true
            }
        }
    };
}

uint64_newtype! {
    /// A slot number, counted from genesis.
    Slot
}

uint64_newtype! {
    /// A validator's position in the registry.
    ValidatorIndex
}

uint64_newtype! {
    /// Identifier of an attestation subnet.
    SubnetId
}

uint64_newtype! {
    /// An interval counter. A slot spans [`crate::config::INTERVALS_PER_SLOT`] intervals.
    Interval
}

#[cfg(test)]
mod tests {
    use super::{HashTreeRoot, Slot, SszDecode, SszEncode};
    use libssz_merkle::Sha2Hasher;

    #[test]
    fn should_encode_as_little_endian_u64_when_slot_is_serialized() {
        assert_eq!(Slot(1).to_ssz(), 1u64.to_le_bytes());
    }

    #[test]
    fn should_round_trip_when_slot_is_decoded_from_its_own_encoding() {
        let slot = Slot(u64::MAX);
        assert_eq!(Slot::from_ssz_bytes(&slot.to_ssz()).unwrap(), slot);
    }

    #[test]
    fn should_match_inner_u64_root_when_slot_root_is_computed() {
        assert_eq!(
            Slot(42).hash_tree_root(&Sha2Hasher),
            42u64.hash_tree_root(&Sha2Hasher)
        );
    }

    #[test]
    fn should_report_basic_type_when_asked_so_lists_pack_their_elements() {
        assert!(<Slot as HashTreeRoot>::is_basic_type());
    }
}
