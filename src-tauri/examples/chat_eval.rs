// `chat_eval` binary — CLI entry point for the Rust-native chat eval harness.
//
// Usage:
//   cargo run --features eval --bin chat_eval -- [flags]
//
// Flags:
//   --tier smoke|full|all          Which tier to run (default: smoke).
//   --case <id>                    Run only the case with this id.
//   --use-judge                    Enable OpenRouter judge scoring.
//   --model <name>                 Override per-case / global model.
//   --account <id>                 Scope retrieval to this account (required
//                                  if multiple enabled accounts are present).
//   --out <dir>                    Report output directory.
//
// Env (optional):
//   OPENROUTER_API_KEY             Required for judge mode.
//   OPENROUTER_JUDGE_MODEL         Default: anthropic/claude-sonnet-4.5.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::runner::{run, RunnerConfig};

#[derive(Parser, Debug)]
#[command(
    name = "chat_eval",
    about = "Run EmailOps chat evals against a copy of the production DB.",
    long_about = None,
)]
struct Args {
    #[arg(long, default_value = "smoke")]
    tier: String,

    #[arg(long)]
    case: Option<String>,

    /// Enable OpenRouter judge scoring. This sends questions, answers, and sources
    /// to OpenRouter, so it is opt-in for private mailbox evals.
    #[arg(long = "use-judge")]
    use_judge: bool,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    account: Option<String>,

    /// Deprecated: judge is disabled by default. Kept as a no-op for old commands.
    #[arg(long = "no-judge", hide = true)]
    no_judge: bool,

    #[arg(long)]
    out: Option<PathBuf>,

    /// Path to the production SQLite DB to copy.  Defaults to the macOS
    /// production location: ~/Library/Application Support/com.emailops.app/emailops.db
    #[arg(long)]
    prod_db: Option<PathBuf>,

    /// Open the production DB in place instead of copying it to a temp DB.
    #[arg(long, hide = true)]
    in_place_dangerous: bool,

    /// Use ignored private benchmark cases and private report output defaults.
    #[arg(long)]
    private: bool,

    /// Directory containing `*.yaml` case files.
    /// Defaults to `src-tauri/evals/chat/cases`.
    #[arg(long)]
    cases_dir: Option<PathBuf>,
}

fn main() {
    // Load .env from a few common locations.
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }

    let args = Args::parse();

    let prod_db = args
        .prod_db
        .clone()
        .or_else(default_prod_db)
        .unwrap_or_else(|| PathBuf::from("emailops.db"));

    let cases_dir = args
        .cases_dir
        .clone()
        .unwrap_or_else(|| default_cases_dir(args.private));

    let out_dir = args.out.clone().unwrap_or_else(|| default_out_dir(args.private));

    let cfg = RunnerConfig {
        tier: args.tier,
        only_case: args.case,
        yes: args.use_judge,
        model_override: args.model,
        account_id: args.account,
        no_judge: !args.use_judge,
        out_dir,
        cases_dir,
        prod_db_path: prod_db,
        db_mode: if args.in_place_dangerous {
            EvalDbMode::InPlaceDangerous
        } else {
            EvalDbMode::CopyToTemp
        },
    };

    // Use Tokio to drive async code. A single-thread runtime is enough —
    // cases run serially.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(run(cfg)) {
        Ok(path) => {
            eprintln!("[eval] done → {}", path.display());
        }
        Err(e) => {
            eprintln!("[eval] ERROR: {}", e);
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "macos")]
fn default_prod_db() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("Application Support")
            .join("com.emailops.app")
            .join("emailops.db")
    })
}

#[cfg(not(target_os = "macos"))]
fn default_prod_db() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("com.emailops.app").join("emailops.db"))
}

fn default_cases_dir(private: bool) -> PathBuf {
    if private {
        return PathBuf::from("private-evals/chat/cases");
    }
    if PathBuf::from("src-tauri/evals/chat/cases").exists() {
        PathBuf::from("src-tauri/evals/chat/cases")
    } else {
        PathBuf::from("evals/chat/cases")
    }
}

fn default_out_dir(private: bool) -> PathBuf {
    if private {
        PathBuf::from("reports/evaluations/private/chat")
    } else {
        PathBuf::from("reports/evaluations/chat")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_is_disabled_by_default() {
        let args = Args::try_parse_from(["chat_eval"]).expect("parse default args");
        assert!(!args.use_judge);
    }

    #[test]
    fn use_judge_enables_judge_scoring() {
        let args = Args::try_parse_from(["chat_eval", "--use-judge"]).expect("parse use-judge args");
        assert!(args.use_judge);
    }
}
