//! The stage between the network and the chain task, and the only place `Verified*` is made.
//!
//! # Why it is a stage and not a step inside the importer
//!
//! `docs/design/concurrency.md`, Decision 2: the network task does topic checks and
//! deduplication only, so network liveness never waits on a proof; the chain task is a loop
//! of short sequential steps, so its interleaving space stays small enough to model-check.
//! Everything expensive — SSZ decode, `hash_tree_root`, XMSS and aggregate-proof verification
//! — happens here, in between, on the blocking pool.
//!
//! # The type boundary is the enforcement
//!
//! [`VerifiedBlock`], [`VerifiedAttestation`] and [`VerifiedAggregate`] have private fields
//! and no public constructor, so they can only be made in this module. The chain task's
//! network channel carries nothing else, which is what makes leanSpec's verify-before-import
//! ordering a fact about the types rather than a rule someone has to remember.
//!
//! # Pending, and what is thrown away
//!
//! An item whose state is not yet in view cannot be verified — a block's parent, a vote's
//! target — so it waits in one bounded buffer, keyed by the root it awaits, and is retried
//! when a new snapshot resolves it. Every *definitive* failure is dropped on the spot and
//! counted: malformed bytes, an unresolvable validator, a proof that does not hold. None of
//! them is punished, because gossipsub forwards before anyone verifies and the peer that
//! delivered it may be an honest relay (`docs/design/sync.md`, Decision 3).
//!
//! Overflow evicts the oldest arrival, silently. Nothing re-requests it: an evicted block is
//! what range sync is for, and an evicted vote comes back inside an aggregate or a block.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use libssz::SszDecode;
use tokio::sync::{mpsc, watch};

use verity_chain::{AttestationSignature, ChainView, hash_tree_root};
use verity_crypto::aggregate::{MultiMessageProof, SingleMessageProof};
use verity_crypto::containers::SignedAttestation;
use verity_p2p::GossipKind;
use verity_types::{
    AttestationData, Block, Bytes32, MultiMessageAggregate, SignedAggregatedAttestation,
    SignedBlock, ValidatorIndex, Validators,
};
use verity_validator::proofs;

/// A block whose proof has been checked against the registry its parent fixed.
///
/// Carries the root computed during verification, so the chain task re-hashes nothing.
#[derive(Debug, Clone)]
pub struct VerifiedBlock {
    root: Bytes32,
    block: Block,
    proof: MultiMessageAggregate,
}

impl VerifiedBlock {
    /// The block's root.
    #[must_use]
    pub const fn root(&self) -> Bytes32 {
        self.root
    }

    /// The block itself.
    #[must_use]
    pub const fn block(&self) -> &Block {
        &self.block
    }

    /// Everything the importer needs: the root, the block, and the proof to persist with it.
    #[must_use]
    pub fn into_parts(self) -> (Bytes32, Block, MultiMessageAggregate) {
        (self.root, self.block, self.proof)
    }
}

/// One validator's vote, with its XMSS signature checked against the target's registry.
#[derive(Debug, Clone)]
pub struct VerifiedAttestation {
    validator_index: ValidatorIndex,
    data: AttestationData,
    signature: AttestationSignature,
}

impl VerifiedAttestation {
    /// The vote, its signer, and the signature bytes the aggregator pool holds.
    #[must_use]
    pub fn into_parts(self) -> (ValidatorIndex, AttestationData, AttestationSignature) {
        (self.validator_index, self.data, self.signature)
    }
}

/// An aggregate whose proof holds, over the vote it claims to cover.
#[derive(Debug, Clone)]
pub struct VerifiedAggregate {
    attestation: SignedAggregatedAttestation,
}

impl VerifiedAggregate {
    /// The aggregate, ready to be recorded.
    #[must_use]
    pub fn into_inner(self) -> SignedAggregatedAttestation {
        self.attestation
    }
}

/// What the stage hands the chain task.
#[derive(Debug, Clone)]
pub enum Verified {
    /// A block, with its proof checked.
    Block(VerifiedBlock),
    /// A vote, with its signature checked.
    Attestation(VerifiedAttestation),
    /// An aggregate, with its proof checked.
    Aggregate(VerifiedAggregate),
}

