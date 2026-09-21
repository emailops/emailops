// `tag_classification_eval` — measures the email classifier on its own.
//
// `services::classification` tags every synced email with an intent, a topic
// and an urgency, and the sidebar filters, the chat query planner and the
// priority ordering all read those tags. This harness runs the real
// classifier over a synthetic labelled corpus with the rule engine left out,
// so the score is the model's.
//
// Not to be confused with `email_classification_eval`, which probes a
// different model on a different 10-way taxonomy and has no ground truth.
//
// Usage:
//   cargo run --features eval --example tag_classification_eval -- [flags]
//   make eval-classify ARGS="--json"
//
// Flags:
//   --case <id>         Run only this case.
//   --lang <en|es>      Run only cases in this language.
//   --mode <json>       How the classifier is asked for its answer.
//   --repeats <n>       Passes per case; labels come from the first.
//   --model <name>      Override the model.
//   --json              Print the metrics JSON to stdout instead of prose.
//   --out <dir>         Report output directory.
//   --prod-db <path>    SQLite DB to copy (provider, model, prompt override).
//   --cases-dir <path>  Directory with `*.yaml` classification cases.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::tag_classification::runner::{run, DecodeMode, TagRunnerConfig};

#[derive(Parser, Debug)]
#[command(
    name = "tag_classification_eval",
    about = "Score intent / topic / urgency tagging against a synthetic labelled corpus.",
    long_about = None,
)]
struct Args {
    #[arg(long)]
    case: Option<String>,

    #[arg(long)]
    lang: Option<String>,

    #[arg(long, default_value = "json")]
    mode: String,

    #[arg(long, default_value_t = 1)]
    repeats: usize,

    #[arg(long)]
    model: Option<String>,

    #[arg(long, default_value_t = false)]
    json: bool,

    #[arg(long)]
    out: Option<PathBuf>,

    #[arg(long)]
    prod_db: Option<PathBuf>,

    #[arg(long)]
    cases_dir: Option<PathBuf>,
}

fn parse_mode(raw: &str) -> Result<DecodeMode, String> {
    match raw {
        "json" => Ok(DecodeMode::Json),
        other => Err(format!("unknown --mode `{other}` (expected: json)")),
    }
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }

    let args = Args::parse();
    let mode = match parse_mode(&args.mode) {
        Ok(mode) => mode,
        Err(e) => {
            eprintln!("[tag-eval] ERROR: {e}");
            std::process::exit(2);
        }
    };

    let cfg = TagRunnerConfig {
        only_case: args.case,
        only_lang: args.lang,
        model_override: args.model,
        out_dir: args
            .out
            .unwrap_or_else(|| PathBuf::from("reports/evaluations/classification")),
        cases_dir: args.cases_dir.unwrap_or_else(default_cases_dir),
        prod_db_path: args
            .prod_db
            .or_else(default_prod_db)
            .unwrap_or_else(|| PathBuf::from("emailops.db")),
        // The corpus is synthetic; the DB is only read for provider, model
        // and prompt override, and never the live one.
        db_mode: EvalDbMode::CopyToTemp,
        mode,
        repeats: args.repeats,
        json_stdout: args.json,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let code = match rt.block_on(run(cfg)) {
        Ok(summary) => {
            eprintln!("[tag-eval] done → {}", summary.html_path.display());
            0
        }
        Err(e) => {
            eprintln!("[tag-eval] ERROR: {e}");
            1
        }
    };

    // The single exit path for anything that can load the embedded provider:
    // leaving normally lets ggml's Metal static destructor abort and turns a
    // finished run into a failing exit code. The report is on disk by here.
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
    if PathBuf::from("src-tauri/evals/classification/cases").exists() {
        PathBuf::from("src-tauri/evals/classification/cases")
    } else {
        PathBuf::from("evals/classification/cases")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_without_flags() {
        let args = Args::try_parse_from(["tag_classification_eval"]).expect("parse default args");
        assert_eq!(args.mode, "json");
        assert_eq!(args.repeats, 1);
        assert!(!args.json);
    }

    #[test]
    fn json_mode_is_the_shipped_decode_path() {
        assert_eq!(parse_mode("json"), Ok(DecodeMode::Json));
    }

    #[test]
    fn an_unknown_mode_is_rejected() {
        assert!(parse_mode("telepathy").is_err());
    }
}
