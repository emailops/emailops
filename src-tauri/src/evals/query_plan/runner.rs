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
use crate::evals::shared::percentile;
use crate::evals::{EvalError, EvalResult};
use crate::services::chat::planner::{plan_search, Plan, PlanOutcome, SearchPlan};
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
    /// Print the machine-readable summary to stdout instead of prose.
    pub json_stdout: bool,
}

/// What one case produced, kept for the report.
pub struct CaseRun {
    pub case: PlanCase,
    pub plan: Option<SearchPlan>,
    pub report: PlanReport,
    pub latency_ms: u128,
    /// Why the planner did not search — `Unparseable` is a decoding failure,
    /// `Deferred` is the planner doing its job, and the two used to be
    /// indistinguishable here.
    pub outcome: PlanOutcome,
    pub prompt_tokens: u32,
    pub prefill_ms: Option<i64>,
    pub cached_prompt_tokens: Option<u32>,
}

/// The run as a whole, for `--json` and for the before/after report.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanMetricsReport {
    pub model: String,
    pub total_cases: usize,
    pub passed: usize,
    pub searched: usize,
    pub deferred: usize,
    pub empty_filter: usize,
    pub unparseable: usize,
    pub provider_errors: usize,
    pub latency_ms_mean: Option<f64>,
    pub latency_ms_p50: Option<u64>,
    pub latency_ms_p95: Option<u64>,
    pub prompt_tokens_mean: Option<f64>,
    pub prefill_ms_mean: Option<f64>,
}

impl PlanMetricsReport {
    fn build(model: &str, runs: &[CaseRun]) -> Self {
        let count = |want: PlanOutcome| runs.iter().filter(|r| r.outcome == want).count();
        let mut latencies: Vec<u64> = runs.iter().map(|r| r.latency_ms as u64).collect();
        latencies.sort_unstable();
        let mean = |values: Vec<f64>| (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64);
        Self {
            model: model.to_string(),
            total_cases: runs.len(),
            passed: runs.iter().filter(|r| r.report.passed).count(),
            searched: count(PlanOutcome::Search),
            deferred: count(PlanOutcome::Deferred),
            empty_filter: count(PlanOutcome::EmptyFilter),
            unparseable: count(PlanOutcome::Unparseable),
            provider_errors: count(PlanOutcome::ProviderError),
            latency_ms_mean: mean(latencies.iter().map(|v| *v as f64).collect()),
            latency_ms_p50: percentile(&latencies, 0.5),
            latency_ms_p95: percentile(&latencies, 0.95),
            prompt_tokens_mean: mean(runs.iter().map(|r| r.prompt_tokens as f64).collect()),
            prefill_ms_mean: mean(runs.iter().filter_map(|r| r.prefill_ms).map(|v| v as f64).collect()),
        }
    }
}

pub async fn run(cfg: PlanRunnerConfig) -> EvalResult<PlanEvalSummary> {
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
        let outcome = planned.outcome;
        let prompt_tokens = planned.prompt_tokens;
        let prefill_ms = planned.prefill_ms;
        let cached_prompt_tokens = planned.cached_prompt_tokens;
        let plan = match planned.plan {
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
            outcome,
            prompt_tokens,
            prefill_ms,
            cached_prompt_tokens,
        });
    }

    let metrics = PlanMetricsReport::build(&model, &runs);

    let cases: Vec<ReportCase> = runs
        .iter()
        .map(|r| ReportCase {
            case: &r.case,
            plan: r.plan.as_ref(),
            report: &r.report,
            latency_ms: r.latency_ms,
        })
        .collect();
    let html_path = render(&cfg.out_dir, &model, &cases)?;

    if cfg.json_stdout {
        println!("{}", serde_json::to_string_pretty(&metrics)?);
    } else {
        println!("[plan-eval] {}/{} cases passed", metrics.passed, metrics.total_cases);
        println!(
            "[plan-eval] search {} · defer {} · empty filter {} · unparseable {} · provider errors {}",
            metrics.searched, metrics.deferred, metrics.empty_filter, metrics.unparseable, metrics.provider_errors
        );
        println!(
            "[plan-eval] latency mean {} p50 {} p95 {} ms · prompt tokens {} · prefill {} ms",
            metrics
                .latency_ms_mean
                .map(|v| format!("{v:.0}"))
                .unwrap_or_else(|| "n/a".into()),
            metrics
                .latency_ms_p50
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".into()),
            metrics
                .latency_ms_p95
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".into()),
            metrics
                .prompt_tokens_mean
                .map(|v| format!("{v:.0}"))
                .unwrap_or_else(|| "n/a".into()),
            metrics
                .prefill_ms_mean
                .map(|v| format!("{v:.0}"))
                .unwrap_or_else(|| "n/a".into()),
        );
        println!("[plan-eval] report written to {}", html_path.display());
    }

    let metrics_path = cfg.out_dir.join("query_plan_metrics.json");
    std::fs::write(&metrics_path, serde_json::to_string_pretty(&metrics)?)?;

    Ok(PlanEvalSummary {
        metrics,
        html_path,
        metrics_path,
    })
}

pub struct PlanEvalSummary {
    pub metrics: PlanMetricsReport,
    pub html_path: PathBuf,
    pub metrics_path: PathBuf,
}
