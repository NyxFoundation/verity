//! Signing and verification of one validator's XMSS signature.
//!
//! Two functions and the bridge between the wire shape and the library's own. Everything
//! that decides *whether* a signature should be produced — duty scheduling, the persisted
//! signing watermark, key preparation — lives above this crate; what is here is the
//! operation itself, with leanSig's two panicking preconditions turned into typed errors.

use leansig::serialization::Serializable;
use leansig::signature::SignatureScheme;
use leansig_wrapper::{LeanSigScheme, XmssSignature, xmss_verify};
use libssz::{SszDecode, SszEncode};
use verity_types::{Bytes32, Slot};

use crate::containers::{PublicKey, Signature};
use crate::error::SignatureError;
use crate::key::SecretKey;
use crate::scheme::epoch_for_slot;

impl Signature {
    /// Re-parses into the signature library's own signature type.
    pub(crate) fn to_leansig(&self) -> Result<XmssSignature, SignatureError> {
        XmssSignature::from_bytes(&self.to_ssz()).map_err(|_| SignatureError::MalformedSignature)
    }

    /// Re-parses from the signature library's own signature type.
    pub(crate) fn from_leansig(signature: &XmssSignature) -> Result<Self, SignatureError> {
        Self::from_ssz_bytes(&signature.to_bytes()).map_err(|_| SignatureError::MalformedSignature)
    }
}

/// Signs a message for one slot with one key.
///
/// # The message is always a root
///
/// leanSpec signs `hash_tree_root(attestation_data)` with the attestation key and
/// `hash_tree_root(block)` with the proposal key, and passes the consensus slot straight
/// through as the XMSS epoch. This function takes the root and the slot as given; deriving
/// them is the caller's job, because only the caller knows which of the two duties it is
/// performing.
///
/// # This function does not prevent key reuse
///
/// Determinism means re-signing the *same* message at the same slot returns the identical
/// signature and is harmless. Signing a *different* message at that slot breaks the key.
/// Nothing here can tell the two apart, because a second call carries no memory of the
/// first. The guarantee is the persisted watermark on the validator's signing path, checked
/// and durably written *before* this is called — see `docs/design/key-management.md`.
///
/// # Errors
///
/// [`SignatureError::KeyNotActive`] or [`SignatureError::KeyNotPrepared`] when the key
/// cannot cover the slot; leanSig asserts on both, so they are checked here first and the
/// asserts are unreachable. [`SignatureError::EncodingAttemptsExceeded`] when the
/// probabilistic message encoding exhausts its retry budget, which nothing the caller
/// controls causes.
pub fn sign(
    secret_key: &SecretKey,
    slot: Slot,
    message: &Bytes32,
) -> Result<Signature, SignatureError> {
    let epoch = secret_key.check_signable(slot)?;

    let signature = LeanSigScheme::sign(secret_key.inner(), epoch, message)
        .map_err(|_| SignatureError::EncodingAttemptsExceeded)?;

    Signature::from_leansig(&signature)
}

/// Verifies one validator's signature over a message at a slot.
///
/// # Errors
///
/// [`SignatureError::SlotOutsideLifetime`] when the slot is not an epoch the scheme has,
/// [`SignatureError::MalformedPublicKey`] or [`SignatureError::MalformedSignature`] when
/// either input does not re-parse, and [`SignatureError::InvalidSignature`] when the
/// cryptography rejects it. All four mean the same thing to a caller weighing a gossiped
/// attestation — do not accept it — but they mean different things in a log.
pub fn verify(
    public_key: &PublicKey,
    slot: Slot,
    message: &Bytes32,
    signature: &Signature,
) -> Result<(), SignatureError> {
    let epoch = epoch_for_slot(slot)?;
    let public_key = public_key.to_leansig()?;
    let signature = signature.to_leansig()?;

    xmss_verify(&public_key, epoch, message, &signature)
        .map_err(|()| SignatureError::InvalidSignature)
}
