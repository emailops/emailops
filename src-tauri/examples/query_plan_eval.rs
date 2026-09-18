// `query_plan_eval` — measures the chat query planner on its own.
//
// The planner turns one question into a single `search_emails` filter, and the
// rest of the turn inherits whatever it gets wrong. Running it directly costs
// one small completion per case instead of a full chat turn, so a prompt or
// model change can be checked in seconds.
//
// Usage:
//   cargo run --features eval --example query_plan_eval -- [flags]
//   make eval-plan
//
// Flags:
//   --case <id>        Run only this case.
//   --model <name>     Override the model the planner runs on.
//   --account <email>  The address the prompt resolves "me" to.
//   --out <dir>        Report output directory.
//   --prod-db <path>   SQLite DB to copy (prompt overrides + tag glossary).
//   --cases-dir <path> Directory with `*.yaml` planner cases.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::query_plan::runner::{run, PlanRunnerConfig};

#[derive(Parser, Debug)]
#[command(
    name = "query_plan_eval",
    about = "Score the chat query planner field by field, without running chat turns.",
    long_about = None,
)]
struct Args {
    #[arg(long)]
    case: Option<String>,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    account: Option<String>,

    #[arg(long)]
    out: Option<PathBuf>,

    #[arg(long)]
    prod_db: Option<PathBuf>,

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
    let cfg = PlanRunnerConfig {
        only_case: args.case,
        model_override: args.model,
        account: args.account,
        out_dir: args
            .out
            .unwrap_or_else(|| PathBuf::from("reports/evaluations/query_plan")),
        cases_dir: args.cases_dir.unwrap_or_else(default_cases_dir),
        prod_db_path: args
            .prod_db
            .or_else(default_prod_db)
            .unwrap_or_else(|| PathBuf::from("emailops.db")),
        // Never the live DB: the planner only reads prompts and tags, but the
        // harness has no business holding the app's database open.
        db_mode: EvalDbMode::CopyToTemp,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(run(cfg)) {
        Ok(path) => eprintln!("[plan-eval] done → {}", path.display()),
        Err(e) => {
            eprintln!("[plan-eval] ERROR: {e}");
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

fn default_cases_dir() -> PathBuf {
    if PathBuf::from("src-tauri/evals/chat/query_plan").exists() {
        PathBuf::from("src-tauri/evals/chat/query_plan")
    } else {
        PathBuf::from("evals/chat/query_plan")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_without_flags() {
        let args = Args::try_parse_from(["query_plan_eval"]).expect("parse default args");
        assert!(args.case.is_none());
        assert!(args.model.is_none());
    }

    #[test]
    fn a_single_case_can_be_selected() {
        let args = Args::try_parse_from(["query_plan_eval", "--case", "no_date_window"]).expect("parse case arg");
        assert_eq!(args.case.as_deref(), Some("no_date_window"));
    }
}
