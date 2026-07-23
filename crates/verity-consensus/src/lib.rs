//! `verity-consensus` — the single starting crate of the Verity consensus client.
//!
//! Per the 2026-07-22 kickoff decision, implementation begins in this one crate: shared
//! types, STF, fork choice, and proposer selection live here as modules, and the
//! ARCHITECTURE.md workspace layout is what this crate later splits into once a second
//! crate earns its existence.
//!
//! **CI canary stub.** This crate currently holds only a placeholder so the Rust quality gate
//! (`cargo fmt` / `clippy` / `test` / `build`) has real code to run against before
//! implementation begins.

/// Returns the crate's semantic version string, as recorded in `Cargo.toml`.
///
/// Placeholder exercised by the CI canary; remove once real types land.
#[must_use]
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::crate_version;

    #[test]
    fn should_report_package_version_when_called() {
        assert_eq!(crate_version(), env!("CARGO_PKG_VERSION"));
    }
}
