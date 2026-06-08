// Eval runner — orchestrates the full pipeline.
//
// Responsibilities:
//   1. Confirm judge-access env (OPENROUTER_API_KEY) or the --no-judge flag.
//   2. Copy the production SQLite DB to a temp location so the real DB stays
//      untouched.
//   3. Load cases and apply --tier / --case filters.
//   4. Iterate cases serially (Ollama + local DB don't benefit from parallelism).
//   5. Render an HTML report.

use std::path::PathBuf;
use std::sync::Arc;

use crate::db::Database;
use crate::evals::case_loader::{load_cases, EvalCase};
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::harness::{run_case, CaseOutcome};
use crate::evals::judge::{Judge, JudgeScores};
use crate::evals::metrics::{evaluate, HeuristicReport};
use crate::evals::report::{render, ReportCase};
use crate::evals::shared::build_mock_app;
use crate::evals::{EvalError, EvalResult};

#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub tier: String,
    pub only_case: Option<String>,
    pub yes: bool,
    pub model_override: Option<String>,
    pub account_id: Option<String>,
    pub no_judge: bool,
    pub out_dir: PathBuf,
    pub cases_dir: PathBuf,
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
}

/// Run the whole suite. Returns the path to the generated report on success.
pub async fn run(cfg: RunnerConfig) -> EvalResult<PathBuf> {
    // ── 1. Judge env / confirmation ─────────────────────────────────────────
    let api_key = std::env::var("OPENROUTER_API_KEY").ok();
    let judge_model = std::env::var("OPENROUTER_JUDGE_MODEL").ok();

    let judge_enabled = !cfg.no_judge;
    if judge_enabled {
        if api_key.is_none() {
            return Err(EvalError::Config(
                "OPENROUTER_API_KEY is not set. Export it or pass --no-judge.".into(),
            ));
        }
        let model_name = judge_model
            .clone()
            .unwrap_or_else(|| "anthropic/claude-sonnet-4.5".into());
        eprintln!(
            "[eval] judge ENABLED — will send question + response + sources to OpenRouter (model={}).",
            model_name
        );
        if !cfg.yes {
            eprintln!(
                "[eval] pass --yes to skip this confirmation. Aborting for safety. (Rerun with --yes or --no-judge.)"
            );
            return Err(EvalError::Aborted("judge requires --yes or --no-judge".into()));
        }
    } else {
        eprintln!("[eval] judge DISABLED (--no-judge) — heuristics only.");
    }

    // ── 2. Prepare DB ───────────────────────────────────────────────────────
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "chat")?;
    let db = Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);
    crate::evals::shared::apply_eval_model_override_from_env(&db)?;
    // When no env override is active, default the suite to the app default
    // provider (local llama.cpp) + default model rather than inheriting the
    // copied prod DB's prefs. Per-case `model:` still overrides via the pin in
    // the run loop below.
    crate::evals::shared::pin_eval_provider(&db, crate::evals::shared::DEFAULT_EVAL_MODEL)?;

    // ── 3. Resolve suite-level default account ──────────────────────────────
    // Cases may override via their own `account:` field (matched against id or
    // email). We only need a suite-level default for cases that omit the field;
    // if none do, multiple enabled accounts is fine.
    let suite_default_account_id: Option<String> = match cfg.account_id.clone() {
        Some(a) => Some(a),
        None => {
            let accounts = db.list_accounts()?;
            let enabled: Vec<_> = accounts.into_iter().filter(|a| a.enabled).collect();
            match enabled.len() {
                0 => None,
                1 => Some(enabled[0].id.clone()),
                _ => None, // defer; per-case override required
            }
        }
    };
    eprintln!(
        "[eval] suite default account = {}",
        suite_default_account_id
            .as_deref()
            .unwrap_or("<none — per-case account required>")
    );

    // ── 4. Resolve model ────────────────────────────────────────────────────
    let default_model = resolve_default_model(&db);
    eprintln!("[eval] default model = {}", default_model);

    // ── 5. Load cases ───────────────────────────────────────────────────────
    let mut cases = load_cases(&cfg.cases_dir)?;
    if cfg.tier != "all" {
        cases.retain(|c| c.tier == cfg.tier);
    }
    if let Some(only) = cfg.only_case.as_deref() {
        cases.retain(|c| c.id == only);
    }
    if cases.is_empty() {
        return Err(EvalError::Config("no cases matched the tier/id filter".into()));
    }
    eprintln!("[eval] running {} case(s)", cases.len());

    // ── 6. Build a Tauri app handle for event emission ──────────────────────
    let app = build_mock_app()?;

    // ── 7. Iterate cases serially ───────────────────────────────────────────
    let judge = if judge_enabled {
        Some(Judge::new(
            api_key.expect("api_key should be Some when judge_enabled"),
            judge_model,
        ))
    } else {
        None
    };

    let mut outcomes: Vec<(EvalCase, CaseOutcome, HeuristicReport, JudgeScores)> = Vec::new();

    // Cache enabled accounts once so per-case account resolution doesn't hit
    // the DB inside the loop.
    let enabled_accounts = db.list_accounts()?;

    for case in &cases {
        eprintln!("[eval] ── {} ── {}", case.id, case.question);
        let model = cfg
            .model_override
            .clone()
            .or_else(|| case.model.clone())
            .unwrap_or_else(|| default_model.clone());

        // Per-case account override: accept either a full account id or an
        // email address, falling back to the suite-level default.
        let effective_account_id = match case.account.as_deref() {
            Some(hint) => {
                let h = hint.trim();
                let resolved = enabled_accounts
                    .iter()
                    .find(|a| a.id.eq_ignore_ascii_case(h) || a.email.eq_ignore_ascii_case(h));
                match resolved {
                    Some(a) => a.id.clone(),
                    None => {
                        return Err(EvalError::Config(format!(
                            "case {}: account '{}' not found in DB",
                            case.id, hint
                        )))
                    }
                }
            }
            None => match suite_default_account_id.as_ref() {
                Some(a) => a.clone(),
                None => {
                    return Err(EvalError::Config(format!(
                        "case {}: no `account:` set and multiple enabled accounts — add an override",
                        case.id
                    )))
                }
            },
        };

        // Evals must run on the app default provider (local llama.cpp) unless an
        // explicit EMAILOPS_EVAL_MODEL override is set. `run_case` → `run_chat_turn`
        // resolves the provider from DB prefs via `load_provider`, so pin them here.
        crate::evals::shared::pin_eval_provider(&db, &model)?;

        match run_case(db.clone(), app.clone(), &effective_account_id, &model, case).await {
            Ok(outcome) => {
                let heuristics = evaluate(case, &outcome)?;
                // Skip the judge entirely when the answer is empty — there is
                // nothing to score and it would burn judge quota for no signal.
                let judge_scores = if outcome.assistant_content.trim().is_empty() {
                    JudgeScores {
                        error: Some("skipped: empty assistant answer".into()),
                        ..JudgeScores::default()
                    }
                } else {
                    match &judge {
                        Some(j) => j.score(case, &outcome).await,
                        None => JudgeScores::default(),
                    }
                };
                eprintln!(
                    "[eval]    {} heuristic checks passed ({}/{})",
                    if heuristics.all_passed() { "OK" } else { "FAIL" },
                    heuristics.passed_count(),
                    heuristics.total()
                );
                // Clean up the eval-created conversation from the benchmark DB.
                // FK cascade removes chat_messages + chat_message_sources.
                let conv_id = outcome.conversation_id.clone();
                outcomes.push((case.clone(), outcome, heuristics, judge_scores));
                if !conv_id.is_empty() {
                    if let Err(e) = db.delete_chat_conversation(&conv_id) {
                        eprintln!("[eval]    WARN: failed to delete eval conversation {}: {}", conv_id, e);
                    }
                }
            }
            Err(e) => {
                eprintln!("[eval]    ERROR: {}", e);
                // Record a placeholder outcome so the report still shows the case.
                let placeholder = CaseOutcome {
                    conversation_id: String::new(),
                    conversation_title: "<error>".into(),
                    assistant_message_id: String::new(),
                    assistant_content: format!("harness error: {}", e),
                    assistant_trace: None,
                    assistant_token_count: None,
                    assistant_latency_ms: None,
                    wall_elapsed_ms: 0,
                    sources_used: Vec::new(),
                };
                let report = HeuristicReport {
                    checks: vec![crate::evals::metrics::HeuristicCheck {
                        name: "case_run".into(),
                        passed: false,
                        expected: "case runs to completion".into(),
                        actual: format!("{}", e),
                        detail: "harness-level failure".into(),
                    }],
                };
                outcomes.push((case.clone(), placeholder, report, JudgeScores::default()));
            }
        }
    }

    // ── 8. Render report ────────────────────────────────────────────────────
    let report_cases: Vec<ReportCase> = outcomes
        .iter()
        .map(|(c, o, h, j)| ReportCase {
            case: c,
            outcome: o,
            heuristics: h,
            judge: j,
        })
        .collect();

    let judge_model_for_report =
        std::env::var("OPENROUTER_JUDGE_MODEL").unwrap_or_else(|_| "anthropic/claude-sonnet-4.5".to_string());
    let chat_model_for_report = cfg.model_override.clone().unwrap_or_else(|| default_model.clone());
    let path = render(
        &cfg.out_dir,
        &report_cases,
        &chat_model_for_report,
        judge_enabled,
        &judge_model_for_report,
    )?;
    eprintln!("[eval] report written to {}", path.display());
    Ok(path)
}

/// Pick the preferred chat model from `user_preferences`, else a sensible default.
fn resolve_default_model(db: &Arc<Database>) -> String {
    match db.get_preference("ai_model") {
        Ok(Some(v)) if !v.is_empty() => v,
        _ => crate::evals::shared::DEFAULT_EVAL_MODEL.to_string(),
    }
}
