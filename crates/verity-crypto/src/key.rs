//! Key material: the secret half a validator signs with, and the public half everyone verifies against.
//!
//! # Two representations of a public key, on purpose
//!
//! [`crate::containers::PublicKey`] is the SSZ shape — what the validator registry stores as
//! `Bytes52` and what `hash_tree_root` sees. leanSig's own `XmssPublicKey` is what the
//! signature scheme operates on. They encode identically, so moving between them is a
//! re-parse of 52 bytes, done here rather than at every call site.
//!
//! The secret key has no such split: nothing outside this crate should hold one in a form it
//! could accidentally serialize, so the only representations are [`SecretKey`] and the bytes
//! on disk.

use core::ops::Range;

use leansig::serialization::Serializable;
use leansig::signature::SignatureSchemeSecretKey;
use leansig_wrapper::{
    XmssPublicKey, XmssSecretKey, xmss_public_key_from_ssz, xmss_public_key_to_ssz,
};
use libssz::{SszDecode, SszEncode};
use verity_types::{Bytes52, Slot};

use crate::containers::PublicKey;
use crate::error::SignatureError;
use crate::scheme::epoch_for_slot;

impl PublicKey {
    /// Parses a public key from the 52 bytes the validator registry stores.
    ///
    /// # Errors
    ///
    /// [`SignatureError::MalformedPublicKey`] when the bytes are not a well-formed key —
    /// which, at this fixed length, means a field element outside the canonical range.
    pub fn from_bytes52(bytes: &Bytes52) -> Result<Self, SignatureError> {
        Self::from_ssz_bytes(bytes).map_err(|_| SignatureError::MalformedPublicKey)
    }

    /// Renders the key back to the 52 bytes the validator registry stores.
    pub fn to_bytes52(&self) -> Bytes52 {
        let encoded = self.to_ssz();
        debug_assert_eq!(encoded.len(), crate::scheme::PUBLIC_KEY_BYTES);
        let mut bytes = [0u8; 52];
        bytes.copy_from_slice(&encoded);
        bytes
    }

    /// Re-parses into the signature library's own key type.
    pub(crate) fn to_leansig(&self) -> Result<XmssPublicKey, SignatureError> {
        xmss_public_key_from_ssz(&self.to_ssz()).map_err(|()| SignatureError::MalformedPublicKey)
    }

    /// Re-parses from the signature library's own key type.
    pub(crate) fn from_leansig(key: &XmssPublicKey) -> Result<Self, SignatureError> {
        Self::from_ssz_bytes(&xmss_public_key_to_ssz(key))
            .map_err(|_| SignatureError::MalformedPublicKey)
    }
}

/// A validator's XMSS secret key, with its signable window.
///
/// # Stateful, and the state is the whole point
///
/// XMSS signs at most once per epoch. Signing two *different* messages at one epoch does not
/// cost a penalty — the lean protocol has no slashing — it exposes that epoch's one-time-chain
/// values, making a forgery possible at that one slot. Epochs are independent (each chain start
/// comes from the PRF applied to its own epoch), so the rest of the key is untouched. This type
/// does not enforce non-reuse: the guarantee is the validator duty loop's once-per-slot
/// structure, held in memory and never persisted. See `docs/design/key-management.md`,
/// Decision 1.
///
/// What this type does enforce is the weaker precondition leanSig asserts on: that the slot
/// is inside the key's activation range and its prepared window. leanSig panics on both, and
/// Runtime Shell code must not panic, so every path here checks first and returns a typed
/// error instead.
pub struct SecretKey(XmssSecretKey);

impl core::fmt::Debug for SecretKey {
    /// Prints the window, never the material.
    ///
    /// A `Debug` that rendered the key would put 33.5 MB of secret into any log line that
    /// formatted a validator, so the derive is deliberately not used.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let activation = self.activation_interval();
        let prepared = self.prepared_interval();
        f.debug_struct("SecretKey")
            .field("activation", &activation)
            .field("prepared", &prepared)
            .finish_non_exhaustive()
    }
}

impl SecretKey {
    /// Parses a secret key from its canonical SSZ encoding, as written by the key generator.
    ///
    /// # Errors
    ///
    /// `Err(())` when the bytes do not decode. The caller turns that into whichever of its
    /// own errors names the file, which this function does not know.
    #[allow(clippy::result_unit_err)]
    pub fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, ()> {
        XmssSecretKey::from_bytes(bytes).map(Self).map_err(|_| ())
    }

