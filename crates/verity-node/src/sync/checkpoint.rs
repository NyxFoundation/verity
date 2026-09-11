//! Checkpoint entry: adopting a recent finalized state instead of replaying from genesis.
//!
//! `docs/design/sync.md`, Decision 1. Replaying from genesis means verifying every aggregate
//! proof the chain ever carried — 155–236 KB each, one per block — so a node joining a
//! running network fetches the finalized state and its block over HTTP and starts there.
//!
//! # The trust this buys, and what pays for it
//!
//! A genesis start trusts nobody. A checkpoint start trusts whoever answers the URL, so the
//! anchor is verified at the strictest depth surveyed before it is written: it must belong to
//! the chain the local genesis file describes, be internally consistent, and be the block the
//! state itself names as final. Two failure classes, and they are handled differently:
//! transient ones (timeouts, refused connections, 5xx) are retried within a bounded budget,
//! and definitive ones (any 4xx, an SSZ decode failure, any verification check) end it at
//! once.
//!
//! When the budget runs out or a definitive failure occurs, the node **fails closed and
//! exits**. It never falls back to genesis or to a stale database: starting from a different
//! anchor than the operator asked for is an operator's decision, never an automatic one.
//!
//! Endpoints and the state-structure check transcribe leanSpec
//! `src/lean_spec/node/sync/checkpoint_sync.py`, read at commit `8603fa63`.

use std::sync::OnceLock;
use std::time::Duration;

use libssz::SszDecode;
use verity_chain::hash_tree_root;
use verity_types::config::VALIDATOR_REGISTRY_LIMIT;
use verity_types::{SignedBlock, State};

/// Beacon-API-shaped path for the finalized state.
pub const FINALIZED_STATE_ENDPOINT: &str = "/lean/v0/states/finalized";
/// Path for the signed block that state names as final.
pub const FINALIZED_BLOCK_ENDPOINT: &str = "/lean/v0/blocks/finalized";

/// Seconds allowed per request. A finalized state runs to tens of megabytes, so the transfer
/// needs a wide window (leanSpec's own timeout).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// How many times a transient failure is retried before the node gives up and exits.
pub const MAX_ATTEMPTS: u32 = 5;

/// The first backoff interval. Each further attempt doubles it.
pub const BACKOFF_BASE: Duration = Duration::from_secs(1);

/// The anchor a checkpoint start begins from.
#[derive(Debug, Clone)]
pub struct CheckpointAnchor {
    /// The finalized state.
    pub state: State,
    /// The block that state names as final, with its proof.
    pub block: SignedBlock,
}

/// Why a checkpoint start could not happen. Every variant is fatal to startup.
#[derive(Debug)]
pub enum CheckpointError {
    /// The endpoint could not be reached within the retry budget.
    Unreachable {
        /// The URL that was tried.
        url: String,
        /// How many attempts were made.
        attempts: u32,
        /// The last failure seen.
        reason: String,
    },
    /// The server answered, and the answer was wrong. Never retried.
    Rejected {
        /// The URL that answered.
        url: String,
        /// What was wrong with it.
        reason: String,
    },
    /// The anchor did not pass verification against the local genesis file.
    Unverified(AnchorFailure),
}

impl core::fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unreachable {
                url,
                attempts,
                reason,
            } => write!(
                f,
                "checkpoint sync gave up on {url} after {attempts} attempts: {reason}"
            ),
            Self::Rejected { url, reason } => write!(f, "checkpoint sync refused {url}: {reason}"),
            Self::Unverified(failure) => {
                write!(f, "the checkpoint anchor is not usable: {failure}")
            }
        }
    }
}

impl std::error::Error for CheckpointError {}

/// The ways a fetched anchor fails verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorFailure {
    /// The state carries no validators, or more than the registry can hold.
    UnusableRegistry,
    /// The state belongs to a chain with a different genesis time.
    ForeignGenesisTime,
    /// The state's registry is not the one the local genesis file describes.
    ForeignRegistry,
    /// `finalized ≤ justified ≤ state.slot` does not hold.
    SlotsOutOfOrder,
    /// The state's own latest block is above the state itself.
    HeaderAheadOfState,
    /// The fetched block is not the block the state's own header describes.
    BlockIsNotTheAnchor,
    /// The block does not commit to the state that came with it.
    StateIsNotTheBlocks,
    /// The state's latest block sits at the finalized slot but is not the finalized block.
    FinalizedRootMismatch,
    /// Justified and finalized share a slot but not a root.
    CheckpointsDisagree,
}

