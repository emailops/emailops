// `lens_template_eval` — checks what a built-in Lens template extracts.
//
// Each case is a synthetic email plus the row a person reading it would write
// down. The harness inserts the email into a throwaway copy of the DB, checks
// the template's own scope picks it up (or leaves it alone), runs the
// production extractor and compares the row field by field. Results use the
// shared eval report schema, so `make verify` shows them in its HTML report
// under "Lenses, tareas y adjuntos".
//
// Usage:
//   cargo run --features eval --example lens_template_eval -- [flags]
//   make eval-lenses
//   make eval-lenses ARGS="--case contact_form_cf7_en"
//
// Flags:
//   --case <id>        Run only this case.
//   --model <name>     Override the model the extractor runs on.
//   --out <dir>        Report output directory.
//   --prod-db <path>   SQLite DB to copy (model preferences, an account).
//   --cases-dir <path> Directory with `*.yaml` lens cases.

use std::path::PathBuf;

use clap::Parser;

use emailops_lib::evals::db_source::EvalDbMode;
use emailops_lib::evals::lenses::runner::{run, LensRunnerConfig};

#[derive(Parser, Debug)]
#[command(
    name = "lens_template_eval",
    about = "Check what built-in Lens templates extract, field by field.",
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
}

fn main() {
    for p in [".env.local", ".env", "../.env.local", "../.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }

    let args = Args::parse();
    let cfg = LensRunnerConfig {
        only_case: args.case,
        model_override: args.model,
        out_dir: args.out.unwrap_or_else(|| PathBuf::from("reports/evaluations/lenses")),
        cases_dir: args.cases_dir.unwrap_or_else(default_cases_dir),
        prod_db_path: args
            .prod_db
            .or_else(default_prod_db)
            .unwrap_or_else(|| PathBuf::from("emailops.db")),
        // The harness inserts its synthetic emails: never into the live DB.
        db_mode: EvalDbMode::CopyToTemp,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let code = match rt.block_on(run(cfg)) {
        // A failing case is a failing run, so scripts can branch on the code.
        Ok(report) => i32::from(report.failed > 0),
        Err(e) => {
            eprintln!("[lens-eval] ERROR: {e}");
            1
        }
    };

    // Leaving normally lets ggml's Metal static destructor abort (exit 134).
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
    if PathBuf::from("src-tauri/evals/lenses").exists() {
        PathBuf::from("src-tauri/evals/lenses")
    } else {
        PathBuf::from("evals/lenses")
    }
}
