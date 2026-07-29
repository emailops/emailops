//! One-shot CLI that populates embeddings for a demo DB by calling the app's
//! own [`services::embeddings::generate_embeddings`] pipeline.
//!
//! Using the in-tree pipeline (rather than reimplementing it in a script)
//! guarantees the demo vectors land in exactly the same vector space as the
//! query vectors the running app will produce at chat time.
//!
//! Setup it performs before delegating:
//!   1. Symlinks `<demo_dir>/models` → `<source_dir>/models` so the llamacpp
//!      runtime can find the same GGUFs the source app data dir uses (we don't want to
//!      duplicate multi-GB weights).
//!   2. Writes the `app_data_dir` preference into the demo DB — the embeddings
//!      service reads it to resolve the embed GGUF path.
//!
//! ```bash
//! cargo run --release --example embed_demo_db
//! cargo run --release --example embed_demo_db -- \
//!     --source-dir ".emailops-data" \
//!     --demo-dir "$HOME/Library/Application Support/com.emailops.app-demo"
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use emailops_lib::services::embeddings;
use emailops_lib::Database;

#[derive(Parser)]
#[command(name = "embed_demo_db")]
#[command(about = "Populate embeddings for the EmailOps demo DB")]
struct Cli {
    /// Source data dir that provides model GGUFs.
    #[arg(long = "source-dir", alias = "prod-dir")]
    source_dir: Option<PathBuf>,

    /// Target demo data dir holding the demo emailops.db.
    #[arg(long)]
    demo_dir: Option<PathBuf>,
}

fn home() -> PathBuf {
    dirs::home_dir().expect("HOME not set")
}

fn default_source_dir() -> PathBuf {
    std::env::var("EMAILOPS_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".emailops-data"))
}

fn default_demo_dir() -> PathBuf {
    home().join("Library/Application Support/com.emailops.app-demo")
}

fn ensure_models_symlink(source_dir: &Path, demo_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let src = source_dir.join("models");
    let dst = demo_dir.join("models");
    if !src.exists() {
        return Err(format!(
            "source models dir not found at {} — run the app at least once with that data dir",
            src.display()
        )
        .into());
    }
    std::fs::create_dir_all(demo_dir)?;
    // If a broken symlink exists, replace it. If a real dir/file exists, leave it.
    if dst.is_symlink() {
        let resolved = std::fs::read_link(&dst).ok();
        let still_valid = resolved.as_ref().map(|p| p == &src).unwrap_or(false) && dst.exists();
        if !still_valid {
            std::fs::remove_file(&dst)?;
        } else {
            return Ok(());
        }
    } else if dst.exists() {
        return Ok(());
    }
    // Was `#[cfg(unix)]`-gated, which silently did nothing off Unix and then
    // claimed success on the next line.
    emailops_lib::util::fs_link::link_file(&src, &dst)?;
    println!("[embed] linked {} -> {}", dst.display(), src.display());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let source_dir = cli.source_dir.unwrap_or_else(default_source_dir);
    let demo_dir = cli.demo_dir.unwrap_or_else(default_demo_dir);

    let demo_db = demo_dir.join("emailops.db");
    if !demo_db.exists() {
        return Err(format!("demo DB not found at {} — run `make demo-db` first", demo_db.display()).into());
    }

    ensure_models_symlink(&source_dir, &demo_dir)?;

    println!("[embed] opening DB at {}", demo_dir.display());
    let db = Arc::new(Database::new(demo_dir.clone())?);

    // The embeddings pipeline resolves the embed GGUF via this preference.
    db.set_preference("app_data_dir", &demo_dir.to_string_lossy())?;

    let accounts = db.list_accounts()?;
    if accounts.is_empty() {
        println!("[embed] no accounts in demo DB — nothing to do.");
        return Ok(());
    }

    for acc in accounts {
        println!("[embed] embedding emails for {} ({})", acc.email, acc.id);
        // `generate_embeddings` processes a single batch per call, so loop
        // until nothing is left pending. Hard cap mirrors `regenerate_embeddings`.
        const MAX_BATCHES: u32 = 10_000;
        let mut total = 0u32;
        for _ in 0..MAX_BATCHES {
            // `app = None` is the supported headless path — lifecycle events
            // are skipped but the work runs identically.
            let n = embeddings::generate_embeddings(&db, Some(&acc.id), None, 64, Some(&acc.email)).await?;
            if n == 0 {
                break;
            }
            total += n;
            println!("[embed]   +{n} (total {total})");
        }
        println!("[embed]   -> {total} embeddings for {}", acc.email);
    }

    println!("[embed] done.");
    Ok(())
}