impl core::fmt::Display for AnchorFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::UnusableRegistry => "the validator registry is empty or over the limit",
            Self::ForeignGenesisTime => "the genesis time is not this chain's",
            Self::ForeignRegistry => "the validator registry is not this chain's",
            Self::SlotsOutOfOrder => "finalized, justified and state slots are out of order",
            Self::HeaderAheadOfState => "the state's latest block is ahead of the state",
            Self::BlockIsNotTheAnchor => "the block is not the state's own latest block",
            Self::StateIsNotTheBlocks => "the block does not commit to this state",
            Self::FinalizedRootMismatch => "the state's latest block is not the block it finalizes",
            Self::CheckpointsDisagree => "justified and finalized share a slot but not a root",
        })
    }
}

/// Fetches the finalized state and block from `url` and verifies them against local genesis.
///
/// # Errors
///
/// [`CheckpointError`] in every case. There is no partial success: a node that asked for a
/// checkpoint start and cannot have one stops.
pub async fn fetch_anchor(url: &str, genesis: &State) -> Result<CheckpointAnchor, CheckpointError> {
    let base = url.trim_end_matches('/');
    install_crypto_provider();
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| CheckpointError::Rejected {
            url: base.to_string(),
            reason: error.to_string(),
        })?;

    let state_bytes = fetch(&client, &format!("{base}{FINALIZED_STATE_ENDPOINT}")).await?;
    let state = decode::<State>(&state_bytes, base, "state")?;
    tracing::info!(
        slot = state.slot.0,
        bytes = state_bytes.len(),
        "fetched a finalized state"
    );

    let block_bytes = fetch(&client, &format!("{base}{FINALIZED_BLOCK_ENDPOINT}")).await?;
    let block = decode::<SignedBlock>(&block_bytes, base, "block")?;
    tracing::info!(slot = block.block.slot.0, "fetched the finalized block");

    verify_anchor(&state, &block, genesis).map_err(CheckpointError::Unverified)?;
    Ok(CheckpointAnchor { state, block })
}

/// Installs `ring` as the process's rustls provider, once.
///
/// reqwest is built with `rustls-no-provider`, so nothing selects a provider on its behalf;
/// without this the first HTTPS request fails at runtime rather than at build time. `ring` is
/// the provider libp2p-quic already uses, so this keeps one implementation in the process
/// instead of adding `aws-lc-rs` and its C toolchain (see the workspace manifest).
///
/// A provider installed by someone else wins and this is a no-op: the install is attempted
/// once and its result deliberately ignored, because "somebody already chose" is success.
fn install_crypto_provider() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// One endpoint, retried while the failures are transient.
async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, CheckpointError> {
    let mut backoff = BACKOFF_BASE;
    let mut last = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        match attempt_fetch(client, url).await {
            Ok(bytes) => return Ok(bytes),
            Err(Attempt::Definitive(reason)) => {
                return Err(CheckpointError::Rejected {
                    url: url.to_string(),
                    reason,
                });
            }
            Err(Attempt::Transient(reason)) => {
                tracing::warn!(%url, attempt, %reason, "checkpoint fetch failed; retrying");
                last = reason;
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                }
            }
        }
    }

    Err(CheckpointError::Unreachable {
        url: url.to_string(),
        attempts: MAX_ATTEMPTS,
        reason: last,
    })
}

/// One failure of one attempt, classified by whether trying again could help.
enum Attempt {
    /// A timeout, a refused connection, or a 5xx: the server may yet answer.
    Transient(String),
    /// A 4xx: the server answered, and the answer was that this request is wrong.
    Definitive(String),
}

async fn attempt_fetch(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, Attempt> {
    // The endpoints return raw SSZ, not JSON.
    let response = client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .map_err(|error| Attempt::Transient(error.to_string()))?;

    let status = response.status();
    if status.is_client_error() {
        return Err(Attempt::Definitive(format!("HTTP {status}")));
    }
    if !status.is_success() {
        return Err(Attempt::Transient(format!("HTTP {status}")));
    }

    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| Attempt::Transient(error.to_string()))
}

/// Decodes a payload, or refuses the server for good.
fn decode<T: SszDecode>(bytes: &[u8], url: &str, what: &str) -> Result<T, CheckpointError> {
    T::from_ssz_bytes(bytes).map_err(|_| CheckpointError::Rejected {
        url: url.to_string(),
        reason: format!(
            "the {what} payload is not SSZ this build can read ({} bytes)",
            bytes.len()
        ),
    })
}

