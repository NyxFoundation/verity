//! The gossip topic grammar: `/{prefix}/{network_name}/{topic_name}/{encoding}`.
//!
//! Transcribed from leanSpec `node/networking/gossipsub/topic.py`. The `network_name`
//! segment is opaque here, exactly as it is upstream: the spec supplies it as a caller
//! string (a fork-digest rendering), and no fork-digest computation exists in the
//! networking spec to transcribe.

use std::fmt;

use verity_types::SubnetId;

/// First segment of every lean gossip topic.
pub const TOPIC_PREFIX: &str = "leanconsensus";
/// Last segment of every lean gossip topic: SSZ payloads, raw-snappy compressed.
pub const ENCODING_POSTFIX: &str = "ssz_snappy";
/// Topic-name segment for block gossip.
pub const BLOCK_TOPIC_NAME: &str = "block";
/// Topic-name segment for aggregated-attestation gossip.
pub const AGGREGATION_TOPIC_NAME: &str = "aggregation";
/// Prefix of the per-subnet attestation topic-name segment, `attestation_{subnet_id}`.
pub const ATTESTATION_SUBNET_TOPIC_PREFIX: &str = "attestation";

/// Which gossip channel a message belongs to, independent of the network it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GossipKind {
    /// `SignedBlock` gossip.
    Block,
    /// `SignedAggregatedAttestation` gossip.
    Aggregation,
    /// Per-subnet `SignedAttestation` gossip.
    Attestation(SubnetId),
}

impl GossipKind {
    /// The `topic_name` segment for this kind.
    fn topic_name(self) -> String {
        match self {
            Self::Block => BLOCK_TOPIC_NAME.to_string(),
            Self::Aggregation => AGGREGATION_TOPIC_NAME.to_string(),
            Self::Attestation(subnet) => {
                format!("{ATTESTATION_SUBNET_TOPIC_PREFIX}_{}", subnet.0)
            }
        }
    }

    /// Parses a `topic_name` segment.
    fn from_topic_name(name: &str) -> Option<Self> {
        if name == BLOCK_TOPIC_NAME {
            return Some(Self::Block);
        }
        if name == AGGREGATION_TOPIC_NAME {
            return Some(Self::Aggregation);
        }
        let subnet = name
            .strip_prefix(ATTESTATION_SUBNET_TOPIC_PREFIX)?
            .strip_prefix('_')?;
        // Leading zeros or signs would make two spellings of one subnet; refuse them so a
        // topic string has exactly one parse.
        if subnet.is_empty() || (subnet.len() > 1 && subnet.starts_with('0')) {
            return None;
        }
        let id: u64 = subnet.parse().ok()?;
        Some(Self::Attestation(SubnetId(id)))
    }
}

/// A fully-qualified gossip topic on one network.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GossipTopic {
    /// The channel.
    pub kind: GossipKind,
    /// The fork-digest segment naming the network.
    pub network_name: String,
}

impl GossipTopic {
    /// Builds the topic for `kind` on the network named `network_name`.
    #[must_use]
    pub fn new(kind: GossipKind, network_name: &str) -> Self {
        Self {
            kind,
            network_name: network_name.to_string(),
        }
    }

    /// Parses a full topic string. Returns `None` for anything that is not a lean topic —
    /// including a lean topic on a different network, which the caller distinguishes by
    /// comparing `network_name`.
    #[must_use]
    pub fn parse(topic: &str) -> Option<Self> {
        let mut segments = topic.strip_prefix('/')?.split('/');
        let prefix = segments.next()?;
        let network_name = segments.next()?;
        let topic_name = segments.next()?;
        let encoding = segments.next()?;
        if segments.next().is_some() || prefix != TOPIC_PREFIX || encoding != ENCODING_POSTFIX {
            return None;
        }
        Some(Self {
            kind: GossipKind::from_topic_name(topic_name)?,
            network_name: network_name.to_string(),
        })
    }
}

impl fmt::Display for GossipTopic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "/{TOPIC_PREFIX}/{}/{}/{ENCODING_POSTFIX}",
            self.network_name,
            self.kind.topic_name()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_format_topics_when_built_from_kinds() {
        assert_eq!(
            GossipTopic::new(GossipKind::Block, "12345678").to_string(),
            "/leanconsensus/12345678/block/ssz_snappy"
        );
        assert_eq!(
            GossipTopic::new(GossipKind::Aggregation, "12345678").to_string(),
            "/leanconsensus/12345678/aggregation/ssz_snappy"
        );
        assert_eq!(
            GossipTopic::new(GossipKind::Attestation(SubnetId(3)), "12345678").to_string(),
            "/leanconsensus/12345678/attestation_3/ssz_snappy"
        );
    }

    #[test]
    fn should_round_trip_when_parsing_formatted_topics() {
        for kind in [
            GossipKind::Block,
            GossipKind::Aggregation,
            GossipKind::Attestation(SubnetId(0)),
            GossipKind::Attestation(SubnetId(17)),
        ] {
            let topic = GossipTopic::new(kind, "0badf00d");
            let parsed = GossipTopic::parse(&topic.to_string()).expect("round trip");
            assert_eq!(parsed, topic);
        }
    }

    #[test]
    fn should_reject_topics_when_grammar_is_violated() {
        for bad in [
            "leanconsensus/net/block/ssz_snappy",    // missing leading slash
            "/eth2/net/block/ssz_snappy",            // wrong prefix
            "/leanconsensus/net/block/ssz",          // wrong encoding
            "/leanconsensus/net/blocks/ssz_snappy",  // unknown topic name
            "/leanconsensus/net/block/ssz_snappy/x", // trailing segment
            "/leanconsensus/net/attestation/ssz_snappy", // subnet-less attestation
            "/leanconsensus/net/attestation_/ssz_snappy", // empty subnet
            "/leanconsensus/net/attestation_01/ssz_snappy", // non-canonical subnet
            "/leanconsensus/net/attestation_-1/ssz_snappy", // signed subnet
        ] {
            assert!(GossipTopic::parse(bad).is_none(), "accepted: {bad}");
        }
    }
}
