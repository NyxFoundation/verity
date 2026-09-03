//! The one place in the process where a proof may be built.
//!
//! # Why this type exists at all
//!
//! leanVM's prover allocates from a single arena per process: two proofs built concurrently
//! corrupt each other's buffers, however many cores are free (`verity_crypto::aggregate`).
//! The invariant to hold is therefore *per process* — never two proofs at once — and not
//! anything about which validator is doing what.
//!
//! The node has two producers of proofs, block production at interval 0 and the aggregation
//! worker at interval 2, and they reach the prover together for two independent reasons:
//!
//! - **Across slots.** Proving takes seconds and a slot is four, so an aggregation started at
//!   slot N's interval 2 can still be running when slot N+1's interval 0 starts proposing.
//! - **Within one slot.** A node runs whatever validators `validators.yaml` assigns it, and
//!   aggregation is a static node-level role rather than a per-slot selection — leanSpec is
//!   explicit that "aggregator selection is static (node-level flag), not VRF-based
//!   rotation". So an aggregating node aggregates in every slot, including the ones it also
//!   proposes in, whether or not the same validator holds both.
//!
//! Serialization is a correctness requirement, not a throughput policy, and it has to live
//! somewhere both producers pass through.
//!
//! # Where the work runs
//!
//! On `spawn_blocking`, always. Proving takes seconds; running it on a runtime worker would
//! stall every other task on that thread, and the design puts exactly this class of work off
//! the async threads (`docs/design/concurrency.md`, Decision 2).

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::error::DutyError;

/// A shared handle to the process's single prover.
///
/// Cloning shares the permit rather than creating a second prover, which is the entire point:
/// every clone queues behind the same one.
#[derive(Debug, Clone)]
pub struct Prover {
    permit: Arc<Semaphore>,
}

impl Default for Prover {
    fn default() -> Self {
        Self::new()
    }
}

impl Prover {
    /// A prover no job has queued on yet.
    #[must_use = "a prover is a queue; holding one is what serializes the jobs"]
    pub fn new() -> Self {
        Self {
            permit: Arc::new(Semaphore::new(1)),
        }
    }

    /// Compiles the aggregation circuit and warms up the prover.
    ///
    /// Worth paying once at startup: without it the node's first aggregation pays for circuit
    /// compilation, arena setup, and DFT precomputation on top of its own proof, inside a
    /// four-second slot.
    ///
    /// # Errors
    ///
    /// [`DutyError::ProverStopped`] when the runtime is shutting down.
    pub async fn warm_up(&self) -> Result<(), DutyError> {
        self.prove(verity_crypto::aggregate::init_prover).await
    }

    /// Runs one proving job, waiting for any job already running to finish.
    ///
    /// # Errors
    ///
    /// [`DutyError::ProverStopped`] when the runtime is shutting down, which is the only way
    /// the blocking pool or the permit can go away.
    pub async fn prove<T, F>(&self, job: F) -> Result<T, DutyError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let _permit = self
            .permit
            .acquire()
            .await
            .map_err(|_| DutyError::ProverStopped)?;

        tokio::task::spawn_blocking(job)
            .await
            .map_err(|_| DutyError::ProverStopped)
    }
}
