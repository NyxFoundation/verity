//! The XMSS containers as they appear on the consensus wire.
//!
//! # Why these are re-declared here rather than reused from leanSig
//!
//! leanSig already serializes its own key and signature types, through `ethereum_ssz`.
//! Consensus containers in Verity go through `libssz`, and the two libraries do not share a
//! trait, so a `libssz` container cannot hold a leanSig type as a field. Since
//! [`SignedAttestation`] is a consensus container with a signature inside it, the signature
//! has to exist in `libssz` terms.
//!
//! The encodings agree byte for byte — both implement the same SSZ specification over the
//! same field order — which is what makes [`crate::Signature::to_leansig`] and its inverse a
//! re-parse rather than a translation. `hash_tree_root` is the reason the shape has to be
//! modelled at all: a signature carried as opaque bytes would produce the wrong root for
//! every container above it.
//!
//! Transcribed from leanSpec `src/lean_spec/spec/crypto/xmss/`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`.

use libssz::{ContainerDecoder, ContainerEncoder, DecodeError, SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Node, Sha256Hasher};
use libssz_types::{SszList, SszVector};
use verity_types::{AttestationData, ValidatorIndex};

use crate::scheme::{
    DIMENSION, EPOCHS_PER_BOTTOM_TREE, HASH_LENGTH_FIELD_ELEMENTS, LOG_LIFETIME, PARAMETER_LENGTH,
    RAND_LENGTH_FIELD_ELEMENTS, SIGNATURE_BYTES,
};

/// The KoalaBear prime, `2^31 - 2^24 + 1`.
pub const FIELD_MODULUS: u32 = 2_130_706_433;

/// Maximum nodes a sparse Merkle layer can hold.
///
/// The widest layer is a bottom tree's leaf row; padding adds at most one sibling at each
/// end. Twice the leaf count absorbs that with room to spare. This limit is not cosmetic:
/// it fixes the depth an SSZ list merkleizes to, so a different value silently changes every
/// root computed below it.
pub const NODE_LIST_LIMIT: usize = 2 * EPOCHS_PER_BOTTOM_TREE as usize;

/// An element of the KoalaBear field, serialized as a little-endian `uint32`.
///
/// A basic type, so SSZ packs runs of these back to back instead of giving each one a chunk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fp(pub u32);

impl TryFrom<u32> for Fp {
    type Error = DecodeError;

    /// Accepts only the canonical residue. Two encodings of one field element would give one
    /// signature two roots, so a value at or above the modulus is rejected rather than
    /// reduced — this is leanSpec's `Fp.deserialize` rule.
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value >= FIELD_MODULUS {
            // `libssz`'s `DecodeError` has no value-range variant, so the bound and the
            // offending value travel in `InvalidByteLength`'s two fields: `expected` is the
            // modulus, `got` is the value that reached or passed it.
            return Err(DecodeError::InvalidByteLength {
                expected: FIELD_MODULUS as usize,
                got: value as usize,
            });
        }
        Ok(Self(value))
    }
}

impl SszEncode for Fp {
    fn is_fixed_size() -> bool {
        true
    }

    fn fixed_size() -> usize {
        4
    }

    fn encoded_len(&self) -> usize {
        4
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.0.ssz_append(buf);
    }
}

impl SszDecode for Fp {
    fn is_fixed_size() -> bool {
        true
    }

    fn fixed_size() -> usize {
        4
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        Self::try_from(u32::from_ssz_bytes(bytes)?)
    }
}

impl HashTreeRoot for Fp {
    fn hash_tree_root(&self, hasher: &impl Sha256Hasher) -> Node {
        self.0.hash_tree_root(hasher)
    }

    fn is_basic_type() -> bool {
        true
    }
}

/// One Poseidon digest, as a fixed run of field elements.
pub type HashDigest = SszVector<Fp, HASH_LENGTH_FIELD_ELEMENTS>;

/// A variable-length run of digests: an authentication path, or one sparse Merkle layer.
pub type HashDigestList = SszList<HashDigest, NODE_LIST_LIMIT>;

/// A key's public personalization tag, mixed into every hash.
pub type Parameter = SszVector<Fp, PARAMETER_LENGTH>;

/// The randomness that made a message encode to a valid codeword.
pub type Randomness = SszVector<Fp, RAND_LENGTH_FIELD_ELEMENTS>;

