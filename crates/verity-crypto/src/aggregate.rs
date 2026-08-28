//! Signature aggregation and aggregate-proof verification, over leanVM.
//!
//! # What an aggregate is here
//!
//! Two shapes, and leanSpec names them by the messages they cover rather than by depth.
//!
//! A **single-message** proof attests that many validators each signed the *same* message at
//! the same slot. That is what an aggregator broadcasts for an attestation, and what a block
//! carries one of per distinct vote.
//!
//! A **multi-message** proof merges single-message proofs that cover *different* messages
//! into one. A block carries exactly one: it binds every attestation in the body plus the
//! proposer's own signature over the block root, which is a different message from any of
//! them.
//!
//! # Proving is not concurrency-safe
//!
//! leanVM's prover allocates from one shared arena per process. Two proofs generated
//! concurrently in one process corrupt each other's buffers. [`aggregate_single_message`]
//! and [`merge_single_message_proofs`] are therefore not safe to run in parallel with each
//! other or with themselves, however many threads are available — parallelism has to come
//! from separate processes. Verification carries no such restriction.
//!
//! # Public keys travel separately from the proof
//!
//! The serialized form omits them: a `SingleMessageAggregate` on the network is a
//! participation bitfield plus proof bytes, and the verifier resolves the bitfield against
//! the validator registry it already trusts. So decoding a proof takes the keys as a second
//! argument, and a caller that cannot resolve the bitfield cannot decode the proof at all —
//! which is the intended failure, not an inconvenience.
//!
//! That asymmetry is why [`SingleMessageProof::to_proof_bytes`] is not named for
//! compression, which is what leanVM calls it. Dropping the keys is the load-bearing half;
//! the lz4 pass is incidental.

use rec_aggregation::{
    MultiMessageAggregateSignature, SingleMessageAggregateSignature,
    aggregate_single_message_signatures, init_aggregation_bytecode,
    merge_single_message_aggregates, verify_multi_message_aggregate,
    verify_single_message_aggregate,
};
use verity_types::{Bytes32, Slot};

use crate::containers::{PublicKey, Signature};
use crate::error::AggregationError;
use crate::scheme::epoch_for_slot;

/// Most single-message proofs one multi-message proof can merge.
pub use rec_aggregation::MAX_RECURSIONS;

/// Most individual signatures one single-message proof can cover.
pub use rec_aggregation::MAX_XMSS_AGGREGATED;

/// WHIR blowup used when aggregating signatures over one message.
///
/// leanVM's own default for this shape. It is a prover-side proof-size/time tradeoff, not a
/// consensus parameter: verification reads the rate out of the proof, so a peer proving at a
/// different rate still verifies here.
pub const SINGLE_MESSAGE_LOG_INV_RATE: usize = 1;

/// WHIR blowup used when merging single-message proofs, leanVM's default for that shape.
pub const MERGE_LOG_INV_RATE: usize = 2;

/// Compiles the aggregation circuit, once per process.
///
/// leanVM panics if a proof is verified before the circuit exists. Every entry point in this
/// module calls this first, so no caller can reach that panic; it is public because the
/// compilation is slow enough to be worth paying at startup rather than on the first
/// attestation to arrive.
pub fn init() {
    init_aggregation_bytecode();
}

/// Warms up the prover in addition to the verifier.
///
/// Engages leanVM's arena allocator, spins up its thread pool, and precomputes DFT twiddles.
/// All three are performance, not correctness: proving works without them and is slower. A
/// node that never aggregates does not need this; a node that does should call it at
/// startup, since otherwise the first aggregation of its life pays for all three.
pub fn init_prover() {
    backend::enable_arena();
    backend::parallel::init();
    backend::precompute_dft_twiddles::<backend::KoalaBear>(1 << 24);
    init_aggregation_bytecode();
}

/// A proof that a set of validators each signed one message at one slot.
///
/// leanSpec's Type-1 proof, and the payload of `SingleMessageAggregate`.
#[derive(Debug, Clone)]
pub struct SingleMessageProof(SingleMessageAggregateSignature);

/// A proof merging single-message proofs over distinct messages.
///
/// leanSpec's Type-2 proof, and the payload of `MultiMessageAggregate` — the one proof a
/// block carries whole.
#[derive(Debug, Clone)]
pub struct MultiMessageProof(MultiMessageAggregateSignature);

