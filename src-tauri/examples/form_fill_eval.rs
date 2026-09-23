// `form_fill_eval` — measures how well the model fills the app's forms.
//
// "crea una lens de facturas con importe, fecha y proveedor" becomes a form the
// user reviews and saves. Every field on it comes from one completion, so it is
// measured directly — one small call per case — instead of being inferred from
// a chat answer.
//
// Usage:
//   cargo run --features eval --example form_fill_eval -- [flags]
//   make eval-forms
//   make eval-forms ARGS="--case lens_invoices_es"
//
// Flags:
//   --case <id>        Run only this case.
//   --model <name>     Override the model the filler runs on.
//   --out <dir>        Report output directory.
//   --prod-db <path>   SQLite DB to copy (prompt overrides).
//   --cases-dir <path> Directory with `*.yaml` form cases.
//   --json             Print the metrics JSON to stdout instead of prose.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::forms::runner::{run, FormRunnerConfig};

#[derive(Parser, Debug)]
#[command(
    name = "form_fill_eval",
    about = "Score how the model fills the app's forms, field by field.",
    long_about = None,
)]
struct Args {
    #[arg(long)]
    case: Option<String>,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    out: Option<PathBuf>,

    #[arg(long)]
    prod_db: Option<PathBuf>,

    #[arg(long)]
    cases_dir: Option<PathBuf>,

    /// Print the metrics JSON to stdout instead of prose.
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
    let json = args.json;
    let cfg = FormRunnerConfig {
        only_case: args.case,
        model_override: args.model,
        out_dir: args.out.unwrap_or_else(|| PathBuf::from("reports/evaluations/forms")),
        cases_dir: args.cases_dir.unwrap_or_else(default_cases_dir),
        prod_db_path: args
            .prod_db
            .or_else(default_prod_db)
            .unwrap_or_else(|| PathBuf::from("emailops.db")),
        // Never the live DB: the harness only reads prompt overrides, but it
        // has no business holding the app's database open.
        db_mode: EvalDbMode::CopyToTemp,
        json_stdout: json,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let code = match rt.block_on(run(cfg)) {
        // A case that failed its checks is a failing run, so CI and an agent
        // can branch on the exit code without parsing the report.
        Ok(summary) => {
            if !json {
                eprintln!("[form-eval] done");
            }
            i32::from(summary.passed != summary.total_cases)
        }
        Err(e) => {
            eprintln!("[form-eval] ERROR: {e}");
            1
        }
    };

    // The single exit path for anything that can load the embedded provider:
    // leaving normally lets ggml's Metal static destructor abort and turns a
    // finished run into a failing exit code.
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
    if PathBuf::from("src-tauri/evals/forms").exists() {
        PathBuf::from("src-tauri/evals/forms")
    } else {
        PathBuf::from("evals/forms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_without_flags() {
        let args = Args::try_parse_from(["form_fill_eval"]).expect("parse default args");
        assert!(args.case.is_none());
        assert!(args.model.is_none());
        assert!(!args.json);
    }

    #[test]
    fn a_single_case_can_be_selected() {
        let args = Args::try_parse_from(["form_fill_eval", "--case", "lens_invoices_es"]).expect("parse case arg");
        assert_eq!(args.case.as_deref(), Some("lens_invoices_es"));
    }
}
