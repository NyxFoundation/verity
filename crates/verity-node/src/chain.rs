//! The single writer: the one task that owns consensus state.
//!
//! # Why one task, and not a lock
//!
//! With the store owned outright, single-writer stops being a property anyone has to check
//! and becomes an aliasing-xor-mutability fact. Every call into the state transition is
//! serialized structurally, every event lands in one linear order, and the interleaving space
//! left for model checking is the channel endpoints rather than every read-modify-write
//! inside the store (`docs/design/concurrency.md`, Decision 1).
//!
//! # Three inboxes, biased
//!
//! ① the clock, ② this node's own duty products, ③ verified network input — read in that
//! order, one event per iteration, re-entering the `select!` every time. The order is the
//! design stated directly: time and our own duties are handled ahead of any volume of gossip.
//! The channels differ because their full-queue behaviour has to: ① coalesces, ② is never
//! dropped because nobody else holds our signatures, and ③ propagates backpressure to the
//! network edge, where raw bytes — not verified work — are what gets shed.
//!
//! # Reads leave as snapshots
//!
//! There is no query channel inbound. After every completed event the task publishes an
//! `Arc<ChainView>`, and that snapshot is the entire read path for duties, verification, and
//! anything added later.

use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use verity_chain::{
    ChainView, Store, hash_tree_root, on_block, on_tick, record_aggregated_payload,
    record_attestation_signature,
};
use verity_crypto::containers::SignedAttestation;
use verity_db::{BlockCommit, Repository, StorageBackend, TickCommit};
use verity_types::config::INTERVALS_PER_SLOT;
use verity_types::{
    AttestationData, Block, Bytes32, Interval, MultiMessageAggregate, SignedAggregatedAttestation,
    ValidatorIndex,
};
use verity_validator::{LocalProduct, Prover};

use crate::verification::Verified;

/// What the chain task needs in order to hand an aggregation round off at interval 2.
///
/// Absent on a node that does not aggregate, which is why it is an `Option` rather than a
/// flag: a non-aggregator has nowhere to send a round's output and no reason to run one.
pub struct Aggregator {
    /// The process's single prover, shared with block production.
    pub prover: Prover,
    /// Where a finished round's aggregates re-enter — channel ②, like any other duty product.
    pub products: mpsc::Sender<LocalProduct>,
}

/// The task that owns `Store` and the repository, and the only thing that writes either.
pub struct ChainTask<B: StorageBackend> {
    store: Store,
    repository: Repository<B>,
    view: watch::Sender<Arc<ChainView>>,
    clock: watch::Receiver<Interval>,
    local: mpsc::Receiver<LocalProduct>,
    network: mpsc::Receiver<Verified>,
    aggregator: Option<Aggregator>,
}

impl<B: StorageBackend> ChainTask<B> {
    /// Takes ownership of an opened repository and the store rebuilt from it.
    ///
    /// The view channel is created here, already holding the first snapshot, so no consumer
    /// can exist before a real `ChainView` does — the readiness gate of
    /// `docs/design/concurrency.md` becomes structural rather than a state to wait for.
    #[must_use = "a chain task does nothing until it is run"]
    pub fn new(
        store: Store,
        repository: Repository<B>,
        clock: watch::Receiver<Interval>,
        local: mpsc::Receiver<LocalProduct>,
        network: mpsc::Receiver<Verified>,
        aggregator: Option<Aggregator>,
    ) -> (Self, watch::Receiver<Arc<ChainView>>) {
        let (view, receiver) = watch::channel(Arc::new(ChainView::of(&store)));
        (
            Self {
                store,
                repository,
                view,
                clock,
                local,
                network,
                aggregator,
            },
            receiver,
        )
    }

