//! The fetch pipeline: what to ask for, whom to believe, and what a bad answer costs.
//!
//! `docs/design/sync.md`, Decision 2. Two shapes of request, chosen by the size of the gap:
//! a targeted `BlocksByRoot` for the few slots a fork or a dropped gossip message leaves
//! behind, and a forward `BlocksByRange` walk for anything a by-root chase cannot close
//! faster than the chain grows.
//!
//! Everything here validates *structure* only — that a response is the shape the request
//! asked for. Signatures and proofs are the verification stage's job, and fetched blocks
//! reach it through the same channel gossip does, so there is no side door into the chain
//! task.

use libssz::SszDecode;
use verity_chain::hash_tree_root;
use verity_p2p::{
    BlocksByRangeRequest, BlocksByRootRequest, ErrorCode, MAX_REQUEST_BLOCKS, NetworkHandle,
    PeerId, Request, RequestedBlockRoots, Response,
};
use verity_types::{Bytes32, SignedBlock, Slot};

use super::peers::Outcome;

/// The largest gap still closed by chasing parents one root at a time.
///
/// Beyond a handful of slots a by-root walk cannot close a gap faster than the chain grows,
/// so the split exists to keep the cheap case cheap rather than to save round trips in the
/// deep case (zeam's threshold).
///
/// This threshold is also what bounds the walk. Each fetched block re-enters the verification
/// stage and raises the next gap if its own parent is missing, so a chase is a sequence of
/// one-link steps rather than a loop with a counter — and every step re-asks this question
/// against the current head, switching to a range walk as soon as the answer changes.
pub const BY_ROOT_GAP_SLOTS: u64 = 4;

/// A block the verification stage is waiting on, and could not resolve.
///
/// The slot is the *waiting child's*, not the awaited block's: the awaited block is known
/// only by root, and the child's slot is what upper-bounds the head-side edge of the gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    /// The block root the parked item is waiting for.
    pub awaited_root: Bytes32,
    /// The slot of the item that is waiting.
    pub waiting_slot: Slot,
}

/// What the sync service decided to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Chase specific parents.
    ByRoot(Vec<Bytes32>),
    /// Walk forward from the last connected slot.
    ByRange(BlocksByRangeRequest),
}

/// Chooses the request that closes a gap between `head_slot` and `waiting_slot`.
///
/// A small gap goes by root, because the missing block is known by root and one round trip
/// ends it. Anything larger goes by range, forward from the first slot this node does not
/// have, because a chain of by-root round trips is one block per round trip and a day-long
/// gap is ~21,600 of them.
#[must_use]
pub fn plan_for_gap(head_slot: Slot, gap: Gap) -> Plan {
    if gap.waiting_slot.0.saturating_sub(head_slot.0) <= BY_ROOT_GAP_SLOTS {
        Plan::ByRoot(vec![gap.awaited_root])
    } else {
        Plan::ByRange(range_from(head_slot, gap.waiting_slot))
    }
}

/// The next forward window, from the first slot this node is missing up to `target`.
///
/// One batch, never more than [`MAX_REQUEST_BLOCKS`] wide: the pipeline keeps one request in
/// flight and re-checks its own head against the network before issuing the next.
#[must_use]
pub fn range_from(head_slot: Slot, target: Slot) -> BlocksByRangeRequest {
    let start = head_slot.0.saturating_add(1);
    let wanted = target.0.saturating_sub(head_slot.0);
    BlocksByRangeRequest {
        start_slot: Slot(start),
        count: wanted.clamp(1, MAX_REQUEST_BLOCKS as u64),
    }
}

/// Why a response cannot be used, and what it costs the peer that sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFailure {
    /// The peer answered in the protocol's own terms with a refusal or an error.
    Refused(ErrorCode),
    /// The request never produced a response.
    Transport(String),
    /// The response was a success carrying content the request did not ask for.
    Malformed(Violation),
}

