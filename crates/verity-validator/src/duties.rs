//! The duty loop: one tick in, at most one signature per role out.
//!
//! # The loop's shape is the no-reuse guarantee
//!
//! XMSS signs at most once per epoch, and the epoch *is* the slot. Signing two different
//! messages at one slot with one key does not cost a penalty — it exposes that slot's
//! one-time-chain values. Nothing in `verity-crypto` can prevent that, because a signing call
//! carries no memory of the last one. What prevents it is this loop:
//!
//! - **Block production runs at interval 0 only.** Interval 0 is reached once per slot, so
//!   the proposal key needs no dedup at all.
//! - **Attestation runs at interval ≥ 1**, so a proposal that overruns interval 0 does not
//!   also cost that slot's vote. That admits up to four passes per slot, which is why the
//!   attestation key does carry a dedup: an in-memory set of already-attested slots, kept
//!   four slots deep.
//!
//! Nothing about signing is persisted, and nothing here fsyncs. See
//! `docs/design/key-management.md`, Decision 1, for why the persisted watermark an earlier
//! design specified was withdrawn.
//!
//! # What runs where
//!
//! Signing is done inline: it is this task's own work, it needs the key this task owns, and
//! nothing else waits on it. Proving is not — a block's merged proof takes seconds, so it
//! leaves for the [`Prover`] and the finished block is sent from there. That is what lets the
//! same slot's attestation happen at interval 1 while the proposal is still being proved.

use std::collections::BTreeSet;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use verity_chain::{BuiltBlock, ChainView, build_block, hash_tree_root, proposer_for_slot};
use verity_crypto::aggregate::{aggregate_single_message, merge_single_message_proofs};
use verity_crypto::containers::{Signature, SignedAttestation};
use verity_crypto::sign;
use verity_types::config::INTERVALS_PER_SLOT;
use verity_types::{
    AttestationData, Bytes32, Interval, MultiMessageAggregate, SignedBlock, SingleMessageAggregate,
    Slot, ValidatorIndex, Validators,
};

use crate::error::DutyError;
use crate::keys::{AdvancesInFlight, Keyring, advance};
use crate::product::LocalProduct;
use crate::proofs;
use crate::prover::Prover;

/// How many slots of attestation history the dedup keeps.
///
/// Four, because interval ≥ 1 admits up to four passes inside one slot; anything older can no
/// longer be re-reached by this loop.
const ATTESTED_SLOT_RETENTION: u64 = 4;

/// The validator client: keys, the duties they owe, and the products they yield.
pub struct DutyService {
    keyring: Keyring,
    prover: Prover,
    products: mpsc::Sender<LocalProduct>,
    view: watch::Receiver<Arc<ChainView>>,
    ticks: watch::Receiver<Interval>,
    attested: BTreeSet<Slot>,
    advancing: AdvancesInFlight,
}

impl DutyService {
    /// Wires a validator client to the clock, the chain view, and the product channel.
    ///
    /// The receivers are handed over at construction, before the task starts, which is what
    /// makes the first `ChainView` a serving gate rather than a race: there is no moment at
    /// which this service exists and no view does.
    #[must_use = "a service does nothing until it is run"]
    pub fn new(
        keyring: Keyring,
        prover: Prover,
        products: mpsc::Sender<LocalProduct>,
        view: watch::Receiver<Arc<ChainView>>,
        ticks: watch::Receiver<Interval>,
    ) -> Self {
        Self {
            keyring,
            prover,
            products,
            view,
            ticks,
            attested: BTreeSet::new(),
            advancing: AdvancesInFlight::default(),
        }
    }

