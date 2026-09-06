//! The keys this node signs with, and keeping them able to sign.
//!
//! # The two schedules
//!
//! An XMSS key can only sign inside its *prepared window* — two bottom trees, about six days
//! at four-second slots — and moving that window rebuilds 65,536 one-time chains. There are
//! two moments that has to happen:
//!
//! - **At startup**, once, until the current slot is inside the window. A node that was down
//!   for a year owes one rebuild per three days of downtime, which is why the advanced key is
//!   written back to disk: the catch-up is then bounded by the downtime, not by the key's age.
//! - **In steady state**, when the slot passes the window's midpoint, which is leanSig's own
//!   intended cadence of about once every three days per key.
//!
//! # Clone-advance-swap
//!
//! `advance_preparation` needs `&mut`, so handing the live key to a worker would stall
//! signing for the rebuild's duration — the failure mode zeam demonstrates. Instead the
//! worker gets a *copy*, the original keeps signing while the rebuild runs, and the swap is a
//! plain field replacement afterwards. The windows overlap by about three days around the
//! current slot, so neither copy is ever unable to sign for "now". Both objects are the same
//! key: no-reuse is carried by the duty loop's once-per-slot dedup, not by which copy signed.
//!
//! The copy is not a memcpy — leanSig's secret key has no `Clone`, so it goes through the
//! canonical encoding, about 33.5 MB out and back. Far cheaper than the rebuild beside it,
//! but not free, which is why it happens on the same schedule as the rebuild and not more
//! often.
//!
//! Transcribed from `docs/design/key-management.md`, Decisions 2 and 3.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use verity_crypto::{Role, RoleKeys, SecretKey, ValidatorKeys, keystore, persist_secret_key};
use verity_types::{Slot, ValidatorIndex};

use crate::error::DutyError;

/// Every key this node holds, with the directory it loaded them from.
#[derive(Debug)]
pub struct Keyring {
    directory: PathBuf,
    validators: Vec<ValidatorKeys>,
}

impl Keyring {
    /// Loads the keys for `indices` from a lean-quickstart key directory.
    ///
    /// # Errors
    ///
    /// [`DutyError::KeyLoad`] for every rejection the loader makes — a missing role, a key
    /// that disagrees with the manifest, or one key covering both roles. All of them mean the
    /// node must not start.
    pub fn load(directory: &Path, indices: &[ValidatorIndex]) -> Result<Self, DutyError> {
        Ok(Self {
            directory: directory.to_path_buf(),
            validators: keystore::load(directory, indices)?,
        })
    }

    /// A keyring holding nothing, for a node that runs no validators.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            directory: PathBuf::new(),
            validators: Vec::new(),
        }
    }

    /// Whether this node performs any duty at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }

    /// The validators this node signs for.
    pub fn validators(&self) -> impl Iterator<Item = &ValidatorKeys> {
        self.validators.iter()
    }

    /// One validator's keys, when this node holds them.
    #[must_use]
    pub fn keys_for(&self, index: ValidatorIndex) -> Option<&ValidatorKeys> {
        self.validators
            .iter()
            .find(|validator| validator.index == index)
    }

    /// Advances every key until it can sign for `slot`.
    ///
    /// Blocking and potentially slow — minutes to hours after long downtime — so the caller
    /// runs it off the async threads. Each advanced key is written back before the next one
    /// starts, so an interrupted catch-up keeps whatever progress it made.
    ///
    /// # Errors
    ///
    /// [`DutyError::Preparation`] when a key's window cannot reach the slot at all, which
    /// means the slot is outside the key's activation range: no amount of preparation
    /// recovers it, and the node must not start.
    pub fn prepare_for(&mut self, slot: Slot) -> Result<(), DutyError> {
        for validator in &mut self.validators {
            // Read out before the key is borrowed: the index is what the log line and the
            // file name need, and taking it first keeps the borrow to the key alone.
            let index = validator.index;
            for role in Role::ALL {
                let keys = role_mut(validator, role);
                if keys.secret.check_signable(slot).is_ok() {
                    continue;
                }

                tracing::info!(
                    validator = index.0,
                    ?role,
                    slot = slot.0,
                    "advancing key preparation to the current slot"
                );
                keys.secret
                    .advance_preparation_to(slot)
                    .map_err(DutyError::Preparation)?;
                write_back(&self.directory, index, role, &keys.secret);
            }
        }
        Ok(())
    }

    /// The keys whose prepared window `slot` has passed the midpoint of.
    ///
    /// The midpoint, rather than the end, is what leaves the rebuild about three days of
    /// margin: advancing there produces a window that still covers the current slot, so the
    /// old key stays able to sign for however long the new one takes to build.
    #[must_use]
    pub fn advances_due(&self, slot: Slot) -> Vec<(ValidatorIndex, Role)> {
        self.validators
            .iter()
            .flat_map(|validator| {
                Role::ALL.into_iter().filter_map(move |role| {
                    let prepared = validator.role(role).secret.prepared_interval();
                    let midpoint = prepared.start + (prepared.end - prepared.start) / 2;
                    (slot.0 >= midpoint).then_some((validator.index, role))
                })
            })
            .collect()
    }

    /// An independent copy of one key, for the worker that advances it off-thread.
    ///
    /// # Errors
    ///
    /// [`DutyError::KeyDuplication`] when the key does not survive its own encoding, which
    /// would be a library bug rather than an input problem.
    pub fn duplicate(
        &self,
        index: ValidatorIndex,
        role: Role,
    ) -> Result<Option<SecretKey>, DutyError> {
        let Some(validator) = self.keys_for(index) else {
            return Ok(None);
        };
        validator
            .role(role)
            .secret
            .duplicate()
            .map(Some)
            .map_err(|()| DutyError::KeyDuplication)
    }

    /// Puts an advanced copy in place of the key it was made from.
    ///
    /// The keyring is owned by one task, so this is a field replacement with no torn state.
    pub fn swap(&mut self, index: ValidatorIndex, role: Role, advanced: SecretKey) {
        let Some(validator) = self
            .validators
            .iter_mut()
            .find(|validator| validator.index == index)
        else {
            return;
        };
        role_mut(validator, role).secret = advanced;
    }

    /// The directory the keys came from, which is where an advanced one is written back.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

