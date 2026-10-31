//! Command-line interface for WaveAssert.
//!
//! Placeholder binary: it reports its version. The `diff`, `review`, `accept`
//! and `reject` subcommands are added by later milestones.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::process::ExitCode;

fn main() -> ExitCode {
    if let Some("--version" | "-V") = std::env::args().nth(1).as_deref() {
        println!("waveassert {}", env!("CARGO_PKG_VERSION"));
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "waveassert {}: audio snapshot checking is not implemented yet.",
            env!("CARGO_PKG_VERSION")
        );
        eprintln!("Usage: waveassert --version");
        ExitCode::from(2)
    }
}
