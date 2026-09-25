// Lens extraction runner.
//
// For each case: build the Lens a user gets from the named built-in template,
// insert the synthetic email through the same path sync uses (so the search
// index sees exactly what production indexes), check the template's scope
// picks it up, run the production extractor, and score the row field by field.
//
// Works on a throwaway copy of the source DB: the inserted emails never reach
// it.

use std::path::PathBuf;
use std::time::Instant;

use crate::db::Database;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::json_report::{EvidenceCheck, ItemEvidence, ItemResult, JsonRunReport};
use crate::evals::lenses::case_loader::{load_lens_cases, LensCase};
use crate::evals::lenses::metrics::{evaluate, field_table, LensReport};
use crate::evals::{EvalError, EvalResult};
use crate::models::lens::Lens;
use crate::models::Email;
use crate::services::lenses::extractor::{extract_email, ExtractionStatus};
use crate::services::lenses::{scope, templates};

#[derive(Debug, Clone)]
pub struct LensRunnerConfig {
    pub only_case: Option<String>,
    pub model_override: Option<String>,
    pub out_dir: PathBuf,
    pub cases_dir: PathBuf,
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
}

/// The Lens a user gets from a built-in template, before they touch the form.
pub fn lens_from_template(key: &str) -> EvalResult<Lens> {
    let tpl = templates::get(key).ok_or_else(|| EvalError::Config(format!("no built-in template {key:?}")))?;
    Ok(Lens {
        id: format!("template:{key}"),
        name: tpl.name,
        icon: Some(tpl.icon),
        template_key: Some(tpl.key),
        account_id: None,
        scope: tpl.default_scope,
        schema: tpl.schema,
        prompt_text: tpl.prompt,
        prompt_version: 1,
        model_provider: None,
        model_name: None,
        is_enabled: true,
        sort_order: 0,
        created_at: 0,
        updated_at: 0,
    })
}

fn case_email(case: &LensCase, account_id: &str) -> Email {
    let id = format!("eval-lens-{}", case.id);
    Email {
        id: id.clone(),
        account_id: account_id.to_string(),
        thread_id: id,
        message_id: None,
        references: None,
        subject: case.email.subject.clone(),
        sender: case.email.from_name.clone(),
        sender_email: case.email.from_email.clone(),
        recipients: Vec::new(),
        cc: Vec::new(),
        body: case.email.body.clone(),
        snippet: String::new(),
        timestamp: chrono::Utc::now().timestamp(),
        is_read: false,
        triage_status: None,
        category: "primary".into(),
        mailbox: "inbox".into(),
        is_sent: false,
        headers: None,
    }
}

/// The email as the extractor receives it, for the report's "question" block.
fn email_text(case: &LensCase) -> String {
    format!(
        "De: {} <{}>\nAsunto: {}\n\n{}",
        case.email.from_name,
        case.email.from_email,
        case.email.subject,
        case.email.body.trim_end()
    )
}

