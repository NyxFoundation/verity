//! What can stop a node from starting or force a running node to stop.
//!
//! Bad peer input stays local to the task that rejects it. Failures of owned infrastructure
//! — storage, listeners, and essential tasks — are fatal because continuing would advertise
//! a node whose consensus pipeline is no longer complete.

use core::fmt;
use std::path::PathBuf;

use verity_chain::RejectionReason;
use verity_db::StorageError;
use verity_metrics::MetricsError;
use verity_p2p::BuildError;
use verity_rpc::BindError;
use verity_types::Slot;
use verity_validator::DutyError;

use crate::sync::checkpoint::CheckpointError;

/// A configuration file the node cannot start from.
#[derive(Debug)]
pub enum ConfigError {
    /// A configuration file is missing or unreadable.
    Unreadable {
        /// The file that could not be read.
        path: PathBuf,
        /// The operating system's reason, rendered.
        reason: String,
    },
    /// A configuration file is not the shape the node expects.
    Malformed {
        /// The file that failed to parse.
        path: PathBuf,
        /// What the parser objected to.
        reason: String,
    },
    /// A genesis public key is not 52 bytes of hex.
    MalformedKey {
        /// The validator whose entry is malformed.
        index: u64,
        /// Which of the two keys is malformed.
        role: &'static str,
    },
    /// The genesis file names more validators than the state can hold.
    RegistryTooLarge {
        /// How many the file names.
        count: usize,
    },
    /// A bootnode entry is neither a signed ENR nor a multiaddr.
    MalformedBootnode {
        /// Where the entry came from: the file, or the flag.
        source: String,
        /// The entry as written.
        entry: String,
        /// What was wrong with it.
        reason: String,
    },
    /// The node key file does not hold a secp256k1 secret.
    MalformedNodeKey {
        /// The file that was read.
        path: PathBuf,
        /// What was wrong with its contents.
        reason: String,
    },
    /// A setting the deployment asked for that this build does not implement.
    UnsupportedSetting {
        /// The setting, named for the operator.
        setting: &'static str,
        /// Why it is refused.
        reason: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, reason } => {
                write!(f, "cannot read {}: {reason}", path.display())
            }
            Self::Malformed { path, reason } => {
                write!(f, "malformed {}: {reason}", path.display())
            }
            Self::MalformedBootnode {
                source,
                entry,
                reason,
            } => write!(f, "bad bootnode {entry:?} in {source}: {reason}"),
            Self::MalformedNodeKey { path, reason } => {
                write!(f, "node key {} is unusable: {reason}", path.display())
            }
            Self::UnsupportedSetting { setting, reason } => {
                write!(f, "unsupported {setting}: {reason}")
            }
            Self::MalformedKey { index, role } => write!(
                f,
                "validator {index}'s {role} public key is not 52 bytes of hex"
            ),
            Self::RegistryTooLarge { count } => write!(
                f,
                "the genesis file names {count} validators, more than the registry holds"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// A node that could not be started.
#[derive(Debug)]
pub enum NodeError {
    /// A configuration file could not be used.
    Config(ConfigError),
    /// The database could not be opened, or the anchor could not be written.
    ///
    /// An identity mismatch lands here, and it is deliberately fatal: the directory belongs to
    /// another chain, fork, or schema, and nothing local can tell which one the operator meant
    /// (`docs/design/storage.md`).
    Storage(StorageError),
    /// The stored chain could not be rebuilt into a fork-choice store.
    Restore(RejectionReason),
    /// The canonical index names a block the database does not fully hold.
    ///
    /// Corruption, not an empty database: the two are written in one batch, so a header
    /// without its body means the directory was damaged rather than left half-written.
    IncompleteChain {
        /// The block that cannot be read back whole.
        root: verity_types::Bytes32,
    },
    /// The network service could not bind, subscribe, or dial.
    Network(BuildError),
    /// Validator keys could not be loaded, which is always fatal
    /// (`docs/design/key-management.md`, Decision 2).
    Validator(DutyError),
    /// A checkpoint start was asked for and could not be completed.
    ///
    /// Fatal by design: falling back to genesis or to a stale database would start the node
    /// from an anchor the operator did not choose (`docs/design/sync.md`, Decision 1).
    Checkpoint(CheckpointError),
    /// A checkpoint start was asked for on a directory that already holds a chain.
    ///
    /// The two anchors cannot both be true, and neither may be picked silently: resuming
    /// would ignore the flag, and anchoring would leave a hole below the checkpoint. The
    /// operator points the node at an empty directory, or drops the flag.
    CheckpointOverPopulated {
        /// The slot of the anchor that was fetched.
        slot: Slot,
    },
    /// The metric registry could not be built.
    Metrics(MetricsError),
    /// An HTTP listener could not be bound.
    ///
    /// Fatal rather than skipped: a node started with `--api-port` is a node something else
    /// will probe, and a silent absence would be diagnosed there, much later.
    Http(BindError),
    /// An essential task ended while the node was supposed to be running.
    EssentialTaskStopped {
        /// The stable operator-facing task name.
        task: &'static str,
    },
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(f, "{error}"),
            Self::Storage(error) => write!(f, "storage: {error}"),
            Self::Restore(reason) => write!(f, "cannot rebuild the chain from storage: {reason}"),
            Self::IncompleteChain { root } => {
                let prefix: String = root
                    .iter()
                    .take(4)
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                write!(f, "the stored chain is incomplete at block {prefix}")
            }
            Self::Network(error) => write!(f, "network: {error}"),
            Self::Validator(error) => write!(f, "{error}"),
            Self::Checkpoint(error) => write!(f, "{error}"),
            Self::CheckpointOverPopulated { slot } => write!(
                f,
                "checkpoint sync was asked for at slot {}, but this data directory already \
                 holds a chain; point --data-dir at an empty directory or drop \
                 --checkpoint-sync-url",
                slot.0
            ),
            Self::Metrics(error) => write!(f, "{error}"),
            Self::Http(error) => write!(f, "http: {error}"),
            Self::EssentialTaskStopped { task } => {
                write!(f, "essential task {task} stopped unexpectedly")
            }
        }
    }
}

