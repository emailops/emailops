// Form-filling runner.
//
// For each case: render the live `forms.fill` prompt against the registered
// form definition, ask the configured local model, parse the reply through the
// production parser, and score the result field by field. No conversation, no
// tool loop, no judge — filling a form is one completion, so a case costs a
// second or two.
//
// The harness deliberately calls the same `services::forms::filler::fill_form`
// production uses, so a prompt edit, a registry change or a parser fix shows up
// here without touching this file.

use std::path::PathBuf;
use std::time::Instant;

use crate::db::Database;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::forms::case_loader::{load_form_cases, FormCase};
use crate::evals::forms::metrics::{evaluate, FormReport};
use crate::evals::json_report::{ItemResult, JsonRunReport};
use crate::evals::shared::percentile;
use crate::evals::{EvalError, EvalResult};
use crate::services::forms::filler::fill_form;
use crate::services::forms::{registry, FormFill};

#[derive(Debug, Clone)]
pub struct FormRunnerConfig {
    pub only_case: Option<String>,
    pub model_override: Option<String>,
    pub out_dir: PathBuf,
    pub cases_dir: PathBuf,
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
    /// Print the machine-readable summary to stdout instead of prose.
    pub json_stdout: bool,
}

/// What one case produced.
pub struct CaseRun {
    pub case: FormCase,
    pub fill: Option<FormFill>,
    pub report: FormReport,
    pub latency_ms: u128,
    pub prompt_tokens: u32,
}

/// The run as a whole, for `--json` and for before/after comparisons.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormMetricsReport {
    pub model: String,
    pub total_cases: usize,
    pub passed: usize,
    /// Cases where the model produced nothing the parser could use.
    pub unparseable: usize,
    /// Total keys the model invented across the run. The parser drops them, so
    /// they never reach the UI — but a rising number means the prompt is
    /// drifting and is worth seeing.
    pub invented_keys: usize,
    /// Total required fields left unfilled across the run.
    pub missing_required: usize,
    pub latency_ms_p50: Option<u64>,
    pub latency_ms_p95: Option<u64>,
    pub prompt_tokens_mean: f64,
    pub cases: Vec<CaseSummary>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseSummary {
    pub id: String,
    pub passed: bool,
    pub latency_ms: u128,
    pub failed_checks: Vec<String>,
}

fn mean(values: Vec<f64>) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

impl FormMetricsReport {
    fn build(model: &str, runs: &[CaseRun]) -> Self {
        let mut latencies: Vec<u64> = runs.iter().map(|r| r.latency_ms as u64).collect();
        latencies.sort_unstable();
        Self {
            model: model.to_string(),
            total_cases: runs.len(),
            passed: runs.iter().filter(|r| r.report.passed).count(),
            unparseable: runs.iter().filter(|r| r.fill.is_none()).count(),
            invented_keys: runs
                .iter()
                .filter_map(|r| r.fill.as_ref())
                .map(|f| f.dropped.len())
                .sum(),
            missing_required: runs
                .iter()
                .filter_map(|r| r.fill.as_ref())
                .map(|f| f.missing_required.len())
                .sum(),
            latency_ms_p50: percentile(&latencies, 0.50),
            latency_ms_p95: percentile(&latencies, 0.95),
            prompt_tokens_mean: mean(runs.iter().map(|r| r.prompt_tokens as f64).collect()),
            cases: runs
                .iter()
                .map(|r| CaseSummary {
                    id: r.case.id.clone(),
                    passed: r.report.passed,
                    latency_ms: r.latency_ms,
                    failed_checks: r
                        .report
                        .checks
                        .iter()
                        .filter(|c| !c.passed())
                        .map(|c| format!("{}: wanted {}, got {}", c.field, c.expected, c.actual))
                        .collect(),
                })
                .collect(),
        }
    }
}