/// leanSpec's own structural check on a downloaded state.
///
/// Two invariants, and both are about the registry: a state with no validators cannot drive
/// fork choice, and one claiming more than the registry holds is an attacker-supplied blob.
/// Kept as its own function because it is what leanSpec's `sync_test` vectors pin.
///
/// Transcribed from leanSpec `node/sync/checkpoint_sync.py::verify_checkpoint_state`.
#[must_use]
pub fn verify_checkpoint_state(state: &State) -> bool {
    let count = state.validators.len();
    count > 0 && count <= VALIDATOR_REGISTRY_LIMIT
}

/// The full anchor check: leanSpec's, plus everything that ties the anchor to *this* chain.
///
/// # What pairs the block with the state, and what does not
///
/// The block is paired with `state.latest_block_header`, not with `state.latest_finalized`.
/// Those are different things, and only the first pairing is unconditional: a state's
/// finalized checkpoint names an *ancestor* in the general case, and demanding that the
/// anchor block be that ancestor is circular — the block commits to this very state, so this
/// state cannot also be one the block descends from. The finalized root is therefore checked
/// exactly where it is meaningful: when the state's own latest block sits at the finalized
/// slot, it must be the finalized block.
///
/// The pairing itself is two facts, and each has its own failure: the block *is* the block
/// the state's header describes (same slot, proposer, parent, and body), and the block
/// commits to this state (`block.state_root == hash_tree_root(state)`, which is the equality
/// leanSpec's own state transition asserts when it accepts a block).
///
/// Mirrors ethlambda `bin/ethlambda/src/checkpoint_sync.rs`, the strictest of the surveyed
/// implementations (`docs/design/sync.md`, Decision 1).
///
/// # Errors
///
/// [`AnchorFailure`] naming the first check that failed.
pub fn verify_anchor(
    state: &State,
    block: &SignedBlock,
    genesis: &State,
) -> Result<(), AnchorFailure> {
    if !verify_checkpoint_state(state) {
        return Err(AnchorFailure::UnusableRegistry);
    }
    if state.config.genesis_time != genesis.config.genesis_time {
        return Err(AnchorFailure::ForeignGenesisTime);
    }
    if state.validators != genesis.validators {
        return Err(AnchorFailure::ForeignRegistry);
    }
    if state.latest_finalized.slot.0 > state.latest_justified.slot.0
        || state.latest_justified.slot.0 > state.slot.0
    {
        return Err(AnchorFailure::SlotsOutOfOrder);
    }
    if state.latest_block_header.slot.0 > state.slot.0 {
        return Err(AnchorFailure::HeaderAheadOfState);
    }
    if state.latest_justified.slot == state.latest_finalized.slot
        && state.latest_justified.root != state.latest_finalized.root
    {
        return Err(AnchorFailure::CheckpointsDisagree);
    }

    let header = &state.latest_block_header;
    let anchor = &block.block;
    if anchor.slot != header.slot
        || anchor.proposer_index != header.proposer_index
        || anchor.parent_root != header.parent_root
        || hash_tree_root(&anchor.body) != header.body_root
    {
        return Err(AnchorFailure::BlockIsNotTheAnchor);
    }
    if anchor.state_root != hash_tree_root(state) {
        return Err(AnchorFailure::StateIsNotTheBlocks);
    }

    let block_root = hash_tree_root(anchor);
    if state.latest_block_header.slot == state.latest_finalized.slot
        && block_root != state.latest_finalized.root
    {
        return Err(AnchorFailure::FinalizedRootMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use verity_chain::{generate_genesis, hash_tree_root};
    use verity_db::stored_header;
    use verity_types::{
        BlockBody, Checkpoint, MultiMessageAggregate, SignedBlock, Slot, State, Validator,
        ValidatorIndex, Validators,
    };

    use crate::store_open::block_from;

    use super::{AnchorFailure, verify_anchor, verify_checkpoint_state};

    fn genesis(count: u64, genesis_time: u64) -> State {
        let mut validators = Validators::default();
        for index in 0..count {
            let seed = index as u8;
            validators
                .push(Validator {
                    attestation_public_key: [seed; 52],
                    proposal_public_key: [seed.wrapping_add(128); 52],
                    index: ValidatorIndex(index),
                })
                .expect("under the registry limit");
        }
        generate_genesis(genesis_time, validators)
    }

    /// The anchor pair a server would serve: a state, and the block that produced it.
    ///
    /// Built the way the chain builds it — the block is the state's own latest header with
    /// its `state_root` filled in — so nothing here is circular.
    fn anchor(base: &State, slot: u64, finalized_slot: u64) -> (State, SignedBlock) {
        let mut state = base.clone();
        state.slot = Slot(slot);
        state.latest_block_header.slot = Slot(slot);
        state.latest_block_header.parent_root = [3u8; 32];
        // A post-state carries its own header with the state root not yet filled in.
        state.latest_block_header.state_root = [0u8; 32];
        state.latest_finalized = Checkpoint {
            root: [4u8; 32],
            slot: Slot(finalized_slot),
        };
        state.latest_justified = state.latest_finalized;

        let block = SignedBlock {
            block: block_from(&stored_header(&state), BlockBody::default()),
            proof: MultiMessageAggregate::default(),
        };
        (state, block)
    }

    #[test]
    fn should_accept_an_anchor_that_belongs_to_this_chain() {
        let base = genesis(4, 1_700_000_000);
        let (state, block) = anchor(&base, 128, 96);
        assert_eq!(verify_anchor(&state, &block, &base), Ok(()));
    }

    #[test]
    fn should_refuse_a_state_from_a_chain_with_another_genesis_time() {
        let base = genesis(4, 1_700_000_000);
        let (mut state, block) = anchor(&base, 128, 96);
        state.config.genesis_time += 1;
        assert_eq!(
            verify_anchor(&state, &block, &base),
            Err(AnchorFailure::ForeignGenesisTime)
        );
    }

    #[test]
    fn should_refuse_a_state_with_another_validator_registry() {
        let base = genesis(4, 1_700_000_000);
        let other = genesis(5, 1_700_000_000);
        let (state, block) = anchor(&other, 128, 96);
        assert_eq!(
            verify_anchor(&state, &block, &base),
            Err(AnchorFailure::ForeignRegistry)
        );
    }

    #[test]
    fn should_refuse_slots_that_are_out_of_order() {
        let base = genesis(4, 1_700_000_000);
        let (mut state, block) = anchor(&base, 128, 96);
        state.latest_finalized.slot = Slot(200);
        assert_eq!(
            verify_anchor(&state, &block, &base),
            Err(AnchorFailure::SlotsOutOfOrder)
        );
    }

    #[test]
    fn should_refuse_a_state_whose_latest_block_is_ahead_of_it() {
        let base = genesis(4, 1_700_000_000);
        let (mut state, block) = anchor(&base, 128, 96);
        state.latest_block_header.slot = Slot(129);
        assert_eq!(
            verify_anchor(&state, &block, &base),
            Err(AnchorFailure::HeaderAheadOfState)
        );
    }

    #[test]
    fn should_refuse_a_block_that_did_not_produce_this_state() {
        let base = genesis(4, 1_700_000_000);
        let (state, _) = anchor(&base, 128, 96);
        let (_, other_block) = anchor(&base, 127, 96);
        assert_eq!(
            verify_anchor(&state, &other_block, &base),
            Err(AnchorFailure::BlockIsNotTheAnchor)
        );
    }

    #[test]
    fn should_refuse_a_block_that_does_not_commit_to_the_state_it_arrived_with() {
        let base = genesis(4, 1_700_000_000);
        let (state, mut block) = anchor(&base, 128, 96);
        block.block.state_root = [8u8; 32];
        assert_eq!(
            verify_anchor(&state, &block, &base),
            Err(AnchorFailure::StateIsNotTheBlocks)
        );
    }

    #[test]
    fn should_check_the_finalized_root_only_at_the_finalized_slot() {
        let base = genesis(4, 1_700_000_000);
        // A normal anchor finalizes an ancestor, so the check does not apply and the
        // arbitrary finalized root is no objection.
        let (state, block) = anchor(&base, 128, 96);
        assert_eq!(verify_anchor(&state, &block, &base), Ok(()));

        // Move the finalized checkpoint up to the anchor's own slot, keeping a root that is
        // not the anchor's: now the state claims this block is final and it is not.
        let (state, block) = anchor(&base, 128, 128);
        assert_ne!(state.latest_finalized.root, hash_tree_root(&block.block));
        assert_eq!(
            verify_anchor(&state, &block, &base),
            Err(AnchorFailure::FinalizedRootMismatch)
        );
    }

    #[test]
    fn should_refuse_justified_and_finalized_that_share_a_slot_but_not_a_root() {
        let base = genesis(4, 1_700_000_000);
        let (mut state, block) = anchor(&base, 128, 96);
        state.latest_justified.root = [9u8; 32];
        assert_eq!(
            verify_anchor(&state, &block, &base),
            Err(AnchorFailure::CheckpointsDisagree)
        );
    }

    #[test]
    fn should_refuse_an_empty_registry() {
        let empty = State::default();
        assert!(!verify_checkpoint_state(&empty));
        assert!(verify_checkpoint_state(&genesis(1, 0)));
    }
}
