//! `verity-types` — foundational consensus container definitions and constants.
//!
//! **CI canary stub.** This crate currently holds only a placeholder so the Rust quality gate
//! (`cargo fmt` / `clippy` / `test` / `build`) has real code to run against before implementation
//! begins. Replace this with the actual container definitions (`Block`, `State`, `Vote`, …) and
//! constants when implementation starts.

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