    /// Runs until every inbox has closed.
    ///
    /// Closure is the whole shutdown protocol: the binary stops the producers at the edge,
    /// each stopped producer drops its sender, and this loop ends once all three are gone.
    /// Duty products are drained to the end — they are never dropped, shutdown included.
    pub async fn run(mut self) {
        let mut clock_open = true;
        let mut local_open = true;
        let mut network_open = true;

        while clock_open || local_open || network_open {
            // The `select!` yields a value and nothing more: its futures each borrow one
            // field, and letting them end before the handler runs is what gives the handler
            // `&mut self` back.
            let event = tokio::select! {
                biased;

                changed = self.clock.changed(), if clock_open =>
                    changed.map_or(Event::ClockClosed, |()| Event::Tick),

                product = self.local.recv(), if local_open =>
                    product.map_or(Event::LocalClosed, Event::Local),

                verified = self.network.recv(), if network_open =>
                    verified.map_or(Event::NetworkClosed, Event::Verified),
            };

            match event {
                Event::Tick => {
                    let target = *self.clock.borrow_and_update();
                    self.on_clock(target).await;
                }
                Event::Local(product) => self.on_local(product),
                Event::Verified(verified) => self.on_verified(verified),
                Event::ClockClosed => {
                    clock_open = false;
                    // The aggregation handoff holds a sender into channel ②. Keeping it past
                    // the clock would stop the relay's stream from ever closing, and this
                    // task would then wait on an inbox nothing can fill.
                    self.aggregator = None;
                    continue;
                }
                Event::LocalClosed => {
                    local_open = false;
                    continue;
                }
                Event::NetworkClosed => {
                    network_open = false;
                    continue;
                }
            }

            self.publish();
        }

        tracing::info!(head = %hex(self.store.head), "chain task stopped");
    }

    /// Advances consensus time to `target`, running every interval on the way.
    ///
    /// Never only the latest one: `on_tick` steps interval by interval so no interval's
    /// action is skipped, which is what makes the latest-only clock channel sound.
    ///
    /// `has_proposal` is false here, matching leanSpec's own node: interval 0's acceptance is
    /// the proposer's private step, and the tick loop of a following node does not take it.
    async fn on_clock(&mut self, target: Interval) {
        if target.0 <= self.store.time.0 {
            return;
        }

        while self.store.time.0 < target.0 {
            let next = Interval(self.store.time.0 + 1);
            on_tick(&mut self.store, next, false);
            self.commit_interval(next);
        }

        if target.0 % INTERVALS_PER_SLOT == 2 {
            self.start_aggregation_round();
        }
    }

    /// Persists what an interval changed, in the one batch a restart reads back.
    fn commit_interval(&mut self, interval: Interval) {
        let position = interval.0 % INTERVALS_PER_SLOT;

        if position == 3
            && let Err(error) = self
                .repository
                .commit_safe_target(self.store.safe_target, interval)
        {
            tracing::error!(%error, interval = interval.0, "cannot persist the safe target");
        }

        let tick = TickCommit {
            head: self.store.head,
            latest_justified: self.store.latest_justified,
            latest_finalized: self.store.latest_finalized,
            interval,
            merge_pending_votes: position == 4,
        };
        if let Err(error) = self.repository.commit_tick(&tick) {
            tracing::error!(%error, interval = interval.0, "cannot persist the interval tick");
        }
    }

