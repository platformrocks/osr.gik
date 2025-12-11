//! # gik CLI
//!
//! Command-line interface for the Guided Indexing Kernel.
//!
//! This binary provides human-friendly access to `gik-core` functionality.
//! Run `gik --help` for usage information.

mod cli;
pub mod ui;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
