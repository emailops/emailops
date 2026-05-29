// `invoice_extract_eval` binary — evaluation harness for the built-in
// "Invoices received" Lens template against the production DB.
//
// Unlike `lens_extract_eval`, this does NOT require a pre-existing Lens row.
// It fabricates a transient Lens from the `invoices_received` template
// (services::lenses::templates::tpl_invoices_received), samples N matching
// emails via the template's default scope, runs the extractor on each, and
// writes a JSON report under `reports/evaluations/lenses/`.
//
// Usage:
//   cargo run --features eval --bin invoice_extract_eval -- --limit 20
//
// Flags:
//   --limit <N>        Number of emails to sample. Default: 20.
//   --last-days <N>    Override template's last_days window. Default: from template (365).
//   --account <email>  Restrict to a specific account by email address.
//   --out <dir>        Report dir. Default: reports/evaluations/lenses.
//   --prod-db <path>   Prod SQLite DB. Default: platform app-data dir.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use serde::Serialize;

use emailops_lib::db::Database;
use emailops_lib::evals::db_source::{prepare_eval_db, EvalDbMode};
use emailops_lib::models::lens::{DateRange, Lens};
use emailops_lib::services::ai::AiService;
use emailops_lib::services::lenses::{extractor, scope as scope_eval, templates};

const TEMPLATE_KEY: &str = "invoices_received";

#[derive(Parser, Debug)]
#[command(
    name = "invoice_extract_eval",
    about = "Evaluate the invoices_received Lens template against the prod DB."
)]
struct Args {
    #[arg(long, default_value_t = 20)]
    limit: usize,

    #[arg(long = "last-days")]
    last_days: Option<i64>,

    #[arg(long)]
    account: Option<String>,

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
    template_key: String,
    template_name: String,
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
    sender_email: String,
    timestamp: i64,
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
    let db_mode = if args.in_place_dangerous {
        EvalDbMode::InPlaceDangerous
    } else {
        EvalDbMode::CopyToTemp
    };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    match rt.block_on(run(args, prod_db, db_mode, out_dir)) {
        Ok(path) => eprintln!("[invoice-extract-eval] done → {}", path.display()),
        Err(e) => {
            eprintln!("[invoice-extract-eval] ERROR: {}", e);
            std::process::exit(1);
        }
    }
}

async fn run(args: Args, prod_db: PathBuf, db_mode: EvalDbMode, out_dir: PathBuf) -> Result<PathBuf, String> {
    let prepared_db = prepare_eval_db(&prod_db, db_mode, "invoice-extract").map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(prepared_db.db_dir().to_path_buf()).map_err(|e| e.to_string())?);

    // Pull the built-in template and fabricate a transient Lens. The extractor
    // never persists rows, so a synthetic id is fine — it only appears in our
    // own logs.
    let tpl = templates::get(TEMPLATE_KEY).ok_or_else(|| format!("template '{TEMPLATE_KEY}' not found in manifest"))?;

    let mut scope = tpl.default_scope.clone();
    if let Some(days) = args.last_days {
        scope.date_range = Some(DateRange {
            last_days: Some(days),
            from: None,
            to: None,
        });
    }
    if let Some(ref email) = args.account {
        let accounts = db.list_accounts().map_err(|e| e.to_string())?;
        let account = accounts
            .into_iter()
            .find(|a| a.email.eq_ignore_ascii_case(email))
            .ok_or_else(|| format!("no account with email '{email}'"))?;
        scope.account_ids = Some(vec![account.id]);
    }

    let now = chrono::Utc::now().timestamp();
    let lens = Lens {
        id: format!("eval-{TEMPLATE_KEY}-{now}"),
        name: tpl.name.clone(),
        icon: Some(tpl.icon.clone()),
        template_key: Some(tpl.key.clone()),
        account_id: None,
        scope: scope.clone(),
        schema: tpl.schema.clone(),
        prompt_text: tpl.prompt.clone(),
        prompt_version: 1,
        model_provider: None,
        model_name: None,
        is_enabled: true,
        sort_order: 0,
        created_at: now,
        updated_at: now,
    };

    eprintln!(
        "[invoice-extract-eval] template = {} ({}), columns = {}",
        lens.name,
        tpl.key,
        lens.schema.columns.len()
    );

    // Over-fetch then take N to mirror `preview_lens_extraction`.
    let pool = scope_eval::evaluate_with_limit(&db, &scope, (args.limit as i64) * 5).map_err(|e| e.to_string())?;
    let picks: Vec<String> = pool.into_iter().take(args.limit).collect();
    eprintln!(
        "[invoice-extract-eval] sampled {} emails (limit={})",
        picks.len(),
        args.limit
    );
    if picks.is_empty() {
        return Err("scope matched zero emails — try --last-days with a larger window".into());
    }

    let provider = AiService::load_provider(&db).map_err(|e| e.to_string())?;

    let started = Instant::now();
    let mut cases = Vec::with_capacity(picks.len());
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for (i, email_id) in picks.iter().enumerate() {
        eprintln!("[invoice-extract-eval] {}/{}  {}", i + 1, picks.len(), email_id);
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
                    sender_email: email.sender_email.clone(),
                    timestamp: email.timestamp,
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
                    sender_email: email.sender_email.clone(),
                    timestamp: email.timestamp,
                    status: "error".into(),
                    error: Some(e.to_string()),
                    data: serde_json::Value::Null,
                });
            }
        }
    }

    let report = EvalReport {
        template_key: tpl.key.clone(),
        template_name: tpl.name.clone(),
        total: picks.len(),
        succeeded,
        failed,
        elapsed_ms: started.elapsed().as_millis(),
        cases,
    };

    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let out_path = out_dir.join(format!("invoices_received_{}.json", ts));
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(&out_path, json).map_err(|e| e.to_string())?;
    eprintln!(
        "[invoice-extract-eval] succeeded={} failed={} elapsed={}ms",
        report.succeeded, report.failed, report.elapsed_ms
    );
    Ok(out_path)
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