    /// Hands the slot's evidence to the aggregation worker, and does not wait for it.
    ///
    /// Proving takes seconds — far longer than the interval it starts in — so the round runs
    /// on the prover and its output re-enters through channel ② whenever it finishes.
    fn start_aggregation_round(&self) {
        let Some(aggregator) = &self.aggregator else {
            return;
        };

        let view = Arc::new(ChainView::of(&self.store));
        let prover = aggregator.prover.clone();
        let products = aggregator.products.clone();

        tokio::spawn(async move {
            match prover
                .prove(move || verity_validator::aggregate(&view))
                .await
            {
                Ok(aggregates) => {
                    for aggregate in aggregates {
                        if products
                            .send(LocalProduct::Aggregate(aggregate))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "aggregation round abandoned"),
            }
        });
    }

    /// Applies one of this node's own duty products.
    ///
    /// These skip verification by construction: they were produced from this node's own view,
    /// with this node's own keys, by the same functions that would check them.
    fn on_local(&mut self, product: LocalProduct) {
        match product {
            LocalProduct::Block(signed) => {
                let root = hash_tree_root(&signed.block);
                self.import(root, signed.block, signed.proof);
            }
            LocalProduct::Attestation(signed) => self.record_vote(signed),
            LocalProduct::Aggregate(aggregate) => self.record_aggregate(aggregate),
        }
    }

    /// Applies one item the verification stage has cleared.
    fn on_verified(&mut self, verified: Verified) {
        match verified {
            Verified::Block(block) => {
                let (root, block, proof) = block.into_parts();
                self.import(root, block, proof);
            }
            Verified::Attestation(attestation) => {
                let (validator_index, data, signature) = attestation.into_parts();
                if let Err(reason) =
                    record_attestation_signature(&mut self.store, validator_index, data, signature)
                {
                    tracing::debug!(validator = validator_index.0, %reason, "vote not pooled");
                }
            }
            Verified::Aggregate(aggregate) => self.record_aggregate(aggregate.into_inner()),
        }
    }

    /// Imports a block and persists it with the proof that made it acceptable.
    ///
    /// The store is written first and the database second, deliberately: `on_block` is the
    /// thing that can reject, and it leaves the store untouched when it does, so nothing
    /// unacceptable ever reaches a batch.
    fn import(&mut self, root: Bytes32, block: Block, proof: MultiMessageAggregate) {
        let parent_slot = self
            .store
            .blocks
            .get(&block.parent_root)
            .map(|parent| parent.slot);

        if let Err(reason) = on_block(&mut self.store, &block) {
            tracing::debug!(slot = block.slot.0, %reason, "block not imported");
            return;
        }

        let Some(post_state) = self.store.states.get(&root) else {
            // `on_block` inserts the post-state for every block it accepts, so this is only
            // reachable for a block that was already in the store — nothing left to persist.
            return;
        };
        let Some(parent_slot) = parent_slot else {
            return;
        };

        let commit = BlockCommit {
            block_root: root,
            body: &block.body,
            proof: &proof,
            post_state,
            parent_slot,
        };
        if let Err(error) = self.repository.commit_block(&commit) {
            tracing::error!(%error, slot = block.slot.0, "cannot persist an imported block");
            return;
        }

        tracing::info!(
            slot = block.slot.0,
            head = %hex(self.store.head),
            justified = self.store.latest_justified.slot.0,
            finalized = self.store.latest_finalized.slot.0,
            "imported"
        );
    }

    /// Pools this node's own vote, so its own aggregation round can fold it in.
    fn record_vote(&mut self, signed: SignedAttestation) {
        let signature =
            verity_chain::AttestationSignature(libssz::SszEncode::to_ssz(&signed.signature));
        if let Err(reason) = record_attestation_signature(
            &mut self.store,
            signed.validator_index,
            signed.data,
            signature,
        ) {
            tracing::debug!(validator = signed.validator_index.0, %reason, "own vote not pooled");
        }
    }

    /// Records an aggregate and drops the raw signatures it absorbed.
    ///
    /// Dropping them is leanSpec's own step at the end of an aggregation round: the proof now
    /// covers those voters, so keeping the raw copies would only have later rounds re-prove
    /// what is already proved.
    fn record_aggregate(&mut self, aggregate: SignedAggregatedAttestation) {
        if let Err(reason) = record_aggregated_payload(&mut self.store, &aggregate) {
            tracing::debug!(slot = aggregate.data.slot.0, %reason, "aggregate not recorded");
            return;
        }
        self.store.attestation_signatures.remove(&aggregate.data);
        self.persist_pending_votes(&aggregate);
    }

    /// Writes the votes an aggregate carries, so a restart keeps the weight it gave the head.
    fn persist_pending_votes(&mut self, aggregate: &SignedAggregatedAttestation) {
        let votes: Vec<(ValidatorIndex, AttestationData)> =
            verity_chain::fork_choice::participants(&aggregate.proof.participants)
                .map(|validator| (validator, aggregate.data))
                .collect();

        if let Err(error) = self
            .repository
            .record_pending_votes(&votes, self.store.time)
        {
            tracing::error!(%error, "cannot persist the votes an aggregate carries");
        }
    }

    /// Publishes the snapshot every reader answers from.
    ///
    /// Once per loop iteration, after the event has fully applied — never mid-import, so no
    /// reader can see a half-applied mutation.
    fn publish(&self) {
        // A send fails only when every reader is gone, which means shutdown is under way.
        let _ = self.view.send(Arc::new(ChainView::of(&self.store)));
    }
}

/// One iteration's worth of work, taken off whichever inbox produced it first.
enum Event {
    /// The clock moved; the target is read from the channel by the handler.
    Tick,
    /// A duty this node performed.
    Local(LocalProduct),
    /// Something a peer sent, already verified.
    Verified(Verified),
    /// The clock stopped, which is how shutdown starts.
    ClockClosed,
    /// The validator client stopped producing.
    LocalClosed,
    /// The verification stage stopped.
    NetworkClosed,
}

/// The first four bytes of a root, which is what a log line needs to tell heads apart.
fn hex(root: Bytes32) -> String {
    root.iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