pub async fn run(cfg: FormRunnerConfig) -> EvalResult<FormMetricsReport> {
    let mut cases = load_form_cases(&cfg.cases_dir)?;
    if let Some(id) = &cfg.only_case {
        cases.retain(|c| &c.id == id);
        if cases.is_empty() {
            return Err(EvalError::Config(format!("no form case with id `{id}`")));
        }
    }

    // Same isolation as the other harnesses: work on a copy, never the live DB.
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "forms")?;
    let db = std::sync::Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);
    crate::evals::shared::apply_eval_model_override_from_env(&db)?;
    if let Some(model) = &cfg.model_override {
        db.set_preference("ai_model", model)?;
    }

    let provider = crate::services::ai::AiService::load_provider(&db)
        .map_err(|e| EvalError::Config(format!("no AI provider available: {e}")))?;
    let template = crate::services::prompts::get_template(&db, "forms.fill")
        .map_err(|e| EvalError::Config(format!("cannot load forms.fill: {e}")))?;
    let model = db.get_preference("ai_model")?.unwrap_or_default();
    let default_today = chrono::Local::now().format("%Y-%m-%d").to_string();

    if !cfg.json_stdout {
        println!("[form-eval] model = {model}");
        println!("[form-eval] running {} case(s)", cases.len());
    }

    let mut runs = Vec::new();
    for case in cases {
        let Some(form) = registry::lookup(&case.form) else {
            return Err(EvalError::Config(format!(
                "case `{}` names an unregistered form `{}`",
                case.id, case.form
            )));
        };
        let today = case.today.clone().unwrap_or_else(|| default_today.clone());
        let language = case.language.clone().unwrap_or_else(|| "English".to_string());
        let current_values = case
            .current_values
            .as_ref()
            .and_then(|v| serde_json::to_value(v).ok())
            .unwrap_or_else(|| serde_json::json!({}));

        let started = Instant::now();
        let outcome = fill_form(
            provider.as_ref(),
            &template,
            form,
            &language,
            &today,
            &current_values,
            &case.request,
        )
        .await;
        let latency_ms = started.elapsed().as_millis();
        let report = evaluate(&case, outcome.fill.as_ref());

        if !cfg.json_stdout {
            println!(
                "[form-eval] {} {} ({}/{} checks, {latency_ms}ms)",
                if report.passed { "OK  " } else { "FAIL" },
                case.id,
                report.checks.iter().filter(|c| c.passed()).count(),
                report.checks.len(),
            );
            for check in report.checks.iter().filter(|c| !c.passed()) {
                println!(
                    "[form-eval]      {}: wanted {}, got {}",
                    check.field, check.expected, check.actual
                );
            }
        }

        runs.push(CaseRun {
            case,
            fill: outcome.fill,
            report,
            latency_ms,
            prompt_tokens: outcome.prompt_tokens,
        });
    }

    let metrics = FormMetricsReport::build(&model, &runs);

    // The shared eval-report schema, so `make verify` picks this harness up the
    // same way it picks up junk and translation — one record per case, no
    // bespoke parsing in `verify_all.py`.
    let mut report = JsonRunReport::new("form_fill_eval", &model);
    for run in &runs {
        let failed: Vec<String> = run
            .report
            .checks
            .iter()
            .filter(|c| !c.passed())
            .map(|c| format!("{}: wanted {}, got {}", c.field, c.expected, c.actual))
            .collect();
        let total = run.report.checks.len().max(1) as f64;
        let passed = run.report.checks.iter().filter(|c| c.passed()).count() as f64;
        report.push(ItemResult {
            id: run.case.id.clone(),
            passed: run.report.passed,
            score: Some(passed / total),
            detail: if failed.is_empty() {
                run.case.note.clone()
            } else {
                failed.join(" · ")
            },
        });
    }
    report.write(&cfg.out_dir)?;

    if cfg.json_stdout {
        println!("{}", serde_json::to_string_pretty(&metrics)?);
    } else {
        println!("[form-eval] {}/{} cases passed", metrics.passed, metrics.total_cases);
    }
    Ok(metrics)
}
