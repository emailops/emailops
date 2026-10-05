// `draft_case_eval` — synthetic eval of the AI draft generator against the
// cases in `src-tauri/evals/drafts/cases.yaml` (no private data, no judge).
// Seeds an in-memory DB per case; never opens the user's mailbox.
//
// Usage:
//   cargo run --features eval --example draft_case_eval -- --repeat 3
//   cargo run --features eval --example draft_case_eval -- --case signature --model qwen3.5-9b-q4_k_m

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::draft_cases::{run, DraftCaseEvalConfig};

#[derive(Parser, Debug)]
#[command(name = "draft_case_eval", about = "Synthetic eval of AI draft endings.")]
struct Args {
    #[arg(long, default_value = "qwen3.5-4b-q4_k_m")]
    model: String,

    #[arg(long, default_value = "llamacpp")]
    provider: String,

    /// Run only the cases whose id contains this.
    #[arg(long)]
    case: Option<String>,

    /// Drafts generated per case.
    #[arg(long, default_value_t = 1)]
    repeat: usize,

    #[arg(long, default_value = "evals/drafts/cases.yaml")]
    cases: PathBuf,

    #[arg(long, default_value = "reports/evaluations/draft_cases")]
    out: PathBuf,
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }

    let args = Args::parse();
    let cfg = DraftCaseEvalConfig {
        model: args.model,
        provider_name: args.provider,
        cases_path: args.cases,
        out_dir: args.out,
        case_filter: args.case,
        repeat: args.repeat,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(run(cfg)) {
        Ok(path) => eprintln!("[draft-case-eval] done → {}", path.display()),
        Err(e) => {
            eprintln!("[draft-case-eval] ERROR: {}", e);
            std::process::exit(1);
        }
    }
}
