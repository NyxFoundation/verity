//! Symbolic block names, as the generator's checks refer to blocks.

use std::collections::HashMap;

use crate::common::hex;
use verity_types::Bytes32;

// ---------------------------------------------------------------------------------------
// Symbolic block names
// ---------------------------------------------------------------------------------------

/// The generator's block labels — `"block_2b"`, `"fork_a"` — resolved to roots.
///
/// The checks name blocks this way rather than by root, so a failure reads as "expected
/// `block_3`, got `block_2b`" instead of two hashes.
pub struct Labels(HashMap<String, Bytes32>);

impl Labels {
    pub fn new(anchor_root: Bytes32) -> Self {
        Self(HashMap::from([("genesis".to_string(), anchor_root)]))
    }

    pub fn insert(&mut self, label: String, root: Bytes32) {
        self.0.insert(label, root);
    }

    pub fn root(&self, label: &str) -> Result<Bytes32, String> {
        self.0
            .get(label)
            .copied()
            .ok_or_else(|| format!("no block labelled {label}"))
    }

    /// The label a root carries, or its hex when the generator never named it.
    ///
    /// Two labels can name one root: a fork built to be identical to another up to its label
    /// hashes to the same block. Only failure messages read this, so an arbitrary pick among
    /// them is fine — every assertion below compares roots.
    pub fn name(&self, root: Bytes32) -> String {
        self.0
            .iter()
            .find_map(|(label, known)| (*known == root).then(|| label.clone()))
            .unwrap_or_else(|| hex(&root))
    }

    /// Compares a root against the block a label names, reporting the mismatch by name.
    pub fn check(
        &self,
        failures: &mut Vec<String>,
        field: &str,
        label: &str,
        actual: Bytes32,
    ) -> Result<(), String> {
        let expected = self.root(label)?;
        if expected != actual {
            failures.push(format!(
                "{field}: got {}, expected {label}",
                self.name(actual)
            ));
        }
        Ok(())
    }
}