/// A Merkle authentication path from one leaf up to the root.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct HashTreeOpening {
    /// Sibling hashes, ordered from the leaf upward.
    pub siblings: HashDigestList,
}

/// One horizontal slice of a sparse Merkle subtree.
///
/// Secret-key material: this never crosses the network. It is modelled because leanSpec
/// ships SSZ vectors for it, and a container Verity cannot decode is a container Verity
/// cannot check itself against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct HashTreeLayer {
    /// Absolute index of the first stored node within its level.
    pub start_index: u64,
    /// Stored digests for this layer, left to right.
    pub nodes: HashDigestList,
}

/// The long-lived public half of an XMSS key pair.
///
/// Fixed-size at 52 bytes, which is the `Bytes52` the validator registry stores twice per
/// validator.
#[derive(Debug, Clone, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
pub struct PublicKey {
    /// Merkle root over every one-time public key in the lifetime.
    pub root: HashDigest,
    /// Public personalization tag mixed into every hash.
    pub parameter: Parameter,
}

/// The three fields of a signature, in leanSpec's order.
///
/// Split out from [`Signature`] so the derive can produce the container encoding while
/// [`Signature`] overrides only what SSZ is told about its *size*. See that type's docs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, SszEncode, SszDecode, HashTreeRoot)]
struct SignatureFields {
    path: HashTreeOpening,
    rho: Randomness,
    hashes: HashDigestList,
}

/// One XMSS signature, for one slot and message under one public key.
///
/// # Fixed size, despite two variable-length fields
///
/// `path.siblings` and `hashes` are lists at the type level, but a valid signature pins both
/// to scheme constants — one sibling per tree level, one released hash per chain — so every
/// valid signature encodes to exactly [`SIGNATURE_BYTES`]. leanSpec therefore *declares* the
/// container fixed-size, which is what lets [`SignedAttestation`] carry it inline with no
/// offset of its own.
///
/// The declaration only holds if decoding enforces the two lengths, because the inherited
/// container decoder reads each list through an attacker-controlled offset: without the
/// check, distinct byte strings of the same total length would decode to signatures whose
/// lists hold the wrong number of digests. [`Signature::from_ssz_bytes`] enforces them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, HashTreeRoot)]
pub struct Signature {
    /// Authentication path from the one-time key up to the Merkle root.
    pub path: HashTreeOpening,
    /// Randomness that encoded the message to a valid codeword.
    pub rho: Randomness,
    /// Released Winternitz chain hashes forming the one-time signature.
    pub hashes: HashDigestList,
}

impl Signature {
    fn as_fields(&self) -> SignatureFields {
        SignatureFields {
            path: self.path.clone(),
            rho: self.rho.clone(),
            hashes: self.hashes.clone(),
        }
    }
}

impl SszEncode for Signature {
    fn is_fixed_size() -> bool {
        true
    }

    fn fixed_size() -> usize {
        SIGNATURE_BYTES
    }

    fn encoded_len(&self) -> usize {
        self.as_fields().encoded_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.as_fields().ssz_append(buf);
    }
}

impl SszDecode for Signature {
    fn is_fixed_size() -> bool {
        true
    }

    fn fixed_size() -> usize {
        SIGNATURE_BYTES
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let fields = SignatureFields::from_ssz_bytes(bytes)?;

        // Both checks report the *byte* length the field should have had against the one it
        // did, since that is what `DecodeError` can carry. A digest is
        // `HASH_LENGTH_FIELD_ELEMENTS` field elements of four bytes each.
        const DIGEST_BYTES: usize = HASH_LENGTH_FIELD_ELEMENTS * 4;

        let siblings = fields.path.siblings.len();
        if siblings != LOG_LIFETIME {
            return Err(DecodeError::InvalidByteLength {
                expected: LOG_LIFETIME * DIGEST_BYTES,
                got: siblings * DIGEST_BYTES,
            });
        }

        let hashes = fields.hashes.len();
        if hashes != DIMENSION {
            return Err(DecodeError::InvalidByteLength {
                expected: DIMENSION * DIGEST_BYTES,
                got: hashes * DIGEST_BYTES,
            });
        }

        Ok(Self {
            path: fields.path,
            rho: fields.rho,
            hashes: fields.hashes,
        })
    }
}

