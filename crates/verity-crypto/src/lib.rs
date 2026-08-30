//! Post-quantum signatures and aggregation for Verity.
//!
//! # What belongs here
//!
//! One capability with two suppliers behind it. [`leansig`] provides per-validator XMSS
//! signing and verification; leanVM provides aggregation and aggregate-proof verification.
//! Callers see neither: they see [`sign`], [`verify`], and the [`aggregate`] module, and the
//! choice of supplier stays a fact about this crate.
//!
//! Consensus decisions are deliberately absent. Nothing here knows whether a signature
//! *should* have been produced, which vote it covers, or what to do when one fails to
//! verify. `verity-chain` leaves three seams where leanSpec verifies — the gossiped
//! attestation, the gossiped aggregate, and the block proof — and filling them is the
//! caller's composition, not this crate's business.
//!
//! # The no-reuse guarantee is not here either
//!
//! XMSS is a stateful one-time scheme: signing two different messages at one slot with one
//! key does not cost a penalty, it breaks the key. This crate refuses the preconditions
//! leanSig would panic on — a slot outside the key's activation range or its prepared
//! window — but it cannot refuse a second, different message at a slot it already signed,
//! because a call carries no memory of the last one. That guarantee is the validator duty
//! loop's structure: block production runs once per slot at interval 0, and attestation is
//! collapsed to one signature per slot by an in-memory already-attested set. Nothing about
//! signing is persisted. See `docs/design/key-management.md`, Decision 1.
//!
//! # Two SSZ libraries meet here
//!
//! Consensus containers go through `libssz`; leanSig serializes through `ethereum_ssz`. The
//! encodings agree byte for byte, so [`containers`] declares the wire shapes in `libssz`
//! terms and converts by re-parsing. The conversion is explicit rather than free, and it is
//! confined to this crate.
//!
//! # Source
//!
//! Scheme parameters follow leanSpec `PROD_CONFIG`; container shapes are transcribed from
//! leanSpec `src/lean_spec/spec/crypto/xmss/`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`.

pub mod aggregate;
pub mod containers;
pub mod error;
pub mod key;
pub mod keystore;
pub mod scheme;
pub mod signature;

pub use aggregate::{
    MultiMessageProof, SingleMessageProof, aggregate_single_message, merge_single_message_proofs,
};
pub use containers::{
    Fp, HashDigest, HashDigestList, HashTreeLayer, HashTreeOpening, Parameter, PublicKey,
    Randomness, Signature, SignedAttestation,
};
pub use error::{AggregationError, KeyLoadError, SignatureError};
pub use key::SecretKey;
pub use keystore::{RoleKeys, ValidatorKeys};
pub use scheme::{Role, epoch_for_slot};
pub use signature::{sign, verify};