/// The advance job the blocking worker runs: rebuild the window, then make it durable.
///
/// Persisting is the last step and happens *before* the key comes back, so a key the duty
/// loop swaps in is already on disk. A failed write is logged and nothing else: it costs
/// extra catch-up after a restart and carries no part of the no-reuse guarantee.
#[must_use = "the advanced key has to be swapped in, or the rebuild was for nothing"]
pub fn advance(
    directory: &Path,
    index: ValidatorIndex,
    role: Role,
    mut secret: SecretKey,
) -> SecretKey {
    secret.advance_preparation();
    write_back(directory, index, role, &secret);
    secret
}

fn write_back(directory: &Path, index: ValidatorIndex, role: Role, secret: &SecretKey) {
    if let Err(error) = persist_secret_key(directory, index, role, secret) {
        tracing::warn!(
            validator = index.0,
            ?role,
            %error,
            "advanced key not written back; a restart will owe the preparation again"
        );
    }
}

fn role_mut(validator: &mut ValidatorKeys, role: Role) -> &mut RoleKeys {
    match role {
        Role::Attestation => &mut validator.attestation,
        Role::Proposal => &mut validator.proposal,
    }
}

/// Advances in flight, so a key is never rebuilt twice at once.
#[derive(Debug, Default)]
pub(crate) struct AdvancesInFlight {
    jobs: HashMap<(ValidatorIndex, Role), tokio::task::JoinHandle<SecretKey>>,
}

impl AdvancesInFlight {
    /// Whether this key already has a rebuild running.
    pub(crate) fn holds(&self, index: ValidatorIndex, role: Role) -> bool {
        self.jobs.contains_key(&(index, role))
    }

    pub(crate) fn insert(
        &mut self,
        index: ValidatorIndex,
        role: Role,
        job: tokio::task::JoinHandle<SecretKey>,
    ) {
        self.jobs.insert((index, role), job);
    }

    /// Collects the rebuilds that have finished since the last call.
    ///
    /// Only finished jobs are awaited, so this never blocks the duty loop on one still
    /// running. A job the runtime cancelled is dropped: the original key is still signing.
    pub(crate) async fn reap(&mut self) -> Vec<(ValidatorIndex, Role, SecretKey)> {
        let finished: Vec<(ValidatorIndex, Role)> = self
            .jobs
            .iter()
            .filter(|(_, job)| job.is_finished())
            .map(|(key, _)| *key)
            .collect();

        let mut advanced = Vec::with_capacity(finished.len());
        for (index, role) in finished {
            let job = self.jobs.remove(&(index, role)).expect("just listed");
            match job.await {
                Ok(secret) => advanced.push((index, role, secret)),
                Err(error) => tracing::warn!(
                    validator = index.0,
                    ?role,
                    %error,
                    "key preparation worker did not finish; the original key keeps signing"
                ),
            }
        }
        advanced
    }
}
