// `emailops-cli` — power-user / agent-driven command line for EmailOps.
//
// Thin wrapper: all logic lives in `emailops_lib::cli` so it stays unit
// testable and the bin contributes almost nothing to compile time. Gated
// behind the `cli` cargo feature (see Cargo.toml `[[bin]]`).

use std::process::ExitCode;

fn main() -> ExitCode {
    emailops_lib::cli::run()
}