/// One `column: value` line per extracted column, for the "answer" block.
fn row_text(lens: &Lens, row: Option<&serde_json::Value>, error: Option<&str>) -> String {
    let Some(row) = row else {
        return format!(
            "(extracción fallida){}",
            error.map(|e| format!(": {e}")).unwrap_or_default()
        );
    };
    lens.schema
        .columns
        .iter()
        .map(|c| match row.get(&c.key) {
            None | Some(serde_json::Value::Null) => format!("{}: ∅", c.key),
            Some(serde_json::Value::String(s)) => format!("{}: {s}", c.key),
            Some(other) => format!("{}: {other}", c.key),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `key=value` pairs of the extracted row, for the report.
fn row_summary(row: &serde_json::Value) -> String {
    row.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| match v {
                    serde_json::Value::Null => format!("{k}=∅"),
                    serde_json::Value::String(s) => format!("{k}={s}"),
                    other => format!("{k}={other}"),
                })
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default()
}

fn item_detail(case: &LensCase, report: &LensReport, row: Option<&serde_json::Value>, error: Option<&str>) -> String {
    let failed: Vec<String> = report
        .checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| format!("{}: esperado {}, obtenido {}", c.field, c.expected, c.actual))
        .collect();
    let mut parts = Vec::new();
    if !failed.is_empty() {
        parts.push(failed.join(" · "));
    } else if !case.note.is_empty() {
        parts.push(case.note.clone());
    }
    match (row, error) {
        (Some(r), _) => parts.push(format!("fila: {}", row_summary(r))),
        (None, Some(e)) => parts.push(format!("error: {e}")),
        (None, None) => {}
    }
    parts.join(" — ")
}

pub async fn run(cfg: LensRunnerConfig) -> EvalResult<JsonRunReport> {
    let mut cases = load_lens_cases(&cfg.cases_dir)?;
    if let Some(id) = &cfg.only_case {
        cases.retain(|c| &c.id == id);
        if cases.is_empty() {
            return Err(EvalError::Config(format!("no lens case with id `{id}`")));
        }
    }

    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "lenses")?;
    let db = std::sync::Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);
    crate::evals::shared::apply_eval_model_override_from_env(&db)?;
    if let Some(model) = &cfg.model_override {
        db.set_preference("ai_model", model)?;
    }
    let account_id = db
        .list_accounts()?
        .into_iter()
        .next()
        .map(|a| a.id)
        .ok_or_else(|| EvalError::Config("the source DB has no account to attach cases to".into()))?;

    let provider = crate::services::ai::AiService::load_provider(&db)
        .map_err(|e| EvalError::Config(format!("no AI provider available: {e}")))?;
    let model = db.get_preference("ai_model")?.unwrap_or_default();
    println!("[lens-eval] model = {model}");
    println!("[lens-eval] running {} case(s)", cases.len());

    let mut report = JsonRunReport::new("lens_template_eval", &model);
    for case in &cases {
        let mut lens = lens_from_template(&case.template)?;
        if let Some(own) = &case.lens {
            lens.schema.columns.clone_from(&own.columns);
            lens.prompt_text.clone_from(&own.prompt);
        }
        let email = case_email(case, &account_id);
        db.insert_email(&email)?;

        let in_scope = scope::evaluate(&db, &lens.scope)?.contains(&email.id);
        let started = Instant::now();
        let result = extract_email(&db, provider.clone(), &lens, &email.id, None).await?;
        let latency_ms = started.elapsed().as_millis();
        let row = (result.status == ExtractionStatus::Ok).then_some(&result.data);
        let scored = evaluate(case, row, in_scope);

        println!(
            "[lens-eval] {} {} ({}/{} checks, {latency_ms}ms)",
            if scored.passed { "OK  " } else { "FAIL" },
            case.id,
            scored.checks.iter().filter(|c| c.ok).count(),
            scored.checks.len(),
        );
        for check in scored.checks.iter().filter(|c| !c.ok) {
            println!(
                "[lens-eval]      {}: wanted {}, got {}",
                check.field, check.expected, check.actual
            );
        }

        let total = scored.checks.len().max(1) as f64;
        let ok = scored.checks.iter().filter(|c| c.ok).count() as f64;
        report.push(ItemResult {
            id: case.id.clone(),
            passed: scored.passed,
            score: Some(ok / total),
            detail: item_detail(case, &scored, row, result.error_message.as_deref()),
            evidence: Some(ItemEvidence {
                input: email_text(case),
                output: row_text(&lens, row, result.error_message.as_deref()),
                checks: field_table(&scored, &lens.schema, row)
                    .into_iter()
                    .map(|c| EvidenceCheck {
                        name: c.field,
                        expected: c.expected,
                        actual: c.actual,
                        passed: c.ok,
                    })
                    .collect(),
            }),
        });
    }
    report.write(&cfg.out_dir)?;
    println!("[lens-eval] {}/{} cases passed", report.succeeded, report.total);
    Ok(report)
}
