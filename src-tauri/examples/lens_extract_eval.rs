// `lens_extract_eval` binary — evaluation harness for the Lenses extractor.
//
// Picks a Lens by id, samples N matching emails via the Lens's own scope
// filter, runs `services::lenses::extractor::extract_email` on each, and
// writes a JSON report with per-email extraction outputs. No LLM judge —
// this is a skeleton meant to be extended.
//
// Usage:
//   cargo run --features eval --bin lens_extract_eval -- \
//     --lens-id <id> --limit 20
//
//   # Run against specific email IDs (bypasses scope evaluation):
//   cargo run --features eval --bin lens_extract_eval -- \
//     --lens-id <id> --email-ids 19e3f5ffc919cc9c,19d4e767bb03dbb6,19d7c385a8b2cfb9
//
// Flags:
//   --lens-id <id>           Required. Existing Lens id in the prod DB.
//   --limit <N>              Number of emails to sample from scope. Default: 20.
//   --email-ids <id,...>     Comma-separated email IDs. Bypasses scope; runs
//                            extraction on exactly these emails.
//   --out <dir>              Report dir. Default: reports/evaluations/lenses.
//   --prod-db <path>         Prod SQLite DB. Default: platform app-data dir.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use serde::Serialize;

use emailops_lib::db::Database;
use emailops_lib::evals::db_source::{prepare_eval_db, EvalDbMode};
use emailops_lib::services::ai::AiService;
use emailops_lib::services::lenses::{extractor, scope as scope_eval};

#[derive(Parser, Debug)]
#[command(name = "lens_extract_eval", about = "Evaluate per-email Lens extraction.")]
struct Args {
    #[arg(long = "lens-id")]
    lens_id: String,

    #[arg(long, default_value_t = 20)]
    limit: usize,

    /// Comma-separated email IDs to run against. When set, bypasses scope
    /// evaluation and runs extraction on exactly these emails.
    #[arg(long = "email-ids")]
    email_ids: Option<String>,

    #[arg(long)]
    out: Option<PathBuf>,

    #[arg(long)]
    prod_db: Option<PathBuf>,

    /// Open the production DB in place instead of copying it to a temp DB.
    #[arg(long, hide = true)]
    in_place_dangerous: bool,
}

#[derive(Debug, Serialize)]
struct EvalReport {
    lens_id: String,
    lens_name: String,
    total: usize,
    succeeded: usize,
    failed: usize,
    elapsed_ms: u128,
    cases: Vec<EvalCase>,
}

#[derive(Debug, Serialize)]
struct EvalCase {
    email_id: String,
    subject: String,
    sender: String,
    status: String,
    error: Option<String>,
    data: serde_json::Value,
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
    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("reports/evaluations/lenses"));

    let pinned_ids: Vec<String> = args
        .email_ids
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let db_mode = if args.in_place_dangerous {
        EvalDbMode::InPlaceDangerous
    } else {
        EvalDbMode::CopyToTemp
    };

    match rt.block_on(run(args.lens_id, args.limit, pinned_ids, prod_db, db_mode, out_dir)) {
        Ok(path) => eprintln!("[lens-extract-eval] done → {}", path.display()),
        Err(e) => {
            eprintln!("[lens-extract-eval] ERROR: {}", e);
            std::process::exit(1);
        }
    }
}

async fn run(
    lens_id: String,
    limit: usize,
    pinned_ids: Vec<String>,
    prod_db: PathBuf,
    db_mode: EvalDbMode,
    out_dir: PathBuf,
) -> Result<PathBuf, String> {
    let prepared_db = prepare_eval_db(&prod_db, db_mode, "lens-extract").map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(prepared_db.db_dir().to_path_buf()).map_err(|e| e.to_string())?);

    let lens = db.get_lens(&lens_id).map_err(|e| e.to_string())?;
    eprintln!(
        "[lens-extract-eval] lens = {} ({}), columns = {}",
        lens.name,
        lens.id,
        lens.schema.columns.len()
    );

    // Either use the caller-supplied email IDs or sample from scope.
    let picks: Vec<String> = if !pinned_ids.is_empty() {
        eprintln!("[lens-extract-eval] using {} pinned email id(s)", pinned_ids.len());
        pinned_ids
    } else {
        // Sample matching emails. We over-fetch and take the first N to mirror
        // the dry-run flow in `preview_lens_extraction`.
        let pool = scope_eval::evaluate_with_limit(&db, &lens.scope, (limit as i64) * 5).map_err(|e| e.to_string())?;
        let sampled: Vec<String> = pool.into_iter().take(limit).collect();
        eprintln!("[lens-extract-eval] sampled {} emails from scope", sampled.len());
        if sampled.is_empty() {
            return Err("scope matched zero emails".into());
        }
        sampled
    };

    let provider = AiService::load_provider(&db).map_err(|e| e.to_string())?;

    let started = Instant::now();
    let mut cases = Vec::with_capacity(picks.len());
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for (i, email_id) in picks.iter().enumerate() {
        eprintln!("[lens-extract-eval] {}/{}  {}", i + 1, picks.len(), email_id);
        let email = db
            .get_email_by_id(email_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("email {email_id} disappeared mid-eval"))?;
        match extractor::extract_email(&db, provider.clone(), &lens, email_id, None).await {
            Ok(res) => {
                let status = res.status.as_str().to_string();
                if status == "ok" {
                    succeeded += 1;
                } else {
                    failed += 1;
                }
                cases.push(EvalCase {
                    email_id: email_id.clone(),
                    subject: email.subject.clone(),
                    sender: email.sender.clone(),
                    status,
                    error: res.error_message,
                    data: res.data,
                });
            }
            Err(e) => {
                failed += 1;
                cases.push(EvalCase {
                    email_id: email_id.clone(),
                    subject: email.subject.clone(),
                    sender: email.sender.clone(),
                    status: "error".into(),
                    error: Some(e.to_string()),
                    data: serde_json::Value::Null,
                });
            }
        }
    }

    let report = EvalReport {
        lens_id: lens.id.clone(),
        lens_name: lens.name.clone(),
        total: picks.len(),
        succeeded,
        failed,
        elapsed_ms: started.elapsed().as_millis(),
        cases,
    };

    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let out_path = out_dir.join(format!("lens_{}_{}.json", sanitize(&lens.id), ts));
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(&out_path, json).map_err(|e| e.to_string())?;
    eprintln!(
        "[lens-extract-eval] succeeded={} failed={} elapsed={}ms",
        report.succeeded, report.failed, report.elapsed_ms
    );
    Ok(out_path)
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
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