impl SingleMessageProof {
    /// The message every participant signed.
    pub fn message(&self) -> Bytes32 {
        self.0.info.without_pubkeys.message
    }

    /// The slot every participant signed at.
    pub fn slot(&self) -> Slot {
        Slot(u64::from(self.0.info.without_pubkeys.slot))
    }

    /// The participating public keys, in the sorted order leanVM keeps them in.
    ///
    /// # Errors
    ///
    /// [`AggregationError::MalformedPublicKey`] if a key leanVM holds does not re-parse,
    /// which would mean the two libraries disagree about the encoding.
    pub fn participants(&self) -> Result<Vec<PublicKey>, AggregationError> {
        self.0
            .info
            .pubkeys
            .iter()
            .map(|key| {
                PublicKey::from_leansig(key).map_err(|_| AggregationError::MalformedPublicKey)
            })
            .collect()
    }

    /// Checks the proof.
    ///
    /// # Errors
    ///
    /// [`AggregationError::InvalidProof`] when the proof does not hold, which also covers a
    /// participant set the circuit refuses — empty, oversized, or not strictly sorted.
    pub fn verify(&self) -> Result<(), AggregationError> {
        init();
        verify_single_message_aggregate(&self.0)
            .map(|_| ())
            .map_err(|_| AggregationError::InvalidProof)
    }

    /// The public-key-free bytes that fill `SingleMessageAggregate.proof`.
    ///
    /// Not a network message: this is one field's value, which the caller places in the
    /// container and the container's own SSZ encoding then carries. The encoding is leanVM's
    /// (postcard, lz4-compressed), opaque to SSZ, which sees only a byte list.
    ///
    /// Infallible here, and bounded elsewhere: `SingleMessageAggregate.proof` is a
    /// `ByteList512KiB`, so the 512 KiB ceiling is enforced when the caller builds that
    /// container, not here. Measured production aggregates run 155-236 KB, so a proof that
    /// reaches the ceiling means the aggregation topology outgrew what the container was
    /// sized for — a design question, not one proof being unlucky.
    pub fn to_proof_bytes(&self) -> Vec<u8> {
        self.0.compress_without_pubkeys()
    }

    /// Parses `SingleMessageAggregate.proof` back into a proof, given the participants' keys.
    ///
    /// The keys come from the caller's validator registry, resolved from the aggregate's
    /// participation bitfield. leanVM sorts and deduplicates them itself, so the order handed
    /// in does not matter.
    ///
    /// # Errors
    ///
    /// [`AggregationError::MalformedPublicKey`] if a key does not parse, and
    /// [`AggregationError::MalformedProof`] if the bytes do not decompress into a proof of
    /// this shape.
    pub fn from_proof_bytes(
        bytes: &[u8],
        participants: &[PublicKey],
    ) -> Result<Self, AggregationError> {
        let keys = to_leansig_keys(participants)?;
        SingleMessageAggregateSignature::decompress_without_pubkeys(bytes, keys)
            .map(Self)
            .ok_or(AggregationError::MalformedProof)
    }
}

impl MultiMessageProof {
    /// Checks the proof.
    ///
    /// # Errors
    ///
    /// [`AggregationError::InvalidProof`] when the proof does not hold, or when any
    /// component's participant set is one the circuit refuses.
    pub fn verify(&self) -> Result<(), AggregationError> {
        init();
        verify_multi_message_aggregate(&self.0)
            .map(|_| ())
            .map_err(|_| AggregationError::InvalidProof)
    }

    /// The public-key-free bytes that fill `MultiMessageAggregate.proof`.
    ///
    /// One field's value, in leanVM's own encoding, bounded where the container is built —
    /// as for the single-message form above.
    pub fn to_proof_bytes(&self) -> Vec<u8> {
        self.0.compress_without_pubkeys()
    }

