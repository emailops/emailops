//! Create (or bring up to date) `<data-dir>/emailops.db` by opening it the way
//! the app does: every embedded migration of this checkout, the sqlite-vec
//! virtual tables and the FTS index.
//!
//! `scripts/generate_demo_db.py` calls it to get the demo DB's schema from the
//! migrations of the branch it runs on, rather than from the developer's
//! production DB (which lags behind any branch that adds a migration).
//!
//! ```bash
//! cargo run --no-default-features --manifest-path src-tauri/Cargo.toml \
//!     --example init_db -- /path/to/data-dir
//! ```
//!
//! Prints the highest applied migration version on stdout.

use std::path::PathBuf;
use std::process::ExitCode;

use emailops_lib::Database;

fn main() -> ExitCode {
    let Some(data_dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: init_db <data-dir>");
        return ExitCode::from(2);
    };
    let db = match Database::new(data_dir.clone()) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("[init_db] cannot open {}: {e}", data_dir.display());
            return ExitCode::FAILURE;
        }
    };
    let version: rusqlite::Result<i64> =
        db.reader()
            .query_row("SELECT MAX(version) FROM refinery_schema_history", [], |row| row.get(0));
    match version {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[init_db] cannot read the schema history: {e}");
            ExitCode::FAILURE
        }
    }
}