impl FetchFailure {
    /// What this failure does to the peer's score (`docs/design/sync.md`, Decision 3).
    ///
    /// `RESOURCE_UNAVAILABLE` is the one neutral outcome: it is the spec's *legal* answer for
    /// history below the serving window, and the request is simply re-routed. Malformed
    /// content inside a success is a plain failure — the peer does serve the protocol, badly
    /// — and never sets the capability flag.
    #[must_use]
    pub const fn outcome(&self) -> Outcome {
        match self {
            Self::Refused(ErrorCode::ResourceUnavailable) => Outcome::Neutral,
            Self::Refused(_) | Self::Malformed(_) => Outcome::Failure,
            Self::Transport(_) => Outcome::Failure,
        }
    }
}

impl core::fmt::Display for FetchFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused(code) => write!(f, "peer refused: {code}"),
            Self::Transport(error) => write!(f, "{error}"),
            Self::Malformed(violation) => write!(f, "malformed response: {violation}"),
        }
    }
}

/// The ways a successful response can still fail to answer the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Violation {
    /// A chunk is not a `SignedBlock`.
    Undecodable,
    /// More chunks than the request could have asked for.
    TooManyChunks,
    /// A block's slot falls outside the requested window.
    OutOfWindow,
    /// Slots do not ascend strictly, so the response is not a chain segment.
    NotMonotonic,
    /// A block was returned whose root was never asked for.
    Unrequested,
}

impl core::fmt::Display for Violation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Undecodable => "a chunk is not a signed block",
            Self::TooManyChunks => "more chunks than were requested",
            Self::OutOfWindow => "a block falls outside the requested window",
            Self::NotMonotonic => "slots do not ascend",
            Self::Unrequested => "a block nobody asked for",
        })
    }
}

/// Checks a `BlocksByRange` response against the request that produced it.
///
/// A short response is legal — empty slots are absent rather than zero-filled, and a peer may
/// hold less of the window than it was asked for — so only the three structural properties
/// are enforced: within the window, strictly ascending, and no more chunks than the window
/// has slots.
///
/// # Errors
///
/// [`Violation`] naming the property that failed.
pub fn validate_range(
    request: &BlocksByRangeRequest,
    chunks: &[Vec<u8>],
) -> Result<Vec<Slot>, Violation> {
    if chunks.len() as u64 > request.count {
        return Err(Violation::TooManyChunks);
    }

    let end = request.start_slot.0.saturating_add(request.count);
    let mut slots = Vec::with_capacity(chunks.len());
    let mut previous: Option<u64> = None;

    for chunk in chunks {
        let block = SignedBlock::from_ssz_bytes(chunk).map_err(|_| Violation::Undecodable)?;
        let slot = block.block.slot;
        if slot.0 < request.start_slot.0 || slot.0 >= end {
            return Err(Violation::OutOfWindow);
        }
        if previous.is_some_and(|last| slot.0 <= last) {
            return Err(Violation::NotMonotonic);
        }
        previous = Some(slot.0);
        slots.push(slot);
    }
    Ok(slots)
}

/// Checks a `BlocksByRoot` response against the roots that were asked for.
///
/// Missing roots are skipped silently by the protocol, so a partial response is legal and an
/// empty one is a legitimate "none of those". What is not legal is a block nobody asked for,
/// or the same root twice.
///
/// # Errors
///
/// [`Violation`] naming the property that failed.
pub fn validate_by_root(
    requested: &[Bytes32],
    chunks: &[Vec<u8>],
) -> Result<Vec<Bytes32>, Violation> {
    if chunks.len() > requested.len() {
        return Err(Violation::TooManyChunks);
    }

    let mut seen = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let block = SignedBlock::from_ssz_bytes(chunk).map_err(|_| Violation::Undecodable)?;
        let root = hash_tree_root(&block.block);
        if !requested.contains(&root) || seen.contains(&root) {
            return Err(Violation::Unrequested);
        }
        seen.push(root);
    }
    Ok(seen)
}

