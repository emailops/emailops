// `research_mode_eval` — checks the research answer-form classifier on its own.
//
// Research answers a list or a count in code and writes a report only for
// questions that need one; this measures that decision with one short
// completion per case.
//
// Usage:
//   cargo run --features eval --example research_mode_eval -- [flags]
//   make eval-research-mode
//
// Flags:
//   --case <id>        Run only this case.
//   --model <name>     Override the model the classifier runs on.
//   --prod-db <path>   SQLite DB to copy (prompt overrides + model config).
//   --cases-dir <path> Directory with `*.yaml` cases.
//   --json             Print the summary JSON to stdout instead of prose.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::research_mode::{run, ModeRunnerConfig};

#[derive(Parser, Debug)]
#[command(name = "research_mode_eval", about = "Score the research answer-form classifier.", long_about = None)]
struct Args {
    #[arg(long)]
    case: Option<String>,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    prod_db: Option<PathBuf>,

    #[arg(long)]
    cases_dir: Option<PathBuf>,

    #[arg(long, default_value_t = false)]
    json: bool,
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }
    let args = Args::parse();
    let cfg = ModeRunnerConfig {
        only_case: args.case,
        model_override: args.model,
        cases_dir: args.cases_dir.unwrap_or_else(default_cases_dir),
        prod_db_path: args
            .prod_db
            .or_else(default_prod_db)
            .unwrap_or_else(|| PathBuf::from("emailops.db")),
        // Never the live DB.
        db_mode: EvalDbMode::CopyToTemp,
        json_stdout: args.json,
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let code = match rt.block_on(run(cfg)) {
        Ok(summary) if summary.passed == summary.total => 0,
        Ok(_) => 1,
        Err(e) => {
            eprintln!("[mode-eval] ERROR: {e}");
            1
        }
    };
    // The single exit path for anything that can load the embedded provider.
    emailops_lib::services::ai::shutdown_and_exit(code);
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
    if PathBuf::from("src-tauri/evals/chat/research_mode").exists() {
        PathBuf::from("src-tauri/evals/chat/research_mode")
    } else {
        PathBuf::from("evals/chat/research_mode")
    }
}
