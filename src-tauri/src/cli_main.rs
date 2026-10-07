// `emailops-cli` — power-user / agent-driven command line for EmailOps.
//
// Thin wrapper: all logic lives in `emailops_lib::cli` so it stays unit
// testable and the bin contributes almost nothing to compile time. Gated
// behind the `cli` cargo feature (see Cargo.toml `[[bin]]`).

use std::process::ExitCode;

fn main() -> ExitCode {
    // Before anything can load a DLL (DASA 1.5.2); see `src/main.rs`.
    #[cfg(windows)]
    if let Err(e) = emailops_lib::util::dll_search::restrict_dll_search() {
        eprintln!("[startup] Warning: could not restrict the DLL search order: {e}");
    }
    emailops_lib::cli::run()
}