/// One validator's vote carrying its own raw XMSS signature.
///
/// The gossip form: this is what arrives on the attestation subnets, before an aggregator
/// folds the signatures into a proof. leanSpec derives it from `Attestation`, so the two
/// vote fields come first, in that order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, HashTreeRoot)]
pub struct SignedAttestation {
    /// The index of the validator making the attestation.
    pub validator_index: ValidatorIndex,
    /// The attestation data produced by the validator.
    pub data: AttestationData,
    /// The validator's signature over `hash_tree_root(data)`.
    pub signature: Signature,
}

impl SszEncode for SignedAttestation {
    fn is_fixed_size() -> bool {
        true
    }

    fn fixed_size() -> usize {
        <ValidatorIndex as SszEncode>::fixed_size()
            + <AttestationData as SszEncode>::fixed_size()
            + <Signature as SszEncode>::fixed_size()
    }

    fn encoded_len(&self) -> usize {
        <Self as SszEncode>::fixed_size()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        let mut encoder = ContainerEncoder::new(buf, <Self as SszEncode>::fixed_size());
        encoder.append_fixed(&self.validator_index);
        encoder.append_fixed(&self.data);
        encoder.append_fixed(&self.signature);
        encoder.finalize();
    }
}

impl SszDecode for SignedAttestation {
    fn is_fixed_size() -> bool {
        true
    }

