// `chat_shortcut_eval` binary — A/B/C-tests shortcut prompt variants.
//
// Usage:
//   cargo run --features eval --bin chat_shortcut_eval -- [flags]
//
// Flags:
//   --shortcut <id>          Run only this shortcut (e.g. daily_summary).
//   --variant <id>           Run only this variant across all shortcuts.
//   --use-judge              Enable OpenRouter judge scoring.
//   --model <name>           Override per-case model.
//   --out <dir>              Report output directory.
//   --prod-db <path>         Path to the prod SQLite DB.
//   --cases-dir <path>       Directory with `*.yaml` shortcut files.
//
// Env:
//   OPENROUTER_API_KEY       Required for judge mode.
//   OPENROUTER_JUDGE_MODEL   Default: anthropic/claude-sonnet-4.5.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::shortcuts::runner::{run, ShortcutRunnerConfig};

#[derive(Parser, Debug)]
#[command(
    name = "chat_shortcut_eval",
    about = "A/B/C-test shortcut prompt variants against a real mailbox.",
    long_about = None,
)]
struct Args {
    #[arg(long)]
    shortcut: Option<String>,

    #[arg(long)]
    variant: Option<String>,

    /// Enable OpenRouter judge scoring. This sends prompts, answers, and sources
    /// to OpenRouter, so it is opt-in for private mailbox evals.
    #[arg(long = "use-judge")]
    use_judge: bool,

    #[arg(long)]
    model: Option<String>,

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

    /// Use ignored private benchmark cases and private report output defaults.
    #[arg(long)]
    private: bool,

    #[arg(long)]
    cases_dir: Option<PathBuf>,
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

    let cases_dir = args
        .cases_dir
        .clone()
        .unwrap_or_else(|| default_cases_dir(args.private));

    let out_dir = args.out.clone().unwrap_or_else(|| default_out_dir(args.private));

    let cfg = ShortcutRunnerConfig {
        only_shortcut: args.shortcut,
        only_variant: args.variant,
        yes: args.use_judge,
        model_override: args.model,
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

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(run(cfg)) {
        Ok(path) => eprintln!("[shortcut-eval] done → {}", path.display()),
        Err(e) => {
            eprintln!("[shortcut-eval] ERROR: {}", e);
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
        return PathBuf::from("private-evals/chat/shortcuts");
    }
    if PathBuf::from("src-tauri/evals/chat/shortcuts").exists() {
        PathBuf::from("src-tauri/evals/chat/shortcuts")
    } else {
        PathBuf::from("evals/chat/shortcuts")
    }
}

fn default_out_dir(private: bool) -> PathBuf {
    if private {
        PathBuf::from("reports/evaluations/private/shortcuts")
    } else {
        PathBuf::from("reports/evaluations/shortcuts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_is_disabled_by_default() {
        let args = Args::try_parse_from(["chat_shortcut_eval"]).expect("parse default args");
        assert!(!args.use_judge);
    }

    #[test]
    fn use_judge_enables_judge_scoring() {
        let args = Args::try_parse_from(["chat_shortcut_eval", "--use-judge"]).expect("parse use-judge args");
        assert!(args.use_judge);
    }
}
