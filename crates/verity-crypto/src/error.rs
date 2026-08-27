//! Why a signing, verification, or key-loading attempt failed.
//!
//! Two error types, split by who has to act on them.
//!
//! [`SignatureError`] is the outcome of a cryptographic operation on data the node already
//! holds. Its variants are what the caller maps onto a consensus rejection reason, so it
//! stays small and carries only what an operator log needs.
//!
//! [`AggregationError`] is the outcome of building or checking an aggregate proof, which is
//! a separate supplier (leanVM) with separate failure modes — proving is fallible for
//! reasons verification is not, and vice versa.
//!
//! [`KeyLoadError`] is a startup failure. Key loading is fail-closed by design
//! (`docs/design/key-management.md`, Decision 2), so every variant here means "refuse to
//! start", never "continue with less". It carries the offending path or index, because the
//! operator fixing it is looking at a directory, not at consensus state.

use core::fmt;
use std::path::PathBuf;

use crate::scheme::Role;

/// A cryptographic operation refused or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureError {
    /// The slot does not fit the scheme's epoch index.
    ///
    /// XMSS indexes epochs with a `u32` and the lifetime is `2^32`, so every representable
    /// epoch is a valid slot number — but a `Slot` is a `u64` and can name one that is not.
    /// Converting is therefore fallible, and this is where that shows up.
    SlotOutsideLifetime {
        /// The slot that could not be used as an epoch.
        slot: u64,
    },

    /// The key was never active for this slot.
    ///
    /// A key is generated for a sub-range of the scheme lifetime. Outside it there is no key
    /// material at all, and no amount of preparation produces any.
    KeyNotActive {
        /// The slot the caller asked to sign.
        slot: u64,
        /// First slot the key is active for.
        activation_start: u64,
        /// First slot past the key's activation range.
        activation_end: u64,
    },

    /// The key is active for this slot but its prepared window has not reached it.
    ///
    /// Recoverable, unlike [`Self::KeyNotActive`]: advancing preparation far enough makes the
    /// slot signable. See [`crate::SecretKey::advance_preparation`].
    KeyNotPrepared {
        /// The slot the caller asked to sign.
        slot: u64,
        /// First slot of the currently prepared window.
        prepared_start: u64,
        /// First slot past the currently prepared window.
        prepared_end: u64,
    },

    /// Message encoding did not reach the target sum within the scheme's retry budget.
    ///
    /// The incomparable encoding is probabilistic: signing retries with fresh randomness
    /// until the codeword hits the target sum. Exhausting the budget is astronomically
    /// unlikely for an honest signer and is not caused by anything the caller controls.
    EncodingAttemptsExceeded,

    /// The signature did not verify against the public key, slot, and message.
    InvalidSignature,

    /// A public key could not be parsed from its 52-byte wire form.
    MalformedPublicKey,

    /// A signature could not be parsed from its wire form.
    MalformedSignature,
}

impl fmt::Display for SignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SlotOutsideLifetime { slot } => {
                write!(f, "slot {slot} is outside the XMSS scheme lifetime")
            }
            Self::KeyNotActive {
                slot,
                activation_start,
                activation_end,
            } => write!(
                f,
                "key is not active at slot {slot}; active range is [{activation_start}, {activation_end})"
            ),
            Self::KeyNotPrepared {
                slot,
                prepared_start,
                prepared_end,
            } => write!(
                f,
                "key is not prepared for slot {slot}; prepared window is [{prepared_start}, {prepared_end})"
            ),
            Self::EncodingAttemptsExceeded => {
                f.write_str("message encoding exhausted the scheme's retry budget")
            }
            Self::InvalidSignature => f.write_str("signature did not verify"),
            Self::MalformedPublicKey => f.write_str("public key is not a valid 52-byte XMSS key"),
            Self::MalformedSignature => {
                f.write_str("signature bytes are not a valid XMSS signature")
            }
        }
    }
}

impl std::error::Error for SignatureError {}

