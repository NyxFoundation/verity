//! What can stop a node from starting.
//!
//! Everything here is a startup failure. Once the tasks are up, a failure is a dropped
//! message, a rejected block, or a missed duty — all of which are handled where they happen
//! and logged, because a consensus node that exits on bad input is a node an adversary can
//! turn off.

use core::fmt;
use std::path::PathBuf;

use verity_chain::RejectionReason;
use verity_db::StorageError;
use verity_p2p::BuildError;
use verity_validator::DutyError;

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
        }
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
