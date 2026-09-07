//! Peer management: what each peer claims, how reliably it answers, and who gets asked next.
//!
//! `docs/design/sync.md`, Decision 3. The spec is silent on scoring, disconnects and limits,
//! so this is deliberately minimal policy over the one implemented precedent: a
//! request-reliability score, weighted selection that never fully excludes a peer, and no
//! bans at all. Deprioritization is the only consequence a peer ever suffers here.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rand::RngExt;
use verity_p2p::{PeerId, RequestError, Status};
use verity_types::Slot;

use super::state::median_finalized;

/// Score a peer starts at, and the midpoint of the range.
pub const INITIAL_SCORE: i32 = 100;
/// What a completed request is worth.
pub const SUCCESS_REWARD: i32 = 10;
/// What a failed one costs. Twice the reward, so a failing peer loses weight faster than it
/// earns it.
pub const FAILURE_PENALTY: i32 = 20;
/// The score floor. A peer at the floor is still selectable — exclusion on a devnet-sized
/// peer set is a direct liveness cut.
pub const MIN_SCORE: i32 = 0;
/// The score ceiling.
pub const MAX_SCORE: i32 = 200;

/// How many requests may be outstanding to one peer at a time.
pub const MAX_CONCURRENT_REQUESTS: usize = 2;

/// How long a peer stays flagged as not speaking a block protocol.
///
/// The flag has exactly one trigger — libp2p protocol negotiation failing — and a TTL so a
/// peer that gains the protocol on its next release is not excluded forever.
pub const CAPABILITY_TTL: Duration = Duration::from_secs(600);

/// What one request outcome does to a peer's standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The peer answered, and the answer was usable.
    Success,
    /// The peer timed out, disconnected, errored, or served malformed content inside a
    /// success. All four are the same consequence: it does serve the protocol, badly.
    Failure,
    /// `RESOURCE_UNAVAILABLE`: the spec's legal answer for history below the serving window.
    /// No score change; the request is re-routed to another peer.
    Neutral,
    /// The peer does not speak the protocol at all. The one thing that sets the capability
    /// flag, and it is never set by anything response-shaped.
    Unsupported,
}

impl Outcome {
    /// How a transport-level failure scores.
    #[must_use]
    pub const fn of_request_error(error: &RequestError) -> Self {
        match error {
            RequestError::UnsupportedProtocol => Self::Unsupported,
            _ => Self::Failure,
        }
    }
}

/// One peer, as the sync service sees it.
#[derive(Debug)]
struct Peer {
    /// The peer's last claimed checkpoints, absent until it answers a handshake.
    status: Option<Status>,
    score: i32,
    in_flight: usize,
    /// When the capability flag lapses, absent when the peer is not flagged.
    unsupported_until: Option<Instant>,
}

impl Peer {
    const fn new() -> Self {
        Self {
            status: None,
            score: INITIAL_SCORE,
            in_flight: 0,
            unsupported_until: None,
        }
    }

    fn serves_blocks(&self, now: Instant) -> bool {
        self.unsupported_until.is_none_or(|until| now >= until)
    }

    /// Selection weight. Never zero: the floor peer keeps one share.
    const fn weight(&self) -> u32 {
        (self.score + 1) as u32
    }
}

/// Every connected peer, with what it claims and how well it has answered.
#[derive(Debug, Default)]
pub struct PeerTable {
    peers: HashMap<PeerId, Peer>,
}

