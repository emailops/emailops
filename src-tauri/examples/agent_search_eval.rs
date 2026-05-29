// `agent_search_eval` — CLI entry for the agentic-search eval harness.
//
// Usage:
//   cargo run --features eval --bin agent_search_eval -- --use-judge
//
// Flags mirror chat_eval where it makes sense. Modes are chosen via repeated
// --mode flags; default is baseline + smart so we always have a head-to-head.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::agent_search::{run_agent_search_eval, RunConfig};
use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::services::agent_search::AgentSearchMode;

#[derive(Parser, Debug)]
#[command(
    name = "agent_search_eval",
    about = "Pooled-recall eval for EmailOps agentic search.",
    long_about = None,
)]
struct Args {
    /// Enable OpenRouter judge scoring. This sends pooled email snippets to
    /// OpenRouter, so it is opt-in for private mailbox evals.
    #[arg(long = "use-judge")]
    use_judge: bool,

    /// Deprecated: judge is disabled by default. Kept as a no-op for old commands.
    #[arg(long = "no-judge", hide = true)]
    no_judge: bool,

    /// Limit run to a single case by id.
    #[arg(long)]
    case: Option<String>,

    /// Top-K used for both retrieval cutoff and metrics. Default 15.
    #[arg(long, default_value_t = 15)]
    top_k: usize,

    /// Modes to evaluate. Repeat to run multiple. Default: baseline,smart.
    #[arg(long = "mode", value_enum)]
    modes: Vec<ModeArg>,

    /// Path to the production SQLite DB.
    #[arg(long)]
    prod_db: Option<PathBuf>,

    /// Open the production DB in place instead of copying it to a temp DB.
    #[arg(long, hide = true)]
    in_place_dangerous: bool,

    /// Use ignored private benchmark cases and private report output defaults.
    #[arg(long)]
    private: bool,

    /// Directory containing `*.yaml` case files.
    #[arg(long)]
    cases_dir: Option<PathBuf>,

    /// Report output directory.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ModeArg {
    Baseline,
    Hybrid,
    Smart,
}

impl From<ModeArg> for AgentSearchMode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Baseline => AgentSearchMode::Baseline,
            ModeArg::Hybrid => AgentSearchMode::Hybrid,
            ModeArg::Smart => AgentSearchMode::Smart,
        }
    }
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }
    let args = Args::parse();

    let modes: Vec<AgentSearchMode> = if args.modes.is_empty() {
        vec![AgentSearchMode::Baseline, AgentSearchMode::Smart]
    } else {
        args.modes.iter().copied().map(Into::into).collect()
    };

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

    let cfg = RunConfig {
        cases_dir,
        out_dir,
        prod_db_path: prod_db,
        yes: args.use_judge,
        no_judge: !args.use_judge,
        db_mode: if args.in_place_dangerous {
            EvalDbMode::InPlaceDangerous
        } else {
            EvalDbMode::CopyToTemp
        },
        top_k: args.top_k,
        only_case: args.case,
        modes,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(run_agent_search_eval(cfg)) {
        Ok(path) => eprintln!("[agent-search-eval] done → {}", path.display()),
        Err(e) => {
            eprintln!("[agent-search-eval] ERROR: {}", e);
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
        return PathBuf::from("private-evals/agent_search");
    }
    if PathBuf::from("src-tauri/evals/agent_search").exists() {
        PathBuf::from("src-tauri/evals/agent_search")
    } else {
        PathBuf::from("evals/agent_search")
    }
}

fn default_out_dir(private: bool) -> PathBuf {
    if private {
        PathBuf::from("reports/evaluations/private/agent_search")
    } else {
        PathBuf::from("reports/evaluations/agent_search")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_is_disabled_by_default() {
        let args = Args::try_parse_from(["agent_search_eval"]).expect("parse default args");
        assert!(!args.use_judge);
    }

    #[test]
    fn use_judge_enables_judge_scoring() {
        let args = Args::try_parse_from(["agent_search_eval", "--use-judge"]).expect("parse use-judge args");
        assert!(args.use_judge);
    }
}
