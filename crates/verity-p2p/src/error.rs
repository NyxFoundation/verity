//! Failure types crossing the service API.

use libp2p::request_response::OutboundFailure;

use crate::reqresp::messages::ErrorCode;

/// Why an outbound req/resp request produced no [`crate::Response`].
///
/// Transport-level failures only. A peer that answered with an error *response* is not a
/// failure of the request machinery — it arrives as [`crate::Response::Error`] — with one
/// exception: an error chunk that is itself malformed (bad framing, oversized message) is
/// indistinguishable from a broken stream and lands in [`RequestError::Io`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// No response arrived within the protocol timeout.
    Timeout,
    /// The connection closed before a response arrived.
    ConnectionClosed,
    /// The peer does not speak the requested protocol at all. `docs/design/sync.md`
    /// Decision 3 makes this the one outcome that sets a peer's capability flag.
    UnsupportedProtocol,
    /// Dialing the peer failed.
    DialFailure,
    /// The stream broke or its content violated the framing rules.
    Io(String),
    /// The network task stopped before the request completed.
    ServiceStopped,
}

impl RequestError {
    /// Collapses libp2p's failure taxonomy onto the outcomes the sync service's scoring
    /// table distinguishes (`docs/design/sync.md`, Decision 3).
    #[must_use]
    pub fn from_outbound_failure(failure: &OutboundFailure) -> Self {
        match failure {
            OutboundFailure::Timeout => Self::Timeout,
            OutboundFailure::ConnectionClosed => Self::ConnectionClosed,
            OutboundFailure::UnsupportedProtocols => Self::UnsupportedProtocol,
            OutboundFailure::DialFailure => Self::DialFailure,
            OutboundFailure::Io(e) => Self::Io(e.to_string()),
        }
    }
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "request timed out"),
            Self::ConnectionClosed => write!(f, "connection closed before a response arrived"),
            Self::UnsupportedProtocol => write!(f, "peer does not support the protocol"),
            Self::DialFailure => write!(f, "dialing the peer failed"),
            Self::Io(e) => write!(f, "stream failure: {e}"),
            Self::ServiceStopped => write!(f, "network service stopped"),
        }
    }
}

impl std::error::Error for RequestError {}

/// Why a publish did not reach the mesh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// No mesh peer is subscribed to the topic yet. Transient during startup — the mesh
    /// forms on the gossipsub heartbeat — so callers may retry.
    InsufficientPeers,
    /// The exact message is already in the duplicate cache.
    Duplicate,
    /// The payload exceeds the transmit cap.
    PayloadTooLarge,
    /// Any other gossipsub-internal failure.
    Other(String),
    /// The network task stopped.
    ServiceStopped,
}

impl From<libp2p::gossipsub::PublishError> for PublishError {
    fn from(error: libp2p::gossipsub::PublishError) -> Self {
        use libp2p::gossipsub::PublishError as Upstream;
        match error {
            Upstream::NoPeersSubscribedToTopic => Self::InsufficientPeers,
            Upstream::Duplicate => Self::Duplicate,
            Upstream::MessageTooLarge => Self::PayloadTooLarge,
            other => Self::Other(other.to_string()),
        }
    }
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InsufficientPeers => write!(f, "no mesh peer subscribed to the topic"),
            Self::Duplicate => write!(f, "message already published"),
            Self::PayloadTooLarge => write!(f, "payload exceeds the transmit cap"),
            Self::Other(e) => write!(f, "publish failed: {e}"),
            Self::ServiceStopped => write!(f, "network service stopped"),
        }
    }
}

impl std::error::Error for PublishError {}

/// Why a dial command failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// The network task stopped.
    ServiceStopped,
    /// The command was accepted by the task but failed immediately, e.g. a malformed
    /// dial address.
    Failed(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServiceStopped => write!(f, "network service stopped"),
            Self::Failed(e) => write!(f, "command failed: {e}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// Why the network service could not be built at all.
#[derive(Debug)]
pub enum BuildError {
    /// Assembling the behaviour failed.
    Behaviour(String),
    /// Binding the listen address failed.
    Listen(String),
    /// Dialing a configured bootnode failed immediately (malformed address).
    Bootnode(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Behaviour(e) => write!(f, "building the behaviour failed: {e}"),
            Self::Listen(e) => write!(f, "binding the listen address failed: {e}"),
            Self::Bootnode(e) => write!(f, "dialing a bootnode failed: {e}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest => write!(f, "invalid request"),
            Self::ServerError => write!(f, "server error"),
            Self::ResourceUnavailable => write!(f, "resource unavailable"),
        }
    }
}
