// `email_classification_eval` binary — evaluates an email category classifier
// against a sample of primary-category emails from the prod DB.
//
// Default backend is the embedded llama.cpp runtime: pass a catalog model id
// (e.g. `qwen3.5-4b-q4_k_m`, `qwen3.5-9b-q4_k_m`) and the eval will route
// through the same `AiService` path the running app uses. The original
// `distil-labs/distil-email-classifier` Ollama flow remains available by
// passing `--provider ollama --model email-classifier`.
//
// Usage:
//   # Embedded llama.cpp (default — used by `make eval-all`)
//   cargo run --features eval --example email_classification_eval -- \
//     --model qwen3.5-9b-q4_k_m
//
//   # Original Ollama-served distil fine-tune
//   cargo run --features eval --example email_classification_eval -- \
//     --provider ollama --model email-classifier
//
// Flags:
//   --account <email|id>    Account to sample from. Default: alex@northwindlabs.io.
//   --limit <N>             Number of emails to sample. Default: 100.
//   --model <id>            Catalog model id (llamacpp) or Ollama tag.
//                           Default: qwen3.5-4b-q4_k_m.
//   --provider <name>       AI provider. Default: llamacpp.
//   --use-judge             Enable OpenRouter judge scoring.
//   --out <dir>             Report dir. Default: reports/evaluations/email_classification.
//   --prod-db <path>        Prod SQLite DB path. Default: platform app-data dir.
//
// Env (optional):
//   EMAILOPS_EVAL_MODEL     Overrides --model. Set by `make eval-all MODEL=…`.
//   EMAILOPS_EVAL_PROVIDER  Overrides --provider. Defaults to `llamacpp` when
//                           only EMAILOPS_EVAL_MODEL is set.
//   OPENROUTER_API_KEY      Required for judge mode.
//   OPENROUTER_JUDGE_MODEL  Default: anthropic/claude-sonnet-4.5.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::email_classification::runner::{run, EmailClassificationConfig};

#[derive(Parser, Debug)]
#[command(
    name = "email_classification_eval",
    about = "Evaluate distil-email-classifier against primary-inbox emails."
)]
struct Args {
    #[arg(long, default_value = "alex@northwindlabs.io")]
    account: String,

    #[arg(long, default_value_t = 100)]
    limit: usize,

    #[arg(long, default_value = "qwen3.5-4b-q4_k_m")]
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
        .unwrap_or_else(|| PathBuf::from("reports/evaluations/email_classification"));

    let cfg = EmailClassificationConfig {
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
        Ok(path) => eprintln!("[classify-eval] done → {}", path.display()),
        Err(e) => {
            eprintln!("[classify-eval] ERROR: {}", e);
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
        let args = Args::try_parse_from(["email_classification_eval"]).expect("parse default args");
        assert!(!args.use_judge);
    }

    #[test]
    fn use_judge_enables_judge_scoring() {
        let args = Args::try_parse_from(["email_classification_eval", "--use-judge"]).expect("parse use-judge args");
        assert!(args.use_judge);
    }

    #[test]
    fn defaults_to_llamacpp_with_qwen3_5_4b() {
        // After the Ollama → llama.cpp migration, the default backend must
        // be the embedded runtime against the 4B Qwen catalog id so
        // `make eval-all` produces an apples-to-apples 4B baseline.
        let args = Args::try_parse_from(["email_classification_eval"]).expect("parse default args");
        assert_eq!(args.provider, "llamacpp");
        assert_eq!(args.model, "qwen3.5-4b-q4_k_m");
    }
}