impl PeerTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a new connection. A reconnecting peer starts over at [`INITIAL_SCORE`]:
    /// nothing here survives a disconnect, because nothing here is a punishment.
    pub fn connected(&mut self, peer: PeerId) {
        self.peers.insert(peer, Peer::new());
    }

    /// Forgets a peer.
    pub fn disconnected(&mut self, peer: &PeerId) {
        self.peers.remove(peer);
    }

    /// Records what a peer claims about its own chain.
    pub fn observe_status(&mut self, peer: PeerId, status: Status) {
        self.peers.entry(peer).or_insert_with(Peer::new).status = Some(status);
    }

    /// Every connected peer, whether or not it has answered a handshake yet.
    #[must_use]
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.peers.keys().copied().collect()
    }

    /// Whether no peer is connected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// The network's finalized slot as the connected peers claim it.
    #[must_use]
    pub fn network_finalized_slot(&self) -> Option<Slot> {
        median_finalized(
            self.peers
                .values()
                .filter_map(|peer| peer.status.map(|status| status.finalized.slot))
                .collect(),
        )
    }

    /// The highest head any peer claims, which is how far a range walk may usefully run.
    #[must_use]
    pub fn highest_claimed_head(&self) -> Option<Slot> {
        self.peers
            .values()
            .filter_map(|peer| peer.status.map(|status| status.head.slot))
            .max_by_key(|slot| slot.0)
    }

    /// Picks a peer to ask for blocks, by score-weighted random selection.
    ///
    /// Peers at the concurrency limit and peers whose capability flag is still live are out
    /// of the draw; everyone else is in it, including a peer sitting at the score floor.
    #[must_use]
    pub fn select_for_blocks(&self, now: Instant) -> Option<PeerId> {
        let eligible =
            |peer: &Peer| peer.in_flight < MAX_CONCURRENT_REQUESTS && peer.serves_blocks(now);

        // Two passes over the table rather than one pass into a `Vec`: the draw needs the
        // total before it can pick a ticket, and the table is a devnet's worth of peers.
        let total: u32 = self
            .peers
            .values()
            .filter(|peer| eligible(peer))
            .map(Peer::weight)
            .sum();
        if total == 0 {
            return None;
        }

        let mut ticket = rand::rng().random_range(0..total);
        for (id, peer) in self.peers.iter().filter(|(_, peer)| eligible(peer)) {
            let weight = peer.weight();
            if ticket < weight {
                return Some(*id);
            }
            ticket -= weight;
        }
        None
    }

    /// Marks a request as outstanding against a peer.
    pub fn begin_request(&mut self, peer: &PeerId) {
        if let Some(entry) = self.peers.get_mut(peer) {
            entry.in_flight += 1;
        }
    }

    /// Records how a request ended, and moves the peer's score accordingly.
    pub fn finish_request(&mut self, peer: &PeerId, outcome: Outcome, now: Instant) {
        let Some(entry) = self.peers.get_mut(peer) else {
            return;
        };
        entry.in_flight = entry.in_flight.saturating_sub(1);

        let delta = match outcome {
            Outcome::Success => SUCCESS_REWARD,
            Outcome::Failure | Outcome::Unsupported => -FAILURE_PENALTY,
            Outcome::Neutral => 0,
        };
        entry.score = (entry.score + delta).clamp(MIN_SCORE, MAX_SCORE);

        if outcome == Outcome::Unsupported {
            entry.unsupported_until = Some(now + CAPABILITY_TTL);
        }
    }

    /// A peer's score, for tests and for the operator's log line.
    #[must_use]
    pub fn score(&self, peer: &PeerId) -> Option<i32> {
        self.peers.get(peer).map(|entry| entry.score)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use verity_p2p::{PeerId, RequestError, Status};
    use verity_types::{Checkpoint, Slot};

    use super::{
        CAPABILITY_TTL, INITIAL_SCORE, MAX_CONCURRENT_REQUESTS, MAX_SCORE, MIN_SCORE, Outcome,
        PeerTable,
    };

    fn status(finalized: u64, head: u64) -> Status {
        Status {
            finalized: Checkpoint {
                root: [1u8; 32],
                slot: Slot(finalized),
            },
            head: Checkpoint {
                root: [2u8; 32],
                slot: Slot(head),
            },
        }
    }

    #[test]
    fn should_reward_success_less_than_it_punishes_failure() {
        let peer = PeerId::random();
        let mut table = PeerTable::new();
        table.connected(peer);
        let now = Instant::now();

        table.begin_request(&peer);
        table.finish_request(&peer, Outcome::Success, now);
        assert_eq!(table.score(&peer), Some(INITIAL_SCORE + 10));

        table.begin_request(&peer);
        table.finish_request(&peer, Outcome::Failure, now);
        assert_eq!(table.score(&peer), Some(INITIAL_SCORE - 10));
    }

    #[test]
    fn should_leave_the_score_alone_when_the_peer_refuses_history_it_may_refuse() {
        let peer = PeerId::random();
        let mut table = PeerTable::new();
        table.connected(peer);

        table.begin_request(&peer);
        table.finish_request(&peer, Outcome::Neutral, Instant::now());
        assert_eq!(table.score(&peer), Some(INITIAL_SCORE));
    }

    #[test]
    fn should_clamp_the_score_to_its_range() {
        let peer = PeerId::random();
        let mut table = PeerTable::new();
        table.connected(peer);
        let now = Instant::now();

        for _ in 0..50 {
            table.finish_request(&peer, Outcome::Success, now);
        }
        assert_eq!(table.score(&peer), Some(MAX_SCORE));

        for _ in 0..50 {
            table.finish_request(&peer, Outcome::Failure, now);
        }
        assert_eq!(table.score(&peer), Some(MIN_SCORE));
    }

    #[test]
    fn should_keep_selecting_a_peer_sitting_at_the_score_floor() {
        let peer = PeerId::random();
        let mut table = PeerTable::new();
        table.connected(peer);
        let now = Instant::now();
        for _ in 0..50 {
            table.finish_request(&peer, Outcome::Failure, now);
        }
        assert_eq!(table.score(&peer), Some(MIN_SCORE));
        assert_eq!(table.select_for_blocks(now), Some(peer));
    }

    #[test]
    fn should_flag_only_a_peer_that_does_not_speak_the_protocol() {
        let peer = PeerId::random();
        let mut table = PeerTable::new();
        table.connected(peer);
        let now = Instant::now();

        table.finish_request(
            &peer,
            Outcome::of_request_error(&RequestError::Timeout),
            now,
        );
        assert_eq!(table.select_for_blocks(now), Some(peer));

        table.finish_request(
            &peer,
            Outcome::of_request_error(&RequestError::UnsupportedProtocol),
            now,
        );
        assert_eq!(table.select_for_blocks(now), None);
    }

    #[test]
    fn should_let_the_capability_flag_lapse() {
        let peer = PeerId::random();
        let mut table = PeerTable::new();
        table.connected(peer);
        let now = Instant::now();

        table.finish_request(&peer, Outcome::Unsupported, now);
        assert_eq!(table.select_for_blocks(now), None);
        assert_eq!(
            table.select_for_blocks(now + CAPABILITY_TTL + Duration::from_secs(1)),
            Some(peer)
        );
    }

    #[test]
    fn should_stop_selecting_a_peer_at_its_concurrency_limit() {
        let peer = PeerId::random();
        let mut table = PeerTable::new();
        table.connected(peer);
        let now = Instant::now();

        for _ in 0..MAX_CONCURRENT_REQUESTS {
            table.begin_request(&peer);
        }
        assert_eq!(table.select_for_blocks(now), None);

        table.finish_request(&peer, Outcome::Success, now);
        assert_eq!(table.select_for_blocks(now), Some(peer));
    }

    #[test]
    fn should_report_the_median_of_what_peers_claim() {
        let mut table = PeerTable::new();
        for (index, finalized) in [10u64, 20, 30, 40].into_iter().enumerate() {
            let peer = PeerId::random();
            table.connected(peer);
            table.observe_status(peer, status(finalized, finalized + index as u64));
        }
        assert_eq!(table.network_finalized_slot(), Some(Slot(20)));
        assert_eq!(table.highest_claimed_head(), Some(Slot(43)));
    }

    #[test]
    fn should_forget_a_peer_that_disconnects() {
        let peer = PeerId::random();
        let mut table = PeerTable::new();
        table.connected(peer);
        table.observe_status(peer, status(10, 12));
        table.disconnected(&peer);

        assert!(table.is_empty());
        assert_eq!(table.network_finalized_slot(), None);
    }
}