    /// Parses `MultiMessageAggregate.proof`, given each component's participants.
    ///
    /// The outer slice runs parallel to the proof's components, in order: for a block, that
    /// is one entry per aggregated attestation followed by a single-element entry for the
    /// proposer.
    ///
    /// # Errors
    ///
    /// [`AggregationError::MalformedPublicKey`] if a key does not parse, and
    /// [`AggregationError::MalformedProof`] if the bytes do not decompress, or if the
    /// component count disagrees with the number of key sets supplied.
    pub fn from_proof_bytes(
        bytes: &[u8],
        participants_per_component: &[Vec<PublicKey>],
    ) -> Result<Self, AggregationError> {
        let keys = participants_per_component
            .iter()
            .map(|component| to_leansig_keys(component))
            .collect::<Result<Vec<_>, _>>()?;

        MultiMessageAggregateSignature::decompress_without_pubkeys(bytes, keys)
            .map(Self)
            .ok_or(AggregationError::MalformedProof)
    }
}

/// Aggregates raw signatures, and any proofs already built, over one message and slot.
///
/// Every input must be over the same `(message, slot)` pair; leanVM refuses a mixed batch
/// rather than silently proving a subset.
///
/// # Errors
///
/// [`SignatureError::SlotOutsideLifetime`] reaches the caller as
/// [`AggregationError::InvalidRequest`], since from the aggregator's side an unusable slot is
/// a malformed request. [`AggregationError::MalformedPublicKey`] or
/// [`AggregationError::MalformedProof`] when an input does not re-parse, and
/// [`AggregationError::ProvingFailed`] when leanVM cannot build the proof.
///
/// [`SignatureError::SlotOutsideLifetime`]: crate::error::SignatureError::SlotOutsideLifetime
pub fn aggregate_single_message(
    children: Vec<SingleMessageProof>,
    signatures: &[(PublicKey, Signature)],
    message: &Bytes32,
    slot: Slot,
) -> Result<SingleMessageProof, AggregationError> {
    init();

    let epoch = epoch_for_slot(slot).map_err(|error| AggregationError::InvalidRequest {
        reason: error.to_string(),
    })?;

    let raw = signatures
        .iter()
        .map(|(key, signature)| {
            let key = key
                .to_leansig()
                .map_err(|_| AggregationError::MalformedPublicKey)?;
            let signature = signature
                .to_leansig()
                .map_err(|_| AggregationError::MalformedProof)?;
            Ok((key, signature))
        })
        .collect::<Result<Vec<_>, AggregationError>>()?;

    // By value, not by reference: a child proof is on the order of 200 KB, the aggregator has
    // no use for it once it is folded in, and leanVM needs its own slice anyway.
    let children: Vec<SingleMessageAggregateSignature> =
        children.into_iter().map(|child| child.0).collect();

    aggregate_single_message_signatures(
        &children,
        raw,
        *message,
        epoch,
        SINGLE_MESSAGE_LOG_INV_RATE,
    )
    .map(SingleMessageProof)
    .map_err(|error| AggregationError::ProvingFailed {
        reason: error.to_string(),
    })
}

/// Merges single-message proofs over distinct messages into the one proof a block carries.
///
/// # Errors
///
/// [`AggregationError::InvalidRequest`] when the component set is empty or longer than
/// [`MAX_RECURSIONS`], and [`AggregationError::ProvingFailed`] when leanVM cannot build the
/// merged proof.
pub fn merge_single_message_proofs(
    components: Vec<SingleMessageProof>,
) -> Result<MultiMessageProof, AggregationError> {
    init();

    if components.is_empty() || components.len() > MAX_RECURSIONS {
        return Err(AggregationError::InvalidRequest {
            reason: format!(
                "a block proof merges between 1 and {MAX_RECURSIONS} components, got {}",
                components.len()
            ),
        });
    }

    let components: Vec<SingleMessageAggregateSignature> =
        components.into_iter().map(|proof| proof.0).collect();

    merge_single_message_aggregates(components, MERGE_LOG_INV_RATE)
        .map(MultiMessageProof)
        .map_err(|error| AggregationError::ProvingFailed {
            reason: error.to_string(),
        })
}

fn to_leansig_keys(
    keys: &[PublicKey],
) -> Result<Vec<leansig_wrapper::XmssPublicKey>, AggregationError> {
    keys.iter()
        .map(|key| {
            key.to_leansig()
                .map_err(|_| AggregationError::MalformedPublicKey)
        })
        .collect()
}