/// Why an item will never be accepted, however long it waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationFailure {
    /// The bytes are not the container the topic promised.
    Undecodable,
    /// A named validator is outside the registry, or its stored key does not parse.
    Unresolvable,
    /// The proof bytes do not decompress into a proof of this shape.
    MalformedProof,
    /// The proof is well-formed but covers different messages than the item claims.
    ///
    /// This is the check that stops a proposer from folding honest signatures over other
    /// data into a block of its own.
    Unbound,
    /// The cryptography rejects it.
    Invalid,
}

impl core::fmt::Display for VerificationFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let reason = match self {
            Self::Undecodable => "undecodable",
            Self::Unresolvable => "names a validator outside the registry",
            Self::MalformedProof => "malformed proof",
            Self::Unbound => "the proof does not cover what the item claims",
            Self::Invalid => "invalid",
        };
        f.write_str(reason)
    }
}

/// What the stage discarded, for the operator and for `verity-metrics` when it lands.
#[derive(Debug, Default)]
pub struct StageCounters {
    rejected: AtomicU64,
    evicted: AtomicU64,
}

impl StageCounters {
    /// Items dropped because they can never be accepted.
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    /// Items dropped from the pending buffer to make room for newer arrivals.
    pub fn evicted(&self) -> u64 {
        self.evicted.load(Ordering::Relaxed)
    }
}

/// What woke the stage up.
enum Wake {
    /// Bytes arrived on a subscribed topic.
    Payload(GossipPayload),
    /// A new snapshot landed, which may resolve parked items.
    Snapshot,
    /// An input closed; the stage is done.
    Stop,
}

/// A payload as it leaves the network task: the topic it arrived on, and raw SSZ bytes.
#[derive(Debug, Clone)]
pub struct GossipPayload {
    /// The gossip channel it arrived on, which fixes how it decodes.
    pub kind: GossipKind,
    /// Uncompressed SSZ bytes. Meaning starts here, not on the network.
    pub payload: Vec<u8>,
}

/// A decoded item, still unverified, still possibly waiting for the state it needs.
#[derive(Debug, Clone)]
enum Decoded {
    Block(Box<SignedBlock>),
    Attestation(Box<SignedAttestation>),
    Aggregate(Box<SignedAggregatedAttestation>),
}

impl Decoded {
    /// The block whose post-state resolves this item's registry.
    ///
    /// A block verifies against its parent's post-state and a vote against its target's,
    /// because those are the states a peer would have resolved the keys from.
    const fn awaited_root(&self) -> Bytes32 {
        match self {
            Self::Block(signed) => signed.block.parent_root,
            Self::Attestation(signed) => signed.data.target.root,
            Self::Aggregate(signed) => signed.data.target.root,
        }
    }
}

/// The verification stage: decode, resolve, verify, forward.
pub struct VerificationStage {
    inbound: mpsc::Receiver<GossipPayload>,
    verified: mpsc::Sender<Verified>,
    view: watch::Receiver<Arc<ChainView>>,
    pending: VecDeque<Decoded>,
    pending_capacity: usize,
    counters: Arc<StageCounters>,
}

impl VerificationStage {
    /// Wires the stage between the network task and the chain task.
    #[must_use = "a stage does nothing until it is run"]
    pub fn new(
        inbound: mpsc::Receiver<GossipPayload>,
        verified: mpsc::Sender<Verified>,
        view: watch::Receiver<Arc<ChainView>>,
        pending_capacity: usize,
        counters: Arc<StageCounters>,
    ) -> Self {
        Self {
            inbound,
            verified,
            view,
            pending: VecDeque::with_capacity(pending_capacity),
            pending_capacity,
            counters,
        }
    }

