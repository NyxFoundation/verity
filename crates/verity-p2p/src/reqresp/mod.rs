//! Req/resp: the three lean protocols, their envelope types, and the stream codec.

pub mod codec;
pub mod messages;

use libp2p::request_response::{self, ProtocolSupport};

use crate::config::RESP_TIMEOUT;
use crate::reqresp::codec::Codec;
use crate::reqresp::messages::Protocol;

/// One `request_response::Behaviour` per protocol, on purpose.
///
/// Upstream libp2p offers every protocol a behaviour registers when it opens an outbound
/// stream, and the *responder* picks — so a single behaviour carrying all three protocols
/// could send a `Status` request down a `blocks_by_range` stream. ethlambda solved this
/// by forking libp2p; Verity's kickoff decision (fork only on concrete, unavoidable need)
/// is satisfied without one: a behaviour that registers exactly one protocol can only
/// negotiate that protocol, and choosing the behaviour chooses the protocol.
pub fn build_behaviour(protocol: Protocol) -> request_response::Behaviour<Codec> {
    request_response::Behaviour::with_codec(
        Codec,
        [(protocol, ProtocolSupport::Full)],
        request_response::Config::default().with_request_timeout(RESP_TIMEOUT),
    )
}