impl From<MetricsError> for NodeError {
    fn from(error: MetricsError) -> Self {
        Self::Metrics(error)
    }
}

impl From<BindError> for NodeError {
    fn from(error: BindError) -> Self {
        Self::Http(error)
    }
}

impl std::error::Error for NodeError {}

impl From<ConfigError> for NodeError {
    fn from(error: ConfigError) -> Self {
        Self::Config(error)
    }
}

impl From<StorageError> for NodeError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<RejectionReason> for NodeError {
    fn from(reason: RejectionReason) -> Self {
        Self::Restore(reason)
    }
}

impl From<BuildError> for NodeError {
    fn from(error: BuildError) -> Self {
        Self::Network(error)
    }
}

impl From<DutyError> for NodeError {
    fn from(error: DutyError) -> Self {
        Self::Validator(error)
    }
}

impl From<CheckpointError> for NodeError {
    fn from(error: CheckpointError) -> Self {
        Self::Checkpoint(error)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use verity_chain::RejectionReason;
    use verity_db::StorageError;
    use verity_p2p::BuildError;
    use verity_types::Slot;
    use verity_validator::DutyError;

    use super::{CheckpointError, ConfigError, NodeError};

    #[test]
    fn should_render_every_configuration_failure_with_operator_context() {
        let failures = [
            ConfigError::Unreadable {
                path: PathBuf::from("genesis.yaml"),
                reason: "denied".to_string(),
            },
            ConfigError::Malformed {
                path: PathBuf::from("genesis.yaml"),
                reason: "yaml".to_string(),
            },
            ConfigError::MalformedKey {
                index: 3,
                role: "proposal",
            },
            ConfigError::RegistryTooLarge { count: 99 },
            ConfigError::MalformedBootnode {
                source: "--bootnode".to_string(),
                entry: "bad".to_string(),
                reason: "invalid".to_string(),
            },
            ConfigError::MalformedNodeKey {
                path: PathBuf::from("node.key"),
                reason: "short".to_string(),
            },
            ConfigError::UnsupportedSetting {
                setting: "discovery",
                reason: "not implemented".to_string(),
            },
        ];
        for failure in failures {
            assert!(!failure.to_string().is_empty());
        }
    }

    #[test]
    fn should_render_and_convert_runtime_startup_failures() {
        let failures = [
            NodeError::from(ConfigError::RegistryTooLarge { count: 2 }),
            NodeError::from(StorageError::Backend("disk".to_string())),
            NodeError::from(RejectionReason::AnchorStateRootMismatch),
            NodeError::IncompleteChain { root: [0xab; 32] },
            NodeError::from(BuildError::Listen("occupied".to_string())),
            NodeError::from(DutyError::KeyDuplication),
            NodeError::from(CheckpointError::Rejected {
                url: "http://checkpoint".to_string(),
                reason: "bad state".to_string(),
            }),
            NodeError::CheckpointOverPopulated { slot: Slot(42) },
            NodeError::EssentialTaskStopped { task: "sync" },
        ];
        for failure in failures {
            assert!(!failure.to_string().is_empty());
        }
    }
}
