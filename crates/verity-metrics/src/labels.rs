//! The label enums leanMetrics fixes, with the exact strings each value is exposed under.
//!
//! Every enum here is closed: the contract names its values, and a client that emits one
//! outside the list breaks the dashboard that reads it. Each carries `ALL`, which is how the
//! registry seeds every series at zero.

/// Declares a label enum together with its wire strings and the full value list.
macro_rules! label_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$variant_meta:meta])* $variant:ident => $label:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$variant_meta])* $variant),+
        }

        impl $name {
            /// Every value, in contract order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The label value leanMetrics fixes for this variant.
            #[must_use]
            pub const fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => $label),+
                }
            }

            /// The wire strings of every value, in contract order.
            #[must_use]
            pub fn labels() -> Vec<&'static str> {
                Self::ALL.iter().map(|value| value.label()).collect()
            }
        }
    };
}

label_enum! {
    /// The `status` label of `lean_node_sync_status`.
    SyncStatus {
        /// Not syncing and not synced: the node has not met the network yet.
        Idle => "idle",
        /// Behind the network and fetching.
        Syncing => "syncing",
        /// Caught up.
        Synced => "synced",
    }
}

label_enum! {
    /// The `result` label of `lean_finalizations_total`.
    FinalizationResult {
        /// The finalized checkpoint advanced.
        Success => "success",
        /// A justifiable slot sat between source and target, so nothing finalized.
        Error => "error",
    }
}

label_enum! {
    /// The `reason` label of `lean_aggregator_skipped_total`.
    SkipReason {
        /// This node runs no aggregation duty.
        NotAggregator => "not_aggregator",
        /// Wall-clock lag gated the round.
        NotSynced => "not_synced",
        /// The pre-state the round needs is not resolvable.
        MissingState => "missing_state",
        /// The aggregation worker could not be started.
        SpawnFailed => "spawn_failed",
        /// Anything the other reasons do not cover.
        Other => "other",
    }
}

label_enum! {
    /// The `direction` label of the peer connection and disconnection counters.
    Direction {
        /// The peer dialed this node.
        Inbound => "inbound",
        /// This node dialed the peer.
        Outbound => "outbound",
    }
}

label_enum! {
    /// The `result` label of `lean_peer_connection_events_total`.
    ConnectionResult {
        /// The connection was established.
        Success => "success",
        /// The attempt timed out.
        Timeout => "timeout",
        /// The attempt failed for any other reason.
        Error => "error",
    }
}

label_enum! {
    /// The `reason` label of `lean_peer_disconnection_events_total`.
    DisconnectReason {
        /// The keep-alive expired.
        Timeout => "timeout",
        /// The peer closed the connection.
        RemoteClose => "remote_close",
        /// This node closed the connection.
        LocalClose => "local_close",
        /// The transport failed.
        Error => "error",
    }
}

label_enum! {
    /// The `position` label of the gossip arrival counters.
    ArrivalPosition {
        /// Arrived before the interval it was due in began.
        Before => "before",
        /// Arrived inside that interval.
        Inside => "inside",
        /// Arrived after that interval ended.
        After => "after",
    }
}

impl ArrivalPosition {
    /// The positions an arrival can take when its anchor never follows it: the aggregation
    /// counter measures from the most recent boundary at or before the arrival, so `before`
    /// is unreachable and must not be exposed as an empty series.
    pub const NON_NEGATIVE: &'static [Self] = &[Self::Inside, Self::After];
}

#[cfg(test)]
mod tests {
    use super::{ArrivalPosition, SkipReason};

    #[test]
    fn should_list_every_value_in_contract_order() {
        assert_eq!(
            SkipReason::labels(),
            [
                "not_aggregator",
                "not_synced",
                "missing_state",
                "spawn_failed",
                "other"
            ]
        );
        assert_eq!(ArrivalPosition::labels(), ["before", "inside", "after"]);
    }
}
