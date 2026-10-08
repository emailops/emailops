// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Before anything can load a DLL (DASA 1.5.2). Not fatal: on failure the
    // process keeps Windows' default search order, as every release before.
    #[cfg(windows)]
    if let Err(e) = emailops_lib::util::dll_search::restrict_dll_search() {
        eprintln!("[startup] Warning: could not restrict the DLL search order: {e}");
    }
    emailops_lib::run()
}