/// Asks one peer for a slot range, and returns the chunks only if they answer the request.
///
/// # Errors
///
/// [`FetchFailure`] carrying the outcome the peer's score is moved by.
pub async fn fetch_range(
    handle: &NetworkHandle,
    peer: PeerId,
    request: BlocksByRangeRequest,
) -> Result<Vec<Vec<u8>>, FetchFailure> {
    let chunks = blocks(handle, peer, Request::BlocksByRange(request)).await?;
    validate_range(&request, &chunks).map_err(FetchFailure::Malformed)?;
    Ok(chunks)
}

/// Asks one peer for specific blocks by root.
///
/// # Errors
///
/// [`FetchFailure`] carrying the outcome the peer's score is moved by, including
/// [`Violation::TooManyChunks`] when more roots are asked for than the protocol allows.
pub async fn fetch_by_root(
    handle: &NetworkHandle,
    peer: PeerId,
    roots: Vec<Bytes32>,
) -> Result<Vec<Vec<u8>>, FetchFailure> {
    let list = RequestedBlockRoots::try_from(roots.clone())
        .map_err(|_| FetchFailure::Malformed(Violation::TooManyChunks))?;
    let chunks = blocks(
        handle,
        peer,
        Request::BlocksByRoot(BlocksByRootRequest { roots: list }),
    )
    .await?;
    validate_by_root(&roots, &chunks).map_err(FetchFailure::Malformed)?;
    Ok(chunks)
}