    /// Prepares the keys, then serves duties until the clock stops.
    ///
    /// Shutdown is channel closure and nothing else: when the clock's sender is dropped the
    /// loop ends, this service's product sender goes with it, and the chain task sees its own
    /// input close in turn.
    ///
    /// A preparation failure at startup stops duties without stopping the node. The node
    /// still follows the chain; it simply does not sign, which is the honest outcome when a
    /// key cannot cover the current slot at all.
    pub async fn run(mut self) {
        if self.keyring.is_empty() {
            tracing::info!("no validator keys configured; this node follows without signing");
            return;
        }

        let slot = slot_of(*self.ticks.borrow_and_update());
        match self.prepare(slot).await {
            Ok(keyring) => self.keyring = keyring,
            Err(error) => {
                tracing::error!(%error, "validator duties disabled: key preparation failed");
                return;
            }
        }
        tracing::info!(
            validators = self.keyring.validators().count(),
            slot = slot.0,
            "validator duties ready"
        );

        while self.ticks.changed().await.is_ok() {
            let interval = *self.ticks.borrow_and_update();
            self.on_interval(interval).await;
        }
    }

    /// Brings every key far enough forward to sign for `slot`, off the async threads.
    ///
    /// The keyring travels into the blocking pool and back rather than being borrowed: the
    /// catch-up can take minutes after long downtime, and there is nothing for this task to
    /// do until it finishes.
    async fn prepare(&mut self, slot: Slot) -> Result<Keyring, DutyError> {
        let mut keyring = core::mem::replace(&mut self.keyring, Keyring::empty());
        tokio::task::spawn_blocking(move || keyring.prepare_for(slot).map(|()| keyring))
            .await
            .map_err(|_| DutyError::ProverStopped)?
    }

    /// One interval's worth of duty.
    async fn on_interval(&mut self, interval: Interval) {
        let slot = slot_of(interval);
        let position = interval.0 % INTERVALS_PER_SLOT;
        let view = Arc::clone(&self.view.borrow_and_update());

        self.swap_in_advanced_keys().await;
        self.start_due_advances(slot);

        let outcome = if position == 0 {
            self.propose(slot, &view).await
        } else {
            self.attest(slot, &view).await
        };

        if let Err(error) = outcome {
            tracing::warn!(slot = slot.0, interval = interval.0, %error, "duty not performed");
        }
    }

    /// Produces this slot's block, when this node holds the scheduled proposer's key.
    ///
    /// The block is signed here and proved elsewhere: the merged proof leaves for the prover
    /// with everything it needs, and the finished [`SignedBlock`] is sent from that task, so
    /// this one is free again before interval 1.
    async fn propose(&mut self, slot: Slot, view: &ChainView) -> Result<(), DutyError> {
        let head_state = view.head_state().ok_or(DutyError::HeadStateMissing)?;
        let proposer = proposer_for_slot(slot, head_state.validators.len() as u64)?;
        let Some(keys) = self.keyring.keys_for(proposer) else {
            return Ok(());
        };

        let BuiltBlock {
            block, components, ..
        } = build_block(
            head_state,
            slot,
            proposer,
            view.head(),
            &view.known_block_roots(),
            view.known_aggregated_payloads(),
        )?;

        let block_root = hash_tree_root(&block);
        let signature =
            sign(&keys.proposal.secret, slot, &block_root).map_err(DutyError::Signing)?;

        tracing::info!(
            slot = slot.0,
            proposer = proposer.0,
            attestations = block.body.attestations.len(),
            "proposing"
        );

        let job = BlockProofJob {
            validators: head_state.validators.clone(),
            components: block
                .body
                .attestations
                .iter()
                .map(|attestation| attestation.data)
                .zip(components)
                .collect(),
            proposer,
            signature,
            block_root,
            slot,
        };

        let prover = self.prover.clone();
        let products = self.products.clone();
        tokio::spawn(async move {
            match prover.prove(move || job.fold()).await {
                Ok(Ok(proof)) => {
                    // The product channel never sheds: a dropped block is a slot nobody else
                    // can fill.
                    let _ = products
                        .send(LocalProduct::Block(SignedBlock { block, proof }))
                        .await;
                }
                Ok(Err(error)) => {
                    tracing::warn!(slot = slot.0, %error, "block proof could not be built");
                }
                Err(error) => tracing::warn!(slot = slot.0, %error, "block proof abandoned"),
            }
        });

        Ok(())
    }

