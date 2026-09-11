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

/// Head lag, in slots, beyond which duties stop: this node's view is stale.
///
/// leanSpec `node/validator/constants.py::SYNC_LAG_THRESHOLD`, the same value ethlambda uses
/// in `crates/blockchain/src/sync_status.rs`. See [`DutyService::serves_duties`].
const DUTY_LAG_THRESHOLD: u64 = 4;

/// Lag of the freshest block *seen* beyond which the network, not this node, is the one that
/// stopped — and duties keep running so the chain can recover.
///
/// leanSpec `NETWORK_STALL_THRESHOLD`, likewise shared with ethlambda.
const NETWORK_STALL_THRESHOLD: u64 = 8;

/// Recovery band that stops the gate flapping at the threshold: once closed it reopens at
/// `DUTY_LAG_THRESHOLD - DUTY_LAG_HYSTERESIS`.
const DUTY_LAG_HYSTERESIS: u64 = 2;

/// The validator client: keys, the duties they owe, and the products they yield.
pub struct DutyService {
    keyring: Keyring,
    prover: Prover,
    products: mpsc::Sender<LocalProduct>,
    view: watch::Receiver<Arc<ChainView>>,
    ticks: watch::Receiver<Interval>,
    /// The lag gate. See [`DutyService::serves_duties`].
    gate: LagGate,
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
            gate: LagGate::default(),
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
            if !self.serves_duties(slot_of(interval)) {
                continue;
            }
            self.on_interval(interval).await;
        }
    }

    /// Whether duties may run for `slot`: the third serving gate.
    ///
    /// # Why lag and not the sync state
    ///
    /// A validator that signs while behind votes for a stale head and spends a one-time XMSS
    /// signature to do it, and unlike a missed duty that spend is not recoverable
    /// (`docs/design/key-management.md`). What the gate must answer is therefore "is this
    /// node's view stale", and the honest measure of that is the node's own head against the
    /// wall clock — not whether it has peers. A node alone at genesis is not behind anything;
    /// a node with fifty peers and a head twenty slots old is.
    ///
    /// This is what every surveyed client does. leanSpec
    /// (`node/validator/service.py::_is_synced_for_duties`), ethlambda
    /// (`crates/blockchain/src/sync_status.rs`) and ream (`chain/lean/src/service.rs`) all
    /// gate on head-versus-clock lag and none of them consults a peer count; the three
    /// constants below are leanSpec's and ethlambda's, which agree exactly.
    ///
    /// # The stall override, and why it is not optional
    ///
    /// If every node stops signing whenever it is behind, a network that pauses can never
    /// restart: every node is behind, so nobody proposes, so every node stays behind. The
    /// freshest block this node has *seen* separates the two cases. Blocks reach the store
    /// only after verification, so a stale maximum is authenticated evidence that the network
    /// is not producing, and the gate opens rather than closing.
    ///
    /// The hysteresis band is what stops the gate flapping around the threshold: once closed
    /// it reopens at `DUTY_LAG_THRESHOLD - DUTY_LAG_HYSTERESIS`, not at the threshold itself.
    fn serves_duties(&mut self, slot: Slot) -> bool {
        let view = self.view.borrow();
        let head_slot = view.head_checkpoint().slot;
        let max_seen = view.max_known_block_slot();
        drop(view);
        self.gate.admits(slot, head_slot, max_seen)
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
/// The duty gate's decision, separated from the service so it can be exercised directly.
///
/// One bit of state — whether the gate is currently closed — which is what makes the
/// hysteresis band expressible: reopening asks a different question from closing.
#[derive(Debug, Default)]
struct LagGate {
    closed: bool,
}

impl LagGate {
    /// Whether duties may run, given the wall-clock slot, this node's head, and the freshest
    /// block it has seen from anyone.
    fn admits(&mut self, slot: Slot, head_slot: Slot, max_seen: Slot) -> bool {
        // Saturating, both of them: a head ahead of the wall clock is local clock drift, not
        // a reason to trust a chain from the future.
        let head_lag = slot.0.saturating_sub(head_slot.0);
        let network_lag = slot.0.saturating_sub(max_seen.0);
        let was_closed = self.closed;

        self.closed = if network_lag > NETWORK_STALL_THRESHOLD {
            false
        } else if self.closed {
            head_lag > DUTY_LAG_THRESHOLD - DUTY_LAG_HYSTERESIS
        } else {
            head_lag > DUTY_LAG_THRESHOLD
        };

        if self.closed != was_closed {
            tracing::info!(
                slot = slot.0,
                head_slot = head_slot.0,
                head_lag,
                network_lag,
                closed = self.closed,
                "validator duty gate changed"
            );
        }
        !self.closed
    }
}

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

#[cfg(test)]
mod tests {
    use verity_types::Slot;

    use super::{DUTY_LAG_THRESHOLD, LagGate, NETWORK_STALL_THRESHOLD};

    /// A node whose head keeps up with the clock, which is also the case of a node alone on
    /// the network at genesis: nothing is behind anything.
    #[test]
    fn should_serve_duties_while_the_head_keeps_up() {
        let mut gate = LagGate::default();
        for lag in 0..=DUTY_LAG_THRESHOLD {
            let slot = Slot(100 + lag);
            assert!(
                gate.admits(slot, Slot(100), slot),
                "lag {lag} is within the threshold"
            );
        }
    }

    #[test]
    fn should_serve_duties_with_no_peers_at_genesis() {
        let mut gate = LagGate::default();
        assert!(gate.admits(Slot(0), Slot(0), Slot(0)));
    }

    #[test]
    fn should_stop_signing_once_the_head_falls_behind() {
        let mut gate = LagGate::default();
        let slot = Slot(100 + DUTY_LAG_THRESHOLD + 1);
        assert!(!gate.admits(slot, Slot(100), slot));
    }

    #[test]
    fn should_reopen_only_below_the_hysteresis_band() {
        let mut gate = LagGate::default();
        // Close it.
        assert!(!gate.admits(Slot(110), Slot(100), Slot(110)));
        // Three slots behind: inside the threshold, but not yet inside the band.
        assert!(!gate.admits(Slot(103), Slot(100), Slot(103)));
        // Two slots behind: the band, so the gate reopens.
        assert!(gate.admits(Slot(102), Slot(100), Slot(102)));
    }

    /// A network-wide stall is the case the gate must not make worse: if every node stopped
    /// signing because every node is behind, nothing would ever propose again.
    #[test]
    fn should_keep_signing_when_the_network_itself_has_stopped() {
        let mut gate = LagGate::default();
        let slot = Slot(100 + NETWORK_STALL_THRESHOLD + 1);
        // The head is far behind the clock, but so is the freshest block anyone has produced.
        assert!(gate.admits(slot, Slot(100), Slot(100)));
    }

    /// The same lag, with the network visibly producing, is this node's problem.
    #[test]
    fn should_stop_signing_when_the_network_moves_and_this_node_does_not() {
        let mut gate = LagGate::default();
        let slot = Slot(100 + NETWORK_STALL_THRESHOLD + 1);
        assert!(!gate.admits(slot, Slot(100), slot));
    }
}
