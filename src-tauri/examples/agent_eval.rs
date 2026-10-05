// `agent_eval` — measures the email agent's decisions: which rules and panels
// an email or event matches, and what the matched rules do.
//
// Usage:
//   cargo run --features eval --example agent_eval -- [flags]
//   make eval-agent
//   make eval-agent ARGS="--case support_request_es"
//
// Flags:
//   --case <id>        Run only this case.
//   --model <name>     The model (default: the one the app ships with).
//   --provider <name>  The provider (default: llamacpp).
//   --out <dir>        Report output directory.
//   --cases <path>     The cases YAML.
//
// Synthetic cases on an in-memory database: no mailbox is read. GGUFs come
// from EMAILOPS_DATA_DIR (else the app's data dir).

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::agent::runner::{run, AgentRunnerConfig};

#[derive(Parser, Debug)]
#[command(name = "agent_eval", about = "Score the email agent's match and action decisions.")]
struct Args {
    #[arg(long)]
    case: Option<String>,

    #[arg(long, default_value = "qwen3.5-4b-q4_k_m")]
    model: String,

    #[arg(long, default_value = "llamacpp")]
    provider: String,

    #[arg(long)]
    out: Option<PathBuf>,

    #[arg(long)]
    cases: Option<PathBuf>,
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }
    let args = Args::parse();
    let cfg = AgentRunnerConfig {
        only_case: args.case,
        provider: args.provider,
        model: args.model,
        out_dir: args.out.unwrap_or_else(|| PathBuf::from("reports/evaluations/agent")),
        cases_path: args.cases.unwrap_or_else(default_cases),
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let code = match rt.block_on(run(cfg)) {
        Ok((passed, total)) => i32::from(passed != total),
        Err(e) => {
            eprintln!("[agent-eval] ERROR: {e}");
            1
        }
    };
    // Leaving normally lets ggml's Metal static destructor abort.
    emailops_lib::services::ai::shutdown_and_exit(code);
}

fn default_cases() -> PathBuf {
    if PathBuf::from("src-tauri/evals/agent/cases.yaml").exists() {
        PathBuf::from("src-tauri/evals/agent/cases.yaml")
    } else {
        PathBuf::from("evals/agent/cases.yaml")
    }
}