    /// Casts this slot's vote, once, for every validator this node runs.
    async fn attest(&mut self, slot: Slot, view: &ChainView) -> Result<(), DutyError> {
        if self.attested.contains(&slot) {
            return Ok(());
        }

        // The vote is produced before the slot is marked, so a view that cannot yet answer
        // leaves the slot open for the next interval to retry.
        let data = view.attestation_data(slot)?;
        self.attested.insert(slot);
        self.attested
            .retain(|attested| attested.0 + ATTESTED_SLOT_RETENTION > slot.0);

        let message = hash_tree_root(&data);
        for validator in self.keyring.validators() {
            let signature =
                sign(&validator.attestation.secret, slot, &message).map_err(DutyError::Signing)?;
            let attestation = SignedAttestation {
                validator_index: validator.index,
                data,
                signature,
            };
            if self
                .products
                .send(LocalProduct::Attestation(attestation))
                .await
                .is_err()
            {
                return Ok(());
            }
        }

        tracing::debug!(slot = slot.0, target = data.target.slot.0, "attested");
        Ok(())
    }

    /// Starts a rebuild for every key whose window this slot has passed the midpoint of.
    fn start_due_advances(&mut self, slot: Slot) {
        for (index, role) in self.keyring.advances_due(slot) {
            if self.advancing.holds(index, role) {
                continue;
            }
            let copy = match self.keyring.duplicate(index, role) {
                Ok(Some(copy)) => copy,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(validator = index.0, ?role, %error, "cannot copy a key to advance it");
                    continue;
                }
            };

            tracing::info!(
                validator = index.0,
                ?role,
                slot = slot.0,
                "advancing key preparation"
            );
            let directory = self.keyring.directory().to_path_buf();
            self.advancing.insert(
                index,
                role,
                tokio::task::spawn_blocking(move || advance(&directory, index, role, copy)),
            );
        }
    }

    /// Puts every finished rebuild in place of the key it was made from.
    async fn swap_in_advanced_keys(&mut self) {
        for (index, role, advanced) in self.advancing.reap().await {
            tracing::info!(validator = index.0, ?role, "advanced key in service");
            self.keyring.swap(index, role, advanced);
        }
    }
}

/// The slot an interval count since genesis falls in.
const fn slot_of(interval: Interval) -> Slot {
    Slot(interval.0 / INTERVALS_PER_SLOT)
}

/// Everything the prover needs to turn a signed block into a block with a proof.
///
/// It owns its inputs outright — the registry included — because it runs on the blocking pool
/// and cannot borrow from the duty task it left.
struct BlockProofJob {
    validators: Validators,
    components: Vec<(AttestationData, Vec<SingleMessageAggregate>)>,
    proposer: ValidatorIndex,
    signature: Signature,
    block_root: Bytes32,
    slot: Slot,
}

impl BlockProofJob {
    /// Folds the body's votes and the proposer's signature into the one proof a block carries.
    ///
    /// Component order is the contract: one entry per aggregated attestation in body order,
    /// then a single-element entry for the proposer. A verifier re-parses the merged proof by
    /// that order, so a different one here is a proof nobody can check.
    fn fold(self) -> Result<MultiMessageAggregate, DutyError> {
        let mut components = Vec::with_capacity(self.components.len() + 1);

        for (data, proofs) in self.components {
            let decoded = proofs
                .iter()
                .map(|proof| proofs::decode(proof, &self.validators))
                .collect::<Result<Vec<_>, DutyError>>()?;

            // One proof already covers this vote's voters; folding it again would cost
            // seconds and change nothing.
            components.push(if decoded.len() == 1 {
                decoded.into_iter().next().expect("length checked")
            } else {
                aggregate_single_message(decoded, &[], &hash_tree_root(&data), data.slot)?
            });
        }

        let key = proofs::proposal_key(&self.validators, self.proposer)?;
        components.push(aggregate_single_message(
            Vec::new(),
            &[(key, self.signature)],
            &self.block_root,
            self.slot,
        )?);

        proofs::to_multi_container(&merge_single_message_proofs(components)?)
    }
}