    fn fixed_size() -> usize {
        <ValidatorIndex as SszDecode>::fixed_size()
            + <AttestationData as SszDecode>::fixed_size()
            + <Signature as SszDecode>::fixed_size()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut decoder = ContainerDecoder::new(bytes, <Self as SszDecode>::fixed_size())?;
        let validator_index = decoder.decode_fixed()?;
        let data = decoder.decode_fixed()?;
        let signature = decoder.decode_fixed()?;
        decoder.finish_fixed()?;
        Ok(Self {
            validator_index,
            data,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use libssz::{DecodeError, SszDecode, SszEncode};
    use libssz_merkle::{HashTreeRoot, Sha2Hasher};
    use verity_types::{AttestationData, Checkpoint, Slot, ValidatorIndex};

    use super::{
        DIMENSION, FIELD_MODULUS, Fp, HashDigest, HashDigestList, HashTreeLayer, HashTreeOpening,
        LOG_LIFETIME, Parameter, PublicKey, Randomness, SIGNATURE_BYTES, Signature,
        SignatureFields, SignedAttestation,
    };

    /// A digest whose elements are distinct, so a transposition would change the encoding.
    fn digest(seed: u32) -> HashDigest {
        HashDigest::try_from((0..8).map(|i| Fp(seed + i)).collect::<Vec<_>>()).unwrap()
    }

    fn digests(count: usize, seed: u32) -> HashDigestList {
        HashDigestList::try_from(
            (0..count)
                .map(|i| digest(seed + i as u32 * 8))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn signature_fields(siblings: usize, hashes: usize) -> SignatureFields {
        SignatureFields {
            path: HashTreeOpening {
                siblings: digests(siblings, 1),
            },
            rho: Randomness::try_from((0..7).map(Fp).collect::<Vec<_>>()).unwrap(),
            hashes: digests(hashes, 5_000),
        }
    }

    fn signature() -> Signature {
        let fields = signature_fields(LOG_LIFETIME, DIMENSION);
        Signature {
            path: fields.path,
            rho: fields.rho,
            hashes: fields.hashes,
        }
    }

    fn public_key() -> PublicKey {
        PublicKey {
            root: digest(11),
            parameter: Parameter::try_from((100..105).map(Fp).collect::<Vec<_>>()).unwrap(),
        }
    }

    fn attestation_data() -> AttestationData {
        AttestationData {
            slot: Slot(7),
            head: Checkpoint {
                root: [1u8; 32],
                slot: Slot(6),
            },
            target: Checkpoint {
                root: [2u8; 32],
                slot: Slot(4),
            },
            source: Checkpoint {
                root: [3u8; 32],
                slot: Slot(2),
            },
        }
    }

    #[test]
    fn should_reject_a_field_element_at_or_above_the_modulus_when_decoding() {
        assert!(Fp::from_ssz_bytes(&(FIELD_MODULUS - 1).to_le_bytes()).is_ok());
        assert!(Fp::from_ssz_bytes(&FIELD_MODULUS.to_le_bytes()).is_err());
        assert!(Fp::from_ssz_bytes(&u32::MAX.to_le_bytes()).is_err());
    }

    #[test]
    fn should_pack_field_elements_when_a_run_of_them_is_merkleized() {
        assert!(<Fp as HashTreeRoot>::is_basic_type());
    }

    #[test]
    fn should_encode_to_the_constant_length_when_a_valid_signature_is_serialized() {
        let encoded = signature().to_ssz();
        assert_eq!(encoded.len(), SIGNATURE_BYTES);
        assert_eq!(<Signature as SszEncode>::fixed_size(), SIGNATURE_BYTES);
        assert!(<Signature as SszEncode>::is_fixed_size());
    }

    #[test]
    fn should_round_trip_when_a_signature_is_decoded_from_its_own_encoding() {
        let signature = signature();
        assert_eq!(
            Signature::from_ssz_bytes(&signature.to_ssz()).unwrap(),
            signature
        );
    }

    #[test]
    fn should_refuse_when_a_signature_carries_the_wrong_number_of_path_siblings() {
        let encoded = signature_fields(LOG_LIFETIME - 1, DIMENSION).to_ssz();
        assert!(matches!(
            Signature::from_ssz_bytes(&encoded),
            Err(DecodeError::InvalidByteLength { .. })
        ));
    }

    #[test]
    fn should_refuse_when_a_signature_carries_the_wrong_number_of_released_hashes() {
        let encoded = signature_fields(LOG_LIFETIME, DIMENSION + 1).to_ssz();
        assert!(matches!(
            Signature::from_ssz_bytes(&encoded),
            Err(DecodeError::InvalidByteLength { .. })
        ));
    }

    /// The two wrong lengths that cancel out. Total encoded length stays exactly
    /// `SIGNATURE_BYTES`, so a decoder trusting the declared size alone would accept it.
    #[test]
    fn should_refuse_when_two_wrong_list_lengths_leave_the_total_size_unchanged() {
        let encoded = signature_fields(LOG_LIFETIME + 1, DIMENSION - 1).to_ssz();
        assert_eq!(encoded.len(), SIGNATURE_BYTES);
        assert!(Signature::from_ssz_bytes(&encoded).is_err());
    }

    #[test]
    fn should_round_trip_through_the_registry_form_when_a_public_key_is_stored() {
        let key = public_key();
        assert_eq!(key.to_bytes52().len(), 52);
        assert_eq!(PublicKey::from_bytes52(&key.to_bytes52()).unwrap(), key);
    }

    #[test]
    fn should_carry_the_signature_inline_when_a_signed_attestation_is_encoded() {
        let signed = SignedAttestation {
            validator_index: ValidatorIndex(3),
            data: attestation_data(),
            signature: signature(),
        };

        let encoded = signed.to_ssz();
        assert!(<SignedAttestation as SszEncode>::is_fixed_size());
        assert_eq!(
            encoded.len(),
            8 + <AttestationData as SszEncode>::fixed_size() + SIGNATURE_BYTES
        );
        assert_eq!(SignedAttestation::from_ssz_bytes(&encoded).unwrap(), signed);
    }

    #[test]
    fn should_commit_to_every_field_when_a_signed_attestation_is_merkleized() {
        let signed = SignedAttestation {
            validator_index: ValidatorIndex(3),
            data: attestation_data(),
            signature: signature(),
        };

        let mut other = signed.clone();
        other.validator_index = ValidatorIndex(4);
        assert_ne!(
            signed.hash_tree_root(&Sha2Hasher),
            other.hash_tree_root(&Sha2Hasher)
        );

        let mut third = signed.clone();
        third.signature.rho = Randomness::try_from((1..8).map(Fp).collect::<Vec<_>>()).unwrap();
        assert_ne!(
            signed.hash_tree_root(&Sha2Hasher),
            third.hash_tree_root(&Sha2Hasher)
        );
    }

    #[test]
    fn should_round_trip_when_the_secret_key_tree_containers_are_decoded() {
        let layer = HashTreeLayer {
            start_index: 4,
            nodes: digests(3, 77),
        };
        assert_eq!(
            HashTreeLayer::from_ssz_bytes(&layer.to_ssz()).unwrap(),
            layer
        );

        let opening = HashTreeOpening {
            siblings: digests(2, 9),
        };
        assert_eq!(
            HashTreeOpening::from_ssz_bytes(&opening.to_ssz()).unwrap(),
            opening
        );
    }
}
