//! Shared test utilities for gik-cli integration tests.

use assert_cmd::Command;

/// Get a Command for the gik binary.
///
/// # Panics
///
/// Panics if the gik binary cannot be found. This should not happen
/// in a properly configured test environment.
#[allow(deprecated)]
pub fn gik_cmd() -> Command {
    Command::cargo_bin("gik").expect("gik binary should exist")
}
