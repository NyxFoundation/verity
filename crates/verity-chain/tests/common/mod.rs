//! Shared plumbing for the leanSpec fixture suites.
//!
//! Both suites are gated on `VERITY_FIXTURES` pointing at an extracted `fixtures-prod-scheme`
//! tree. The fast `cargo test` gate leaves it unset and the tests return; CI's fixtures job
//! always sets it, and each suite fails if no case matched.

use std::fs;
use std::path::{Path, PathBuf};

/// The extracted fixture tree, when the environment points at one.
pub fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("VERITY_FIXTURES").map(PathBuf::from)
}

/// Every `*.json` under a directory named `suite`, anywhere in the tree.
///
/// Matching on the suite directory rather than a fixed depth keeps this working when leanSpec
/// moves a suite, which it has already done once.
pub fn collect_suite_json(root: &Path, suite: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, suite, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, suite: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, suite, out);
            continue;
        }
        let is_json = path.extension().is_some_and(|ext| ext == "json");
        let in_suite = path.components().any(|c| c.as_os_str() == suite);
        if is_json && in_suite {
            out.push(path);
        }
    }
}

/// Reads every case in a suite, keyed by the leanSpec test id that produced it.
pub fn read_cases<T: serde::de::DeserializeOwned>(paths: &[PathBuf]) -> Vec<(String, T)> {
    let mut cases = Vec::new();
    for path in paths {
        let text = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{}: read error: {error}", path.display()));
        let file: std::collections::BTreeMap<String, T> = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{}: json: {error}", path.display()));
        cases.extend(file);
    }
    cases
}