/// The half of a block request that is the same for both protocols.
async fn blocks(
    handle: &NetworkHandle,
    peer: PeerId,
    request: Request,
) -> Result<Vec<Vec<u8>>, FetchFailure> {
    match handle.request(peer, request).await {
        Ok(Response::Blocks(chunks)) => Ok(chunks),
        Ok(Response::Error { code, message }) => {
            tracing::debug!(%peer, %code, %message, "peer refused a block request");
            Err(FetchFailure::Refused(code))
        }
        // A status payload on a block protocol is the peer answering a question nobody asked.
        Ok(Response::Status(_)) => Err(FetchFailure::Malformed(Violation::Undecodable)),
        Err(error) => Err(FetchFailure::Transport(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use libssz::SszEncode;
    use verity_chain::hash_tree_root;
    use verity_p2p::{BlocksByRangeRequest, ErrorCode, MAX_REQUEST_BLOCKS};
    use verity_types::{Block, MultiMessageAggregate, SignedBlock, Slot};

    use super::{
        BY_ROOT_GAP_SLOTS, FetchFailure, Gap, Plan, Violation, plan_for_gap, range_from,
        validate_by_root, validate_range,
    };
    use crate::sync::peers::Outcome;

    fn block_at(slot: u64) -> SignedBlock {
        SignedBlock {
            block: Block {
                slot: Slot(slot),
                ..Block::default()
            },
            proof: MultiMessageAggregate::default(),
        }
    }

    fn chunk(slot: u64) -> Vec<u8> {
        block_at(slot).to_ssz()
    }

    #[test]
    fn should_chase_a_small_gap_by_root() {
        let gap = Gap {
            awaited_root: [7u8; 32],
            waiting_slot: Slot(10 + BY_ROOT_GAP_SLOTS),
        };
        assert_eq!(plan_for_gap(Slot(10), gap), Plan::ByRoot(vec![[7u8; 32]]));
    }

    #[test]
    fn should_walk_a_large_gap_by_range_from_the_first_missing_slot() {
        let gap = Gap {
            awaited_root: [7u8; 32],
            waiting_slot: Slot(5_000),
        };
        let Plan::ByRange(request) = plan_for_gap(Slot(10), gap) else {
            panic!("a deep gap is a range walk");
        };
        assert_eq!(request.start_slot, Slot(11));
        assert_eq!(request.count, MAX_REQUEST_BLOCKS as u64);
    }

    #[test]
    fn should_never_ask_for_a_window_of_zero_slots() {
        let request = range_from(Slot(40), Slot(40));
        assert_eq!(request.start_slot, Slot(41));
        assert_eq!(request.count, 1);
    }

    #[test]
    fn should_accept_a_short_response_inside_the_window() {
        let request = BlocksByRangeRequest {
            start_slot: Slot(10),
            count: 8,
        };
        let chunks = vec![chunk(10), chunk(12), chunk(17)];
        assert_eq!(
            validate_range(&request, &chunks).expect("a legal partial response"),
            vec![Slot(10), Slot(12), Slot(17)]
        );
    }

    #[test]
    fn should_reject_a_block_outside_the_requested_window() {
        let request = BlocksByRangeRequest {
            start_slot: Slot(10),
            count: 4,
        };
        assert_eq!(
            validate_range(&request, &[chunk(14)]),
            Err(Violation::OutOfWindow)
        );
        assert_eq!(
            validate_range(&request, &[chunk(9)]),
            Err(Violation::OutOfWindow)
        );
    }

    #[test]
    fn should_reject_slots_that_do_not_ascend() {
        let request = BlocksByRangeRequest {
            start_slot: Slot(10),
            count: 8,
        };
        assert_eq!(
            validate_range(&request, &[chunk(12), chunk(11)]),
            Err(Violation::NotMonotonic)
        );
        assert_eq!(
            validate_range(&request, &[chunk(12), chunk(12)]),
            Err(Violation::NotMonotonic)
        );
    }

    #[test]
    fn should_reject_more_chunks_than_the_window_holds() {
        let request = BlocksByRangeRequest {
            start_slot: Slot(10),
            count: 1,
        };
        assert_eq!(
            validate_range(&request, &[chunk(10), chunk(11)]),
            Err(Violation::TooManyChunks)
        );
    }

    #[test]
    fn should_reject_a_chunk_that_is_not_a_block() {
        let request = BlocksByRangeRequest {
            start_slot: Slot(10),
            count: 4,
        };
        assert_eq!(
            validate_range(&request, &[vec![0xff; 3]]),
            Err(Violation::Undecodable)
        );
    }

    #[test]
    fn should_accept_a_by_root_response_missing_some_roots() {
        let present = block_at(31);
        let requested = vec![hash_tree_root(&present.block), [9u8; 32]];
        assert_eq!(
            validate_by_root(&requested, &[present.to_ssz()]).expect("a legal partial response"),
            vec![requested[0]]
        );
        assert_eq!(validate_by_root(&requested, &[]), Ok(Vec::new()));
    }

    #[test]
    fn should_reject_a_by_root_response_carrying_a_block_nobody_asked_for() {
        let requested = vec![[9u8; 32]];
        assert_eq!(
            validate_by_root(&requested, &[chunk(31)]),
            Err(Violation::Unrequested)
        );
    }

    #[test]
    fn should_reject_the_same_root_twice() {
        let present = block_at(31);
        let requested = vec![hash_tree_root(&present.block), [9u8; 32]];
        assert_eq!(
            validate_by_root(&requested, &[present.to_ssz(), present.to_ssz()]),
            Err(Violation::Unrequested)
        );
    }

    #[test]
    fn should_treat_a_legal_refusal_as_neutral_and_everything_else_as_a_failure() {
        assert_eq!(
            FetchFailure::Refused(ErrorCode::ResourceUnavailable).outcome(),
            Outcome::Neutral
        );
        assert_eq!(
            FetchFailure::Refused(ErrorCode::ServerError).outcome(),
            Outcome::Failure
        );
        assert_eq!(
            FetchFailure::Malformed(Violation::NotMonotonic).outcome(),
            Outcome::Failure
        );
        assert_eq!(
            FetchFailure::Transport("timed out".to_string()).outcome(),
            Outcome::Failure
        );
    }
}