    /// Runs until the network stops or the chain task goes away.
    ///
    /// The loop waits on two things: new bytes, and a new snapshot. A snapshot is what makes
    /// parked items resolvable, so it is a wake-up reason of its own rather than something
    /// polled between arrivals.
    pub async fn run(mut self) {
        loop {
            // As in the chain task: the `select!` produces a value, its futures end, and only
            // then does the handler take `&mut self`.
            let wake = tokio::select! {
                biased;

                received = self.inbound.recv() =>
                    received.map_or(Wake::Stop, Wake::Payload),

                changed = self.view.changed() =>
                    if changed.is_err() { Wake::Stop } else { Wake::Snapshot },
            };

            let carry_on = match wake {
                Wake::Payload(payload) => self.accept(payload).await,
                Wake::Snapshot => self.retry_pending().await,
                Wake::Stop => false,
            };
            if !carry_on {
                break;
            }
        }

        // Shutdown drops the pending buffer with everything in it. All of it is
        // peer-recoverable, exactly like anything the network edge sheds.
        tracing::debug!(
            pending = self.pending.len(),
            "verification stage stopping; parked items dropped"
        );
    }

    /// Decodes one payload and either verifies it or parks it. Returns whether to keep going.
    async fn accept(&mut self, payload: GossipPayload) -> bool {
        let decoded = match decode(&payload) {
            Ok(decoded) => decoded,
            Err(failure) => {
                self.reject(&payload.kind, failure);
                return true;
            }
        };
        self.resolve(decoded).await
    }

    /// Verifies an item if its state is in view, and parks it otherwise.
    async fn resolve(&mut self, decoded: Decoded) -> bool {
        let Some(validators) = self.registry_for(decoded.awaited_root()) else {
            self.park(decoded);
            return true;
        };

        // Verification is the expensive step and leaves the async threads for it.
        let outcome = tokio::task::spawn_blocking(move || verify(decoded, &validators)).await;
        match outcome {
            Ok(Ok(verified)) => self.verified.send(verified).await.is_ok(),
            Ok(Err(failure)) => {
                self.counters.rejected.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(%failure, "discarding a gossiped item");
                true
            }
            // The blocking pool is gone, which only happens as the runtime shuts down.
            Err(_) => false,
        }
    }

    /// Retries exactly the parked items whose awaited root the new snapshot resolves.
    async fn retry_pending(&mut self) -> bool {
        // The buffer is taken out before the snapshot is borrowed, so the partition below
        // touches locals only — the alternative would hold a borrow of `self` across a
        // mutation of `self.pending`.
        let parked = core::mem::take(&mut self.pending);
        let (resolvable, retained) = {
            let view = self.view.borrow_and_update();
            let mut ready = Vec::new();
            let mut retained = VecDeque::with_capacity(parked.len());
            for entry in parked {
                if view.state(entry.awaited_root()).is_some() {
                    ready.push(entry);
                } else {
                    retained.push_back(entry);
                }
            }
            (ready, retained)
        };
        self.pending = retained;

        for entry in resolvable {
            if !self.resolve(entry).await {
                return false;
            }
        }
        true
    }

    /// The registry an item verifies against, when the snapshot holds the state that fixes it.
    fn registry_for(&self, root: Bytes32) -> Option<Validators> {
        self.view
            .borrow()
            .state(root)
            .map(|state| state.validators.clone())
    }

    /// Parks an item, evicting the oldest arrival when the buffer is full.
    fn park(&mut self, decoded: Decoded) {
        if self.pending.len() >= self.pending_capacity {
            self.pending.pop_front();
            self.counters.evicted.fetch_add(1, Ordering::Relaxed);
        }
        self.pending.push_back(decoded);
    }

    fn reject(&self, kind: &GossipKind, failure: VerificationFailure) {
        self.counters.rejected.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(?kind, %failure, "discarding a gossiped payload");
    }
}

/// Reads a payload as whatever its topic promised it is.
fn decode(payload: &GossipPayload) -> Result<Decoded, VerificationFailure> {
    let undecodable = |_| VerificationFailure::Undecodable;
    match payload.kind {
        GossipKind::Block => SignedBlock::from_ssz_bytes(&payload.payload)
            .map(|block| Decoded::Block(Box::new(block)))
            .map_err(undecodable),
        GossipKind::Aggregation => SignedAggregatedAttestation::from_ssz_bytes(&payload.payload)
            .map(|aggregate| Decoded::Aggregate(Box::new(aggregate)))
            .map_err(undecodable),
        GossipKind::Attestation(_) => SignedAttestation::from_ssz_bytes(&payload.payload)
            .map(|attestation| Decoded::Attestation(Box::new(attestation)))
            .map_err(undecodable),
    }
}

