// `translation_eval` — evaluates AI language detection + translation against
// the synthetic cases in `src-tauri/evals/translation/cases.yaml`.
//
// Fully synthetic (no personal-mailbox content) and heuristic (exact ISO-code
// match for detection, keyword presence/absence for translation) — no judge.
//
// Usage:
//   cargo run --features eval --example translation_eval
//   cargo run --features eval --example translation_eval -- --case detect_es_invoice
//   cargo run --features eval --example translation_eval -- --model qwen3.5-4b-q4_k_m --provider llamacpp

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::translation::{run, TranslationEvalConfig};

#[derive(Parser, Debug)]
#[command(name = "translation_eval", about = "Evaluate AI language detection + translation.")]
struct Args {
    #[arg(long, default_value = "qwen3.5-4b-q4_k_m")]
    model: String,

    #[arg(long, default_value = "llamacpp")]
    provider: String,

    /// Run only the case with this id.
    #[arg(long)]
    case: Option<String>,

    #[arg(long, default_value = "evals/translation/cases.yaml")]
    cases: PathBuf,

    #[arg(long, default_value = "reports/evaluations/translation")]
    out: PathBuf,
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }

    let args = Args::parse();
    let cfg = TranslationEvalConfig {
        model: args.model,
        provider_name: args.provider,
        cases_path: args.cases,
        out_dir: args.out,
        case_filter: args.case,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(run(cfg)) {
        Ok(path) => eprintln!("[translation-eval] done → {}", path.display()),
        Err(e) => {
            eprintln!("[translation-eval] ERROR: {}", e);
            std::process::exit(1);
        }
    }
}
