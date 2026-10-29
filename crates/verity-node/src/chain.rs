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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use tokio::sync::{mpsc, watch};

use verity_chain::{
    ChainView, Store, hash_tree_root, on_block_observed, on_tick, record_aggregated_payload,
    record_attestation_signature,
};
use verity_crypto::containers::SignedAttestation;
use verity_db::{BlockCommit, Repository, StorageBackend, StorageError, TickCommit};
use verity_metrics::{Metrics, SkipReason, observe_elapsed};
use verity_types::config::INTERVALS_PER_SLOT;
use verity_types::{
    AttestationData, Block, Bytes32, Interval, MultiMessageAggregate, SignedAggregatedAttestation,
    ValidatorIndex,
};
use verity_validator::{LocalProduct, Prover};

use crate::observe::{TransitionRecorder, reorg_depth};
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
    /// Whether the role is on. Shared with the admin API, which flips it at runtime; read
    /// once per round, so a flip lands on the next interval 2 rather than mid-round.
    pub enabled: Arc<AtomicBool>,
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
    metrics: Arc<Metrics>,
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
        metrics: Arc<Metrics>,
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
                metrics,
            },
            receiver,
        )
    }

    /// Runs until every inbox has closed.
    ///
    /// Closure is the whole shutdown protocol: the binary stops the producers at the edge,
    /// each stopped producer drops its sender, and this loop ends once all three are gone.
    /// Duty products are drained to the end — they are never dropped, shutdown included.
    ///
    /// # Errors
    ///
    /// Returns the first storage failure. No snapshot containing the uncommitted mutation is
    /// published, and the node runtime treats the error as fatal.
    pub async fn run(mut self) -> Result<(), StorageError> {
        // The clock's current interval is already in the watch when this task starts, and
        // `changed()` below waits for the *next* one. Consuming it here is what stops a node
        // that starts mid-chain from spending its first interval believing it is still at
        // genesis: for those few hundred milliseconds every block the sync service fetches is
        // ahead of the store's own clock, and a block refused as too far in the future is
        // gone — nothing re-requests it, because it failed after the verification stage
        // rather than in it.
        let start = *self.clock.borrow_and_update();
        self.on_clock(start).await?;
        self.publish();

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

            let head_before = self.store.head;
            match event {
                Event::Tick => {
                    let target = *self.clock.borrow_and_update();
                    self.on_clock(target).await?;
                }
                Event::Local(product) => self.on_local(product)?,
                Event::Verified(verified) => self.on_verified(verified)?,
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

            self.record_head_move(head_before);
            self.publish();
        }

        tracing::info!(head = %hex(self.store.head), "chain task stopped");
        Ok(())
    }

    /// Counts a reorg when the event just applied moved the head off its own chain.
    ///
    /// Every event that can move the head — an import, or a tick that promotes votes — ends
    /// up here, which is what makes the count complete rather than an import-only view.
    fn record_head_move(&self, head_before: Bytes32) {
        let Some(depth) = reorg_depth(&self.store.blocks, head_before, self.store.head) else {
            return;
        };
        self.metrics.fork_choice.record_reorg(depth);
        tracing::info!(
            from = %hex(head_before),
            to = %hex(self.store.head),
            depth,
            "fork choice reorg"
        );
    }

    /// Records one vote's admission decision: how long it took, and which way it went.
    fn record_validation(&self, started: Instant, valid: bool) {
        let fork_choice = &self.metrics.fork_choice;
        observe_elapsed(&fork_choice.attestation_validation_time_seconds, started);
        if valid {
            fork_choice.attestations_valid_total.inc();
        } else {
            fork_choice.attestations_invalid_total.inc();
        }
    }

    /// Advances consensus time to `target`, running every interval on the way.
    ///
    /// Never only the latest one: `on_tick` steps interval by interval so no interval's
    /// action is skipped, which is what makes the latest-only clock channel sound.
    ///
    /// `has_proposal` is false here, matching leanSpec's own node: interval 0's acceptance is
    /// the proposer's private step, and the tick loop of a following node does not take it.
    async fn on_clock(&mut self, target: Interval) -> Result<(), StorageError> {
        if target.0 <= self.store.time.0 {
            return Ok(());
        }

        while self.store.time.0 < target.0 {
            let next = Interval(self.store.time.0 + 1);
            on_tick(&mut self.store, next, false);
            self.commit_interval(next)?;
            if next.0 % INTERVALS_PER_SLOT == 2 {
                self.start_aggregation_round();
            }
        }
        Ok(())
    }

    /// Persists what an interval changed, in the one batch a restart reads back.
    fn commit_interval(&mut self, interval: Interval) -> Result<(), StorageError> {
        let position = interval.0 % INTERVALS_PER_SLOT;
        let tick = TickCommit {
            head: self.store.head,
            safe_target: (position == 3).then_some(self.store.safe_target),
            latest_justified: self.store.latest_justified,
            latest_finalized: self.store.latest_finalized,
            interval,
            merge_pending_votes: position == 4,
        };
        self.repository.commit_tick(&tick)
    }

    /// Hands the slot's evidence to the aggregation worker, and does not wait for it.
    ///
    /// Proving takes seconds — far longer than the interval it starts in — so the round runs
    /// on the prover and its output re-enters through channel ② whenever it finishes.
    fn start_aggregation_round(&self) {
        let skipped = |reason| self.metrics.validator.record_aggregation_skipped(reason);
        let Some(aggregator) = &self.aggregator else {
            skipped(SkipReason::NotAggregator);
            return;
        };
        if !aggregator.enabled.load(Ordering::Relaxed) {
            skipped(SkipReason::NotAggregator);
            return;
        }

        let view = Arc::new(ChainView::of(&self.store));
        let prover = aggregator.prover.clone();
        let products = aggregator.products.clone();
        let metrics = Arc::clone(&self.metrics);

        tokio::spawn(async move {
            let round_metrics = Arc::clone(&metrics);
            let round = prover.prove(move || {
                let started = Instant::now();
                let aggregates = verity_validator::aggregate(&view, &round_metrics);
                observe_elapsed(
                    &round_metrics
                        .fork_choice
                        .committee_signatures_aggregation_time_seconds,
                    started,
                );
                aggregates
            });
            match round.await {
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
                Err(error) => {
                    metrics
                        .validator
                        .record_aggregation_skipped(SkipReason::SpawnFailed);
                    tracing::warn!(%error, "aggregation round abandoned");
                }
            }
        });
    }

    /// Applies one of this node's own duty products.
    ///
    /// These skip verification by construction: they were produced from this node's own view,
    /// with this node's own keys, by the same functions that would check them.
    fn on_local(&mut self, product: LocalProduct) -> Result<(), StorageError> {
        match product {
            LocalProduct::Block(signed) => {
                let root = hash_tree_root(&signed.block);
                self.import(root, signed.block, signed.proof)
            }
            LocalProduct::Attestation(signed) => {
                self.record_vote(signed);
                Ok(())
            }
            LocalProduct::Aggregate(aggregate) => self.record_aggregate(aggregate),
        }
    }

    /// Applies one item the verification stage has cleared.
    fn on_verified(&mut self, verified: Verified) -> Result<(), StorageError> {
        match verified {
            Verified::Block(block) => {
                let (root, block, proof) = block.into_parts();
                self.import(root, block, proof)?;
            }
            Verified::Attestation(attestation) => {
                let (validator_index, data, signature) = attestation.into_parts();
                let started = Instant::now();
                let outcome =
                    record_attestation_signature(&mut self.store, validator_index, data, signature);
                self.record_validation(started, outcome.is_ok());
                if let Err(reason) = outcome {
                    tracing::debug!(validator = validator_index.0, %reason, "vote not pooled");
                }
            }
            Verified::Aggregate(aggregate) => self.record_aggregate(aggregate.into_inner())?,
        }
        Ok(())
    }

    /// Imports a block and persists it with the proof that made it acceptable.
    ///
    /// The store is written first and the database second, deliberately: `on_block` is the
    /// thing that can reject, and it leaves the store untouched when it does, so nothing
    /// unacceptable ever reaches a batch.
    fn import(
        &mut self,
        root: Bytes32,
        block: Block,
        proof: MultiMessageAggregate,
    ) -> Result<(), StorageError> {
        // The verification stage drops what the snapshot already holds, but a block verified
        // just before this import's own snapshot was published slips past it. `on_block` is
        // a no-op for a known root; the commit and the log would not be.
        if self.store.blocks.contains_key(&root) {
            tracing::debug!(slot = block.slot.0, "block already imported");
            return Ok(());
        }

        let parent_slot = self
            .store
            .blocks
            .get(&block.parent_root)
            .map(|parent| parent.slot);

        let started = Instant::now();
        let mut recorder = TransitionRecorder::new(&self.metrics);
        let outcome = on_block_observed(&mut self.store, &block, &mut recorder);
        observe_elapsed(
            &self.metrics.fork_choice.block_processing_time_seconds,
            started,
        );
        if let Err(reason) = outcome {
            tracing::debug!(slot = block.slot.0, %reason, "block not imported");
            return Ok(());
        }

        let Some(post_state) = self.store.states.get(&root) else {
            // `on_block` inserts the post-state for every block it accepts, so this is only
            // reachable for a block that was already in the store — nothing left to persist.
            return Ok(());
        };
        let Some(parent_slot) = parent_slot else {
            return Ok(());
        };

        let commit = BlockCommit {
            block_root: root,
            body: &block.body,
            proof: &proof,
            post_state,
            parent_slot,
        };
        self.repository.commit_block(&commit)?;

        tracing::info!(
            slot = block.slot.0,
            head = %hex(self.store.head),
            justified = self.store.latest_justified.slot.0,
            finalized = self.store.latest_finalized.slot.0,
            "imported"
        );
        Ok(())
    }

    /// Pools this node's own vote, so its own aggregation round can fold it in.
    fn record_vote(&mut self, signed: SignedAttestation) {
        let signature =
            verity_chain::AttestationSignature(libssz::SszEncode::to_ssz(&signed.signature));
        let started = Instant::now();
        let outcome = record_attestation_signature(
            &mut self.store,
            signed.validator_index,
            signed.data,
            signature,
        );
        self.record_validation(started, outcome.is_ok());
        if let Err(reason) = outcome {
            tracing::debug!(validator = signed.validator_index.0, %reason, "own vote not pooled");
        }
    }

    /// Records an aggregate and drops the raw signatures it absorbed.
    ///
    /// Dropping them is leanSpec's own step at the end of an aggregation round: the proof now
    /// covers those voters, so keeping the raw copies would only have later rounds re-prove
    /// what is already proved.
    fn record_aggregate(
        &mut self,
        aggregate: SignedAggregatedAttestation,
    ) -> Result<(), StorageError> {
        let started = Instant::now();
        let outcome = record_aggregated_payload(&mut self.store, &aggregate);
        self.record_validation(started, outcome.is_ok());
        if let Err(reason) = outcome {
            tracing::debug!(slot = aggregate.data.slot.0, %reason, "aggregate not recorded");
            return Ok(());
        }
        self.store.attestation_signatures.remove(&aggregate.data);
        self.persist_pending_votes(&aggregate)
    }

    /// Writes the votes an aggregate carries, so a restart keeps the weight it gave the head.
    fn persist_pending_votes(
        &mut self,
        aggregate: &SignedAggregatedAttestation,
    ) -> Result<(), StorageError> {
        let votes: Vec<(ValidatorIndex, AttestationData)> =
            verity_chain::fork_choice::participants(&aggregate.proof.participants)
                .map(|validator| (validator, aggregate.data))
                .collect();

        self.repository
            .record_pending_votes(&votes, self.store.time)
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::sync::{mpsc, watch};
    use verity_chain::{Store, generate_genesis, hash_tree_root, process_block, process_slots};
    use verity_db::{
        AnchorCommit, ColumnFamily, Durability, Identity, MemoryBackend, Repository,
        StorageBackend, StorageError, StorageReader, WriteBatch, stored_header,
    };
    use verity_metrics::Metrics;
    use verity_types::{
        AggregationBits, AttestationData, Block, BlockBody, Interval, MultiMessageAggregate,
        SignedAggregatedAttestation, SingleMessageAggregate, Slot, Validator, ValidatorIndex,
        Validators,
    };

    use crate::store_open::block_from;

    use super::{ChainTask, hex};

    #[derive(Debug, Clone)]
    struct FailingBackend {
        inner: MemoryBackend,
        fail_writes: Arc<AtomicBool>,
    }

    impl StorageReader for FailingBackend {
        fn get(&self, table: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
            self.inner.get(table, key)
        }

        fn table_is_empty(&self, table: ColumnFamily) -> Result<bool, StorageError> {
            self.inner.table_is_empty(table)
        }

        fn range(
            &self,
            table: ColumnFamily,
            start: &[u8],
            end: &[u8],
        ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
            self.inner.range(table, start, end)
        }
    }

    impl StorageBackend for FailingBackend {
        fn write(&mut self, batch: WriteBatch, durability: Durability) -> Result<(), StorageError> {
            if self.fail_writes.load(Ordering::Relaxed) {
                return Err(StorageError::Backend("injected write failure".to_string()));
            }
            self.inner.write(batch, durability)
        }
    }

    type TaskParts<B> = (
        ChainTask<B>,
        watch::Sender<Interval>,
        mpsc::Sender<verity_validator::LocalProduct>,
        mpsc::Sender<crate::verification::Verified>,
        watch::Receiver<Arc<verity_chain::ChainView>>,
    );

    fn task_with_backend<B: StorageBackend>(backend: B) -> TaskParts<B> {
        let validators = Validators::try_from(vec![Validator {
            attestation_public_key: [1; 52],
            proposal_public_key: [2; 52],
            index: ValidatorIndex(0),
        }])
        .expect("one validator");
        let state = generate_genesis(0, validators);
        let body = BlockBody::default();
        let block = block_from(&stored_header(&state), body.clone());
        let store = Store::new(&state, &block, None).expect("anchor");
        let mut repository = Repository::open(
            backend,
            Identity {
                chain_fingerprint: hash_tree_root(&state),
                fork_version: 1,
            },
        )
        .expect("empty repository");
        repository
            .commit_anchor(&AnchorCommit {
                block_root: hash_tree_root(&block),
                state: &state,
                body: Some(&body),
                served_from_slot: state.slot,
            })
            .expect("stored anchor");
        let (clock, clock_stream) = watch::channel(Interval(0));
        let (local, local_stream) = mpsc::channel(1);
        let (network, network_stream) = mpsc::channel(1);
        let (task, view) = ChainTask::new(
            store,
            repository,
            clock_stream,
            local_stream,
            network_stream,
            None,
            Arc::new(Metrics::new().expect("registry")),
        );
        (task, clock, local, network, view)
    }

    fn task() -> (
        ChainTask<MemoryBackend>,
        watch::Sender<Interval>,
        mpsc::Sender<verity_validator::LocalProduct>,
        mpsc::Sender<crate::verification::Verified>,
    ) {
        let (task, clock, local, network, _view) = task_with_backend(MemoryBackend::new());
        (task, clock, local, network)
    }

    #[tokio::test]
    async fn should_stop_without_publishing_when_tick_persistence_fails() {
        let fail_writes = Arc::new(AtomicBool::new(false));
        let backend = FailingBackend {
            inner: MemoryBackend::new(),
            fail_writes: Arc::clone(&fail_writes),
        };
        let (task, clock, _local, _network, view) = task_with_backend(backend);
        fail_writes.store(true, Ordering::Relaxed);
        clock.send_replace(Interval(1));

        let error = tokio::time::timeout(std::time::Duration::from_millis(100), task.run())
            .await
            .expect("persistence failure stops the chain task")
            .expect_err("the injected write must fail");

        assert!(matches!(error, StorageError::Backend(_)));
        assert_eq!(view.borrow().time(), Interval(0));
    }

    #[tokio::test]
    async fn should_propagate_block_persistence_failures() {
        let fail_writes = Arc::new(AtomicBool::new(false));
        let backend = FailingBackend {
            inner: MemoryBackend::new(),
            fail_writes: Arc::clone(&fail_writes),
        };
        let (mut task, _, _, _, _) = task_with_backend(backend);
        task.on_clock(Interval(5))
            .await
            .expect("clock setup persists");
        let parent = task.store.states.get(&task.store.head).expect("head state");
        let advanced = process_slots(parent, Slot(1)).expect("advance to block slot");
        let mut block = Block {
            slot: Slot(1),
            proposer_index: ValidatorIndex(0),
            parent_root: hash_tree_root(&advanced.latest_block_header),
            state_root: [0; 32],
            body: BlockBody::default(),
        };
        let post = process_block(&advanced, &block).expect("valid empty block");
        block.state_root = hash_tree_root(&post);
        let root = hash_tree_root(&block);
        fail_writes.store(true, Ordering::Relaxed);

        let error = task
            .import(root, block, MultiMessageAggregate::default())
            .expect_err("the injected block write must fail");

        assert!(matches!(error, StorageError::Backend(_)));
    }

    #[test]
    fn should_propagate_pending_vote_persistence_failures() {
        let fail_writes = Arc::new(AtomicBool::new(false));
        let backend = FailingBackend {
            inner: MemoryBackend::new(),
            fail_writes: Arc::clone(&fail_writes),
        };
        let (mut task, _, _, _, _) = task_with_backend(backend);
        let aggregate = SignedAggregatedAttestation {
            data: AttestationData::default(),
            proof: SingleMessageAggregate {
                participants: AggregationBits::try_from(vec![true]).expect("one participant"),
                ..SingleMessageAggregate::default()
            },
        };
        fail_writes.store(true, Ordering::Relaxed);

        let error = task
            .persist_pending_votes(&aggregate)
            .expect_err("the injected vote write must fail");

        assert!(matches!(error, StorageError::Backend(_)));
    }

    #[tokio::test]
    async fn should_run_every_interval_action_when_the_clock_coalesces_ticks() {
        let (mut task, _, _, _) = task();

        task.on_clock(Interval(8))
            .await
            .expect("memory persistence succeeds");

        assert_eq!(task.store.time, Interval(8));
        assert_eq!(
            task.metrics
                .validator
                .aggregator_skipped_total
                .with_label_values(&["not_aggregator"])
                .get(),
            2,
            "interval 2 must run once in each crossed slot"
        );
        assert_eq!(
            task.repository.last_processed_interval().unwrap(),
            Some(Interval(8))
        );
    }

    #[tokio::test]
    async fn should_stop_after_all_three_inboxes_close() {
        let (task, clock, local, network) = task();
        drop(clock);
        drop(local);
        drop(network);
        tokio::time::timeout(std::time::Duration::from_millis(100), task.run())
            .await
            .expect("closed inputs stop the chain task")
            .expect("memory persistence succeeds");
    }

    #[test]
    fn should_render_only_the_first_four_root_bytes_for_logs() {
        assert_eq!(hex([0xab; 32]), "abababab");
    }
}