/// The whole cryptographic check, on one item, against one registry.
fn verify(decoded: Decoded, validators: &Validators) -> Result<Verified, VerificationFailure> {
    match decoded {
        Decoded::Block(signed) => verify_block(*signed, validators).map(Verified::Block),
        Decoded::Attestation(signed) => {
            verify_attestation(*signed, validators).map(Verified::Attestation)
        }
        Decoded::Aggregate(signed) => {
            verify_aggregate(*signed, validators).map(Verified::Aggregate)
        }
    }
}

/// Checks a block's one merged proof against the keys and messages the block itself claims.
///
/// The proof carries neither keys nor a claim that its messages are the right ones. Both
/// lists are therefore rebuilt from the block — one entry per body attestation, in body
/// order, then the proposer's — and compared. Without the message comparison a proposer could
/// fold honest signatures over other data into a proof that verifies perfectly.
///
/// Transcribed from leanSpec `src/lean_spec/spec/forks/lstar/signatures.py`, read at commit
/// `8603fa63`.
fn verify_block(
    signed: SignedBlock,
    validators: &Validators,
) -> Result<VerifiedBlock, VerificationFailure> {
    let root = hash_tree_root(&signed.block);

    let mut keys_per_component = Vec::with_capacity(signed.block.body.attestations.len() + 1);
    let mut expected = Vec::with_capacity(signed.block.body.attestations.len() + 1);

    for attestation in signed.block.body.attestations.iter() {
        keys_per_component.push(
            proofs::attestation_keys(validators, &attestation.aggregation_bits)
                .map_err(|_| VerificationFailure::Unresolvable)?,
        );
        expected.push((hash_tree_root(&attestation.data), attestation.data.slot));
    }

    keys_per_component.push(vec![
        proofs::proposal_key(validators, signed.block.proposer_index)
            .map_err(|_| VerificationFailure::Unresolvable)?,
    ]);
    expected.push((root, signed.block.slot));

    let proof = MultiMessageProof::from_proof_bytes(&signed.proof.proof, &keys_per_component)
        .map_err(|_| VerificationFailure::MalformedProof)?;
    if proof.bindings() != expected {
        return Err(VerificationFailure::Unbound);
    }
    proof.verify().map_err(|_| VerificationFailure::Invalid)?;

    Ok(VerifiedBlock {
        root,
        block: signed.block,
        proof: signed.proof,
    })
}

/// Checks one validator's XMSS signature over the vote it claims to have cast.
fn verify_attestation(
    signed: SignedAttestation,
    validators: &Validators,
) -> Result<VerifiedAttestation, VerificationFailure> {
    let key = proofs::attestation_key(validators, signed.validator_index)
        .map_err(|_| VerificationFailure::Unresolvable)?;

    verity_crypto::verify(
        &key,
        signed.data.slot,
        &hash_tree_root(&signed.data),
        &signed.signature,
    )
    .map_err(|_| VerificationFailure::Invalid)?;

    Ok(VerifiedAttestation {
        validator_index: signed.validator_index,
        data: signed.data,
        signature: AttestationSignature(libssz::SszEncode::to_ssz(&signed.signature)),
    })
}

/// Checks an aggregate's proof, and that it covers the vote the envelope names.
fn verify_aggregate(
    signed: SignedAggregatedAttestation,
    validators: &Validators,
) -> Result<VerifiedAggregate, VerificationFailure> {
    let keys = proofs::attestation_keys(validators, &signed.proof.participants)
        .map_err(|_| VerificationFailure::Unresolvable)?;
    if keys.is_empty() {
        return Err(VerificationFailure::Unbound);
    }

    let proof = SingleMessageProof::from_proof_bytes(&signed.proof.proof, &keys)
        .map_err(|_| VerificationFailure::MalformedProof)?;
    if proof.message() != hash_tree_root(&signed.data) || proof.slot() != signed.data.slot {
        return Err(VerificationFailure::Unbound);
    }
    proof.verify().map_err(|_| VerificationFailure::Invalid)?;

    Ok(VerifiedAggregate {
        attestation: signed,
    })
}
