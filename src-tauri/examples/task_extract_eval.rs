// `task_extract_eval` binary — evaluates memory-task extraction against a
// sample of primary-category emails from the prod DB.
//
// Usage:
//   cargo run --features eval --bin task_extract_eval
//
// Flags:
//   --account <email|id>    Account to sample from. Default: alex@example.com.
//   --limit <N>             Number of emails to sample. Default: 20.
//   --model <id>            Embedded chat model id. Default: gemma-4-e2b-it-q4_k_m.
//   --provider <name>       AI provider. Default: llamacpp.
//   --use-judge             Enable OpenRouter judge scoring.
//   --out <dir>             Report dir. Default: reports/evaluations/extraction.
//   --prod-db <path>        Prod SQLite DB path. Default: platform app-data dir.
//
// Env:
//   OPENROUTER_API_KEY      Required for judge mode.
//   OPENROUTER_JUDGE_MODEL  Default: anthropic/claude-sonnet-4.5.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::extraction::runner::{run, ExtractionRunnerConfig};
use emailops_lib::evals::extraction::ExtractionKind;

#[derive(Parser, Debug)]
#[command(name = "task_extract_eval", about = "Evaluate per-email task extraction.")]
struct Args {
    #[arg(long, default_value = "alex@example.com")]
    account: String,

    #[arg(long, default_value_t = 20)]
    limit: usize,

    #[arg(long, default_value = "gemma-4-e2b-it-q4_k_m")]
    model: String,

    #[arg(long, default_value = "llamacpp")]
    provider: String,

    /// Enable OpenRouter judge scoring. This sends email excerpts to OpenRouter,
    /// so it is opt-in for private mailbox evals.
    #[arg(long = "use-judge")]
    use_judge: bool,

    /// Deprecated: judge is disabled by default. Kept as a no-op for old commands.
    #[arg(long = "no-judge", hide = true)]
    no_judge: bool,

    #[arg(long)]
    out: Option<PathBuf>,

    #[arg(long)]
    prod_db: Option<PathBuf>,

    /// Open the production DB in place instead of copying it to a temp DB.
    #[arg(long, hide = true)]
    in_place_dangerous: bool,
}

fn main() {
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

    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("reports/evaluations/extraction"));

    let cfg = ExtractionRunnerConfig {
        kind: ExtractionKind::Tasks,
        account_hint: args.account,
        limit: args.limit,
        model: args.model,
        provider_name: args.provider,
        no_judge: !args.use_judge,
        yes: args.use_judge,
        prod_db_path: prod_db,
        db_mode: if args.in_place_dangerous {
            EvalDbMode::InPlaceDangerous
        } else {
            EvalDbMode::CopyToTemp
        },
        out_dir,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(run(cfg)) {
        Ok(path) => eprintln!("[task-extract-eval] done → {}", path.display()),
        Err(e) => {
            eprintln!("[task-extract-eval] ERROR: {}", e);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_is_disabled_by_default() {
        let args = Args::try_parse_from(["task_extract_eval"]).expect("parse default args");
        assert!(!args.use_judge);
    }

    #[test]
    fn use_judge_enables_judge_scoring() {
        let args = Args::try_parse_from(["task_extract_eval", "--use-judge"]).expect("parse use-judge args");
        assert!(args.use_judge);
    }
}