    /// An independent copy of this key, for the worker that advances it off-thread.
    ///
    /// # Not a memcpy
    ///
    /// `docs/design/key-management.md` describes clone-advance-swap as handing a worker a
    /// clone so the original keeps signing through the rebuild. leanSig's secret key does not
    /// implement `Clone`, so the copy goes through the canonical encoding: roughly 33.5 MB
    /// serialized and parsed again, rather than the memcpy the design assumed. That is still
    /// far cheaper than the rebuild it runs alongside, and it happens about once every three
    /// days per key, but it is not free and the cost belongs where a caller can see it.
    ///
    /// # Errors
    ///
    /// `Err(())` if the key does not survive its own round trip, which would mean the
    /// encoding and the in-memory shape disagree — a library bug, not an input problem.
    #[allow(clippy::result_unit_err)]
    pub fn duplicate(&self) -> Result<Self, ()> {
        Self::from_ssz_bytes(&self.to_ssz_bytes())
    }

    /// The library's own key, for the one call site that needs it.
    pub(crate) const fn inner(&self) -> &XmssSecretKey {
        &self.0
    }

    /// Renders the key to its canonical SSZ encoding.
    ///
    /// This is what an advanced key is written back as, so a restart does not owe one rebuild
    /// per three days since the key's activation.
    pub fn to_ssz_bytes(&self) -> Vec<u8> {
        self.0.to_bytes()
    }

    /// Slots the key holds material for at all, fixed at generation.
    ///
    /// Nothing recovers a slot outside this range: there is no key material there to prepare.
    pub fn activation_interval(&self) -> Range<u64> {
        self.0.get_activation_interval()
    }

    /// Slots the key can sign for right now.
    ///
    /// A sliding window two bottom trees wide — about six days at four-second slots — inside
    /// the activation interval. [`Self::advance_preparation`] moves it.
    pub fn prepared_interval(&self) -> Range<u64> {
        self.0.get_prepared_interval()
    }

    /// Checks the two preconditions leanSig would otherwise assert on.
    ///
    /// # Errors
    ///
    /// [`SignatureError::SlotOutsideLifetime`] if the slot is not an epoch at all,
    /// [`SignatureError::KeyNotActive`] if the key holds no material for it, and
    /// [`SignatureError::KeyNotPrepared`] if it does but the window has not reached it. The
    /// last is the only recoverable one.
    pub fn check_signable(&self, slot: Slot) -> Result<u32, SignatureError> {
        let epoch = epoch_for_slot(slot)?;
        let epoch64 = u64::from(epoch);

        let activation = self.activation_interval();
        if !activation.contains(&epoch64) {
            return Err(SignatureError::KeyNotActive {
                slot: slot.0,
                activation_start: activation.start,
                activation_end: activation.end,
            });
        }

        let prepared = self.prepared_interval();
        if !prepared.contains(&epoch64) {
            return Err(SignatureError::KeyNotPrepared {
                slot: slot.0,
                prepared_start: prepared.start,
                prepared_end: prepared.end,
            });
        }

        Ok(epoch)
    }

    /// Slides the prepared window forward by one bottom tree.
    ///
    /// # Cost, and who should pay it
    ///
    /// One step rebuilds 65,536 one-time chains. It is rayon-parallel and takes long enough
    /// that a caller holding a lock over it stalls signing, so the intended shape is
    /// clone-advance-swap: hand a clone to a blocking worker, keep signing with the original,
    /// swap when the worker returns. The windows overlap by about three days around the
    /// current slot, so the original stays able to sign throughout. Both objects are the same
    /// key; no-reuse is carried by the duty loop's once-per-slot dedup, not by which clone
    /// signed.
    ///
    /// Advancing past the end of the activation interval does nothing, which is why this
    /// returns no error and why [`Self::advance_preparation_to`] bounds its own loop instead
    /// of trusting this to terminate.
    pub fn advance_preparation(&mut self) {
        self.0.advance_preparation();
    }

    /// Advances until `slot` is signable, or reports why it never will be.
    ///
    /// Used at startup, where a node that was down for a while owes one rebuild per three
    /// days of downtime.
    ///
    /// # Errors
    ///
    /// [`SignatureError::KeyNotActive`] when the slot lies outside the activation interval,
    /// and [`SignatureError::KeyNotPrepared`] if the window stops moving before it covers the
    /// slot. The second is the loop's own bound: [`Self::advance_preparation`] is defined to
    /// do nothing once the next window would leave the activation interval, so termination
    /// cannot rest on it always making progress.
    pub fn advance_preparation_to(&mut self, slot: Slot) -> Result<(), SignatureError> {
        loop {
            match self.check_signable(slot) {
                Ok(_) => return Ok(()),
                Err(error @ SignatureError::KeyNotPrepared { .. }) => {
                    let before = self.prepared_interval();
                    self.advance_preparation();
                    if before == self.prepared_interval() {
                        return Err(error);
                    }
                }
                Err(other) => return Err(other),
            }
        }
    }
}