/// Key loading refused to produce a usable key set.
///
/// Every variant is fatal at startup. None of them has a "load what we can" branch: a
/// validator that comes up with one role's key missing, or with a key the manifest does not
/// vouch for, is a validator that signs with material nobody declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyLoadError {
    /// A file the loader needs is missing or unreadable.
    Unreadable {
        /// The file that could not be read.
        path: PathBuf,
        /// The operating system's reason, rendered.
        reason: String,
    },

    /// The manifest is not the YAML shape the loader expects.
    MalformedManifest {
        /// The manifest that failed to parse.
        path: PathBuf,
        /// What the parser objected to.
        reason: String,
    },

    /// The manifest declares only one key per validator.
    ///
    /// lean-quickstart emitted a single `pubkey_hex` before devnet4. Accepting it would mean
    /// covering both roles with one one-time key, which is exactly the dual-role reuse
    /// leanSpec's reference loader rejects and zeam's fallback reproduces.
    SingleKeyManifest {
        /// The manifest carrying the legacy layout.
        path: PathBuf,
    },

    /// The manifest has no entry for a requested validator index.
    UnknownValidator {
        /// The index that was requested.
        index: u64,
    },

    /// A manifest public key is not 52 bytes of hex.
    MalformedManifestKey {
        /// The validator whose entry is malformed.
        index: u64,
        /// The role whose key is malformed.
        role: Role,
    },

    /// A key file on disk does not decode as XMSS key material.
    MalformedKeyFile {
        /// The file that failed to decode.
        path: PathBuf,
    },

    /// A loaded public key disagrees with what the manifest declares for that validator and role.
    ///
    /// The manifest is authoritative. A mismatch means the key directory and the manifest
    /// describe different validator sets, and nothing local can tell which one the network
    /// agreed on.
    PublicKeyMismatch {
        /// The validator whose key files disagree with the manifest.
        index: u64,
        /// The role whose key disagrees.
        role: Role,
    },

    /// One validator's attestation and proposal public keys are the same key.
    ///
    /// A proposer signs a block *and* an attestation in its own slot. One one-time key cannot
    /// cover both without signing two different messages at one epoch, which breaks the key
    /// rather than merely losing a duty.
    DuplicateRoleKeys {
        /// The validator whose two roles share a key.
        index: u64,
    },
}

impl fmt::Display for KeyLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, reason } => {
                write!(f, "cannot read {}: {reason}", path.display())
            }
            Self::MalformedManifest { path, reason } => {
                write!(f, "malformed key manifest {}: {reason}", path.display())
            }
            Self::SingleKeyManifest { path } => write!(
                f,
                "key manifest {} declares one key per validator; separate attester and proposer keys are required",
                path.display()
            ),
            Self::UnknownValidator { index } => {
                write!(f, "key manifest has no entry for validator {index}")
            }
            Self::MalformedManifestKey { index, role } => write!(
                f,
                "manifest {role} public key for validator {index} is not 52 bytes of hex"
            ),
            Self::MalformedKeyFile { path } => {
                write!(f, "{} is not valid XMSS key material", path.display())
            }
            Self::PublicKeyMismatch { index, role } => write!(
                f,
                "{role} public key on disk for validator {index} disagrees with the manifest"
            ),
            Self::DuplicateRoleKeys { index } => write!(
                f,
                "validator {index} uses one key for both attestation and proposal"
            ),
        }
    }
}

impl std::error::Error for KeyLoadError {}

/// Building or checking an aggregate proof failed.
///
/// # Two suppliers, one error
///
/// leanVM distinguishes a prover failure from a verifier failure from a malformed input.
/// Callers of this crate do not: an aggregate that will not prove and an aggregate that will
/// not verify are both "this proof is not usable", and the difference matters in a log, not
/// in a branch. The variants below preserve the distinction without asking the caller to
/// handle three error types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregationError {
    /// The proof did not verify.
    InvalidProof,

    /// The prover could not produce a proof from the inputs it was given.
    ///
    /// Carries leanVM's own rendering, because the reasons are a long tail of circuit and
    /// topology limits that this crate would only lose detail by re-classifying.
    ProvingFailed {
        /// leanVM's description of what went wrong.
        reason: String,
    },

    /// The inputs the caller assembled are not a valid aggregation request.
    ///
    /// Empty participant sets, more signatures than the circuit admits, children disagreeing
    /// on the message or slot. All are caller errors, all are refusals rather than failures.
    InvalidRequest {
        /// leanVM's description of what the request violated.
        reason: String,
    },

    /// Proof bytes did not decompress into a proof of the expected shape.
    MalformedProof,

    /// A public key handed in alongside a proof did not parse.
    ///
    /// The keys travel separately from the proof: a compact proof omits them, and the
    /// verifier supplies them from the validator registry. A key that does not parse means
    /// the registry and the proof cannot be checked against each other at all.
    MalformedPublicKey,

    /// The proof does not fit the 512 KiB the consensus container allows.
    ///
    /// Measured production aggregates run 155-236 KB, so reaching this means the aggregation
    /// topology grew past what the wire format was sized for, not that one proof was unlucky.
    ProofTooLarge {
        /// Size the proof compressed to, in bytes.
        size: usize,
        /// Largest size the container admits.
        limit: usize,
    },
}

impl fmt::Display for AggregationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProof => f.write_str("aggregate proof did not verify"),
            Self::ProvingFailed { reason } => write!(f, "aggregation failed: {reason}"),
            Self::InvalidRequest { reason } => write!(f, "invalid aggregation request: {reason}"),
            Self::MalformedProof => f.write_str("proof bytes are not a valid aggregate proof"),
            Self::MalformedPublicKey => {
                f.write_str("a public key supplied with the proof is not a valid XMSS key")
            }
            Self::ProofTooLarge { size, limit } => {
                write!(
                    f,
                    "proof is {size} bytes, over the {limit}-byte container limit"
                )
            }
        }
    }
}

impl std::error::Error for AggregationError {}
