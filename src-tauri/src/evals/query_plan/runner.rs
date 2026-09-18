// Query-planner runner.
//
// For each case: render the live `chat.query_plan` prompt (same template, same
// tag glossary the app would use), ask the configured local model, and score
// the plan it returns field by field. No conversation, no tool loop, no judge —
// the planner is one completion, so a case costs a second or two.

use std::path::PathBuf;
use std::time::Instant;

use crate::db::Database;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::query_plan::case_loader::{load_plan_cases, PlanCase};
use crate::evals::query_plan::metrics::{evaluate, PlanReport};
use crate::evals::query_plan::report::{render, ReportCase};
use crate::evals::{EvalError, EvalResult};
use crate::services::chat::planner::{plan_search, Plan, SearchPlan};
use crate::services::classification::TagGlossary;

#[derive(Debug, Clone)]
pub struct PlanRunnerConfig {
    pub only_case: Option<String>,
    pub model_override: Option<String>,
    pub account: Option<String>,
    pub out_dir: PathBuf,
    pub cases_dir: PathBuf,
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
}

/// What one case produced, kept for the report.
pub struct CaseRun {
    pub case: PlanCase,
    pub plan: Option<SearchPlan>,
    pub report: PlanReport,
    pub latency_ms: u128,
}

pub async fn run(cfg: PlanRunnerConfig) -> EvalResult<PathBuf> {
    let mut cases = load_plan_cases(&cfg.cases_dir)?;
    if let Some(id) = &cfg.only_case {
        cases.retain(|c| &c.id == id);
        if cases.is_empty() {
            return Err(EvalError::Config(format!("no planner case with id `{id}`")));
        }
    }

    // Same isolation as the other harnesses: work on a copy, never the live DB.
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "query-plan")?;
    let db = std::sync::Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);
    crate::evals::shared::apply_eval_model_override_from_env(&db)?;
    if let Some(model) = &cfg.model_override {
        db.set_preference("ai_model", model)?;
    }

    // The planner runs against the same provider and prompt the app would use,
    // so a prompt edit shows up here without touching the harness.
    let provider = crate::services::ai::AiService::load_provider(&db)
        .map_err(|e| EvalError::Config(format!("no AI provider available: {e}")))?;
    let template = crate::services::prompts::get_template(&db, "chat.query_plan")
        .map_err(|e| EvalError::Config(format!("cannot load chat.query_plan: {e}")))?;
    let glossary = TagGlossary::load(&db);
    let model = db.get_preference("ai_model")?.unwrap_or_default();

    let user_email = match &cfg.account {
        Some(account) => account.clone(),
        None => db
            .list_accounts()?
            .into_iter()
            .find(|a| a.enabled)
            .map(|a| a.email)
            .unwrap_or_default(),
    };
    let default_today = chrono::Local::now().format("%Y-%m-%d").to_string();

    println!("[plan-eval] model = {model}");
    println!("[plan-eval] account = {user_email}");
    println!("[plan-eval] running {} case(s)", cases.len());

    let mut runs = Vec::new();
    for case in cases {
        let today = case.today.clone().unwrap_or_else(|| default_today.clone());
        let started = Instant::now();
        let planned = plan_search(
            provider.as_ref(),
            &template,
            &user_email,
            &today,
            &case.question,
            &glossary,
        )
        .await;
        let latency_ms = started.elapsed().as_millis();
        let plan = match planned {
            Plan::Search(plan) => Some(*plan),
            Plan::Defer => None,
        };
        let report = evaluate(&case, plan.as_ref());
        println!(
            "[plan-eval] {} {} ({}/{} checks, {latency_ms}ms)",
            if report.passed { "OK  " } else { "FAIL" },
            case.id,
            report.checks.iter().filter(|c| c.passed()).count(),
            report.checks.len(),
        );
        runs.push(CaseRun {
            case,
            plan,
            report,
            latency_ms,
        });
    }

    let passed = runs.iter().filter(|r| r.report.passed).count();
    println!("[plan-eval] {passed}/{} cases passed", runs.len());

    let cases: Vec<ReportCase> = runs
        .iter()
        .map(|r| ReportCase {
            case: &r.case,
            plan: r.plan.as_ref(),
            report: &r.report,
            latency_ms: r.latency_ms,
        })
        .collect();
    let path = render(&cfg.out_dir, &model, &cases)?;
    println!("[plan-eval] report written to {}", path.display());
    Ok(path)
}
