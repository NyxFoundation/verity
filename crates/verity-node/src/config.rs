//! The files a lean node is started from.
//!
//! # Why these shapes and not our own
//!
//! Two YAML files describe a lean network, and every client on it reads the same two. Verity
//! reads them unchanged rather than defining its own configuration, because interoperating
//! with lean-quickstart's generator — and with the devnets built from it — is the entire
//! point of having a format at all.
//!
//! - **The genesis file** (`--genesis`) fixes when slot 0 begins and which validators secure
//!   the chain, in registry order. Its keys are `SCREAMING_CASE` because that is what the
//!   cross-client format uses.
//!
//!   ```yaml
//!   GENESIS_TIME: 1704085200
//!   GENESIS_VALIDATORS:
//!     - attestation_public_key: 0xe2a0...
//!       proposal_public_key: 0x51c8...
//!   ```
//!
//! - **The assignment file** (`validators.yaml`, beside the keys) maps each node's identifier
//!   to the validator indices it runs. A node whose identifier is absent runs none, which is
//!   a legitimate configuration — it follows the chain without signing.
//!
//!   ```yaml
//!   lean_spec_0: [0, 1]
//!   lean_spec_1: [2]
//!   ```
//!
//! Public keys are accepted quoted or bare. A 52-byte hex value is far too wide for any
//! integer type, so a YAML parser hands it back as a scalar string either way — which is what
//! makes the unquoted form the generator writes readable without a numeric pre-pass.
//!
//! Transcribed from leanSpec `src/lean_spec/node/genesis.py` and
//! `src/lean_spec/node/validator/registry.py`, read at commit `8603fa63`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use verity_types::{Bytes52, Validator, ValidatorIndex, Validators};

use crate::error::ConfigError;

/// File name of the node-to-validator assignment, in the validator-keys base directory.
pub const ASSIGNMENT_FILE_NAME: &str = "validators.yaml";

/// Subdirectory of the validator-keys base directory holding the key files and their manifest.
pub const KEY_SUBDIRECTORY: &str = "hash-sig-keys";

/// The network-wide origin: when slot 0 begins, and who secures the chain.
#[derive(Debug, Clone, Deserialize)]
pub struct GenesisFile {
    /// Unix timestamp in seconds at which slot 0 begins.
    #[serde(rename = "GENESIS_TIME")]
    pub genesis_time: u64,
    /// The validators present at slot 0, in registry order.
    #[serde(rename = "GENESIS_VALIDATORS")]
    pub genesis_validators: Vec<GenesisValidator>,
}

/// One validator's two public keys, as the genesis file carries them.
#[derive(Debug, Clone, Deserialize)]
pub struct GenesisValidator {
    /// XMSS public key for signing attestations, hex, `0x` prefix optional.
    pub attestation_public_key: String,
    /// XMSS public key the proposer signs the block root with.
    pub proposal_public_key: String,
}

impl GenesisFile {
    /// Reads and validates a genesis file.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Unreadable`] when the file cannot be read and
    /// [`ConfigError::Malformed`] when it is not the shape above.
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|error| ConfigError::Unreadable {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
        serde_norway::from_str(&text).map_err(|error| ConfigError::Malformed {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })
    }

    /// The registry the chain starts with, each validator indexed by its position.
    ///
    /// # Errors
    ///
    /// [`ConfigError::MalformedKey`] when a public key is not 52 bytes of hex, and
    /// [`ConfigError::RegistryTooLarge`] when the file names more validators than the state
    /// can hold.
    pub fn to_validators(&self) -> Result<Validators, ConfigError> {
        let validators = self
            .genesis_validators
            .iter()
            .enumerate()
            .map(|(position, entry)| {
                let index = position as u64;
                Ok(Validator {
                    attestation_public_key: key_bytes(
                        &entry.attestation_public_key,
                        index,
                        "attestation",
                    )?,
                    proposal_public_key: key_bytes(&entry.proposal_public_key, index, "proposal")?,
                    index: ValidatorIndex(index),
                })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;

        let count = validators.len();
        Validators::try_from(validators).map_err(|_| ConfigError::RegistryTooLarge { count })
    }
}

/// Which validators this node runs, from the assignment file beside the keys.
///
/// A missing file, or an identifier the file does not name, means this node runs no
/// validators. That is a configuration, not a failure: a follower node is a normal thing to
/// operate.
///
/// # Errors
///
/// [`ConfigError::Malformed`] when the file exists but is not a mapping of node identifier to
/// a list of indices.
pub fn assigned_validators(path: &Path, node_id: &str) -> Result<Vec<ValidatorIndex>, ConfigError> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };

    let assignment: HashMap<String, Vec<u64>> =
        serde_norway::from_str(&text).map_err(|error| ConfigError::Malformed {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;

    Ok(assignment
        .get(node_id)
        .map(|indices| indices.iter().copied().map(ValidatorIndex).collect())
        .unwrap_or_default())
}

fn key_bytes(text: &str, index: u64, role: &'static str) -> Result<Bytes52, ConfigError> {
    let trimmed = text.strip_prefix("0x").unwrap_or(text);
    let bytes = hex::decode(trimmed).map_err(|_| ConfigError::MalformedKey { index, role })?;
    bytes
        .try_into()
        .map_err(|_| ConfigError::MalformedKey { index, role })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use verity_types::ValidatorIndex;

    use super::{GenesisFile, assigned_validators};

    fn write(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("a temporary file");
        file.write_all(contents.as_bytes()).expect("write");
        file.flush().expect("flush");
        file
    }

    fn genesis_yaml(quote: bool) -> String {
        let attestation = "0x".to_string() + &"11".repeat(52);
        let proposal = "0x".to_string() + &"22".repeat(52);
        let wrap = |key: &str| {
            if quote {
                format!("\"{key}\"")
            } else {
                key.to_string()
            }
        };
        format!(
            "GENESIS_TIME: 1704085200\nGENESIS_VALIDATORS:\n  - attestation_public_key: {}\n    proposal_public_key: {}\n",
            wrap(&attestation),
            wrap(&proposal)
        )
    }

    #[test]
    fn should_read_a_genesis_file_whose_keys_are_quoted() {
        let file = write(&genesis_yaml(true));
        let genesis = GenesisFile::read(file.path()).expect("a genesis file");

        assert_eq!(genesis.genesis_time, 1_704_085_200);
        let validators = genesis.to_validators().expect("one validator");
        assert_eq!(validators.len(), 1);
        assert_eq!(validators[0].attestation_public_key, [0x11u8; 52]);
        assert_eq!(validators[0].proposal_public_key, [0x22u8; 52]);
    }

    #[test]
    fn should_read_a_genesis_file_whose_keys_are_bare() {
        let file = write(&genesis_yaml(false));
        let genesis = GenesisFile::read(file.path()).expect("a genesis file");
        let validators = genesis.to_validators().expect("one validator");

        assert_eq!(validators[0].attestation_public_key, [0x11u8; 52]);
    }

    #[test]
    fn should_refuse_a_public_key_that_is_not_fifty_two_bytes() {
        let file = write(
            "GENESIS_TIME: 1\nGENESIS_VALIDATORS:\n  - attestation_public_key: \"0x1234\"\n    proposal_public_key: \"0x5678\"\n",
        );
        let genesis = GenesisFile::read(file.path()).expect("a genesis file");

        assert!(genesis.to_validators().is_err());
    }

    #[test]
    fn should_index_validators_by_their_position_in_the_file() {
        let entry = |seed: &str| {
            format!(
                "  - attestation_public_key: \"0x{}\"\n    proposal_public_key: \"0x{}\"\n",
                seed.repeat(52),
                seed.repeat(52)
            )
        };
        let file = write(&format!(
            "GENESIS_TIME: 1\nGENESIS_VALIDATORS:\n{}{}",
            entry("aa"),
            entry("bb")
        ));

        let validators = GenesisFile::read(file.path())
            .expect("a genesis file")
            .to_validators()
            .expect("two validators");

        assert_eq!(validators[0].index, ValidatorIndex(0));
        assert_eq!(validators[1].index, ValidatorIndex(1));
    }

    #[test]
    fn should_return_the_indices_the_assignment_file_names_for_this_node() {
        let file = write("lean_spec_0:\n  - 0\n  - 1\nlean_spec_1:\n  - 2\n");

        assert_eq!(
            assigned_validators(file.path(), "lean_spec_0").expect("an assignment"),
            vec![ValidatorIndex(0), ValidatorIndex(1)]
        );
        assert_eq!(
            assigned_validators(file.path(), "lean_spec_1").expect("an assignment"),
            vec![ValidatorIndex(2)]
        );
    }

    #[test]
    fn should_run_no_validators_when_the_assignment_does_not_name_this_node() {
        let file = write("lean_spec_0:\n  - 0\n");
        assert!(
            assigned_validators(file.path(), "verity_0")
                .expect("an assignment")
                .is_empty()
        );
    }

    #[test]
    fn should_run_no_validators_when_the_assignment_file_is_absent() {
        assert!(
            assigned_validators(std::path::Path::new("/nonexistent/validators.yaml"), "any")
                .expect("an absent file is a follower configuration")
                .is_empty()
        );
    }
}
