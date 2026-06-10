// Orchestrates the agent-search eval.
//
// Per case:
//   1. Resolve the case's account (by id or email) against an isolated benchmark DB.
//   2. Run each configured AgentSearchMode and collect their top-K hits.
//   3. Build the pool: union of all email IDs returned across modes.
//   4. Judge every pool member with the OpenRouter judge.
//   5. Compute precision@K / recall@K / F1@K per mode.
//
// All modes share the same pool, so recall is comparable across modes (the
// denominator is the number of relevant emails the *union* found — we cannot
// estimate true recall without exhaustive labelling, but pool recall is the
// standard surrogate).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::db::Database;
use crate::evals::agent_search::cases::{load_cases, AgentSearchCase};
use crate::evals::agent_search::judge::{Judgment, PoolJudge};
use crate::evals::agent_search::report;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::{EvalError, EvalResult};
use crate::services::agent_search::{
    run_agent_search, AgentSearchHit, AgentSearchMode, AgentSearchOptions, AgentSearchResult,
};
use crate::services::ai::AiService;

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub cases_dir: PathBuf,
    pub out_dir: PathBuf,
    pub prod_db_path: PathBuf,
    pub yes: bool,
    pub no_judge: bool,
    pub db_mode: EvalDbMode,
    pub top_k: usize,
    pub only_case: Option<String>,
    pub modes: Vec<AgentSearchMode>,
}

/// What a single mode produced for a single case — kept for the report.
#[derive(Debug, Serialize)]
pub struct ModeOutcome {
    pub mode: AgentSearchMode,
    pub hits: Vec<AgentSearchHit>,
    pub elapsed_ms: i64,
    pub stage_counts: HashMap<String, usize>,
    pub query_plan_raw: Option<String>,
    pub error: Option<String>,
    pub metrics: ModeMetrics,
}

#[derive(Debug, Serialize, Default, Clone, Copy)]
pub struct ModeMetrics {
    pub precision_at_k: f32,
    pub recall_at_k: f32,
    pub f1_at_k: f32,
    /// MRR: reciprocal rank of the first clearly-relevant (score==2) hit.
    pub mrr: f32,
    /// Number of clearly-relevant hits in the top-K.
    pub relevant_in_top_k: usize,
    /// Total pool size (denominator candidates).
    pub pool_size: usize,
    /// Number of clearly-relevant emails in the *pool* (recall denominator).
    pub relevant_in_pool: usize,
}

#[derive(Debug, Serialize)]
pub struct CaseOutcome {
    pub case: AgentSearchCase,
    pub mode_outcomes: Vec<ModeOutcome>,
    /// Per-email judgments, keyed by email_id. Shared across modes.
    pub judgments: HashMap<String, Judgment>,
}

pub async fn run(cfg: RunConfig) -> EvalResult<PathBuf> {
    // ── 1. Judge env ────────────────────────────────────────────────────────
    let api_key = std::env::var("OPENROUTER_API_KEY").ok();
    let judge_model_override = std::env::var("OPENROUTER_JUDGE_MODEL").ok();

    let judge_enabled = !cfg.no_judge;
    if judge_enabled {
        if api_key.is_none() {
            return Err(EvalError::Config(
                "OPENROUTER_API_KEY is not set. Export it or pass --no-judge.".into(),
            ));
        }
        let display_model = judge_model_override
            .clone()
            .unwrap_or_else(|| "anthropic/claude-sonnet-4.5".into());
        eprintln!(
            "[agent-search-eval] judge ENABLED — pooled emails will be sent to OpenRouter (model={}).",
            display_model
        );
        if !cfg.yes {
            eprintln!("[agent-search-eval] pass --yes to skip this confirmation. Aborting for safety.");
            return Err(EvalError::Aborted("judge requires --yes or --no-judge".into()));
        }
    } else {
        eprintln!("[agent-search-eval] judge DISABLED (--no-judge) — pool will be returned unjudged.");
    }

    // ── 2. Prepare DB ───────────────────────────────────────────────────────
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "agent-search")?;
    let db = Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);
    crate::evals::shared::apply_eval_model_override_from_env(&db)?;
    let accounts = db.list_accounts()?;

    // ── 3. AI service (optional for baseline mode but required for smart) ──
    let ai_service = AiService::new(db.clone()).ok();
    if ai_service.is_none() {
        eprintln!("[agent-search-eval] WARNING: AiService unavailable — smart mode will fail");
    } else {
        eprintln!(
            "[agent-search-eval] AI provider loaded: {:?}",
            ai_service.as_ref().map(|s| s.provider().provider_type().to_string())
        );
    }

    // ── 4. Load cases ───────────────────────────────────────────────────────
    let mut cases = load_cases(&cfg.cases_dir)?;
    if let Some(only) = cfg.only_case.as_deref() {
        cases.retain(|c| c.id == only);
    }
    if cases.is_empty() {
        return Err(EvalError::Config("no cases matched the filter".into()));
    }
    eprintln!("[agent-search-eval] {} case(s) loaded", cases.len());

    let judge = if judge_enabled {
        Some(PoolJudge::new(
            api_key.expect("checked above"),
            judge_model_override.clone(),
        ))
    } else {
        None
    };

    // ── 5. Iterate cases ────────────────────────────────────────────────────
    let mut outcomes: Vec<CaseOutcome> = Vec::new();
    for case in &cases {
        eprintln!("\n[agent-search-eval] ── {} ── {}", case.id, case.question);

        // Resolve account.
        let acc_hint = case
            .account
            .as_deref()
            .ok_or_else(|| EvalError::Config(format!("case {} has no account set", case.id)))?;
        let account = accounts
            .iter()
            .find(|a| a.id.eq_ignore_ascii_case(acc_hint) || a.email.eq_ignore_ascii_case(acc_hint))
            .ok_or_else(|| EvalError::Config(format!("case {}: account '{}' not found", case.id, acc_hint)))?;

        // Run each mode.
        let mut mode_outcomes: Vec<ModeOutcome> = Vec::new();
        for &mode in &cfg.modes {
            eprintln!("[agent-search-eval]   running mode={}", mode.as_str());
            let opts = AgentSearchOptions {
                mode,
                top_k: cfg.top_k,
                categories: None,
            };
            let t0 = Instant::now();
            let res: Result<AgentSearchResult, _> =
                run_agent_search(db.clone(), ai_service.as_ref(), &account.id, &case.question, opts).await;
            let elapsed_ms = t0.elapsed().as_millis() as i64;
            match res {
                Ok(r) => {
                    eprintln!(
                        "[agent-search-eval]     mode={} → {} hits in {}ms (stages: {:?})",
                        mode.as_str(),
                        r.hits.len(),
                        elapsed_ms,
                        r.stage_counts
                    );
                    mode_outcomes.push(ModeOutcome {
                        mode,
                        hits: r.hits,
                        elapsed_ms,
                        stage_counts: r.stage_counts,
                        query_plan_raw: r.query_plan.map(|p| {
                            format!(
                                "keywords={:?} direction={:?} semantic_query={:?}\n--- raw ---\n{}",
                                p.keywords, p.direction, p.semantic_query, p.raw
                            )
                        }),
                        error: None,
                        metrics: ModeMetrics::default(),
                    });
                }
                Err(e) => {
                    eprintln!("[agent-search-eval]     mode={} ERROR: {}", mode.as_str(), e);
                    mode_outcomes.push(ModeOutcome {
                        mode,
                        hits: Vec::new(),
                        elapsed_ms,
                        stage_counts: HashMap::new(),
                        query_plan_raw: None,
                        error: Some(e.to_string()),
                        metrics: ModeMetrics::default(),
                    });
                }
            }
        }

        // Build pool: union of email_ids across modes, keeping the first hit
        // (any mode) so we have full metadata for judging.
        let mut pool: HashMap<String, AgentSearchHit> = HashMap::new();
        for mo in &mode_outcomes {
            for h in &mo.hits {
                pool.entry(h.email_id.clone()).or_insert_with(|| h.clone());
            }
        }
        let pool_size = pool.len();
        eprintln!("[agent-search-eval]   pool size: {} unique emails", pool_size);

        // Judge the pool.
        let mut judgments: HashMap<String, Judgment> = HashMap::new();
        if let Some(j) = &judge {
            for (i, (eid, hit)) in pool.iter().enumerate() {
                let t = Instant::now();
                let judgment = j
                    .score(&case.question, &case.judge_criteria, hit)
                    .await
                    .unwrap_or_else(|e| Judgment {
                        score: 0,
                        rationale: String::new(),
                        error: Some(e.to_string()),
                    });
                let elapsed = t.elapsed().as_millis();
                eprintln!(
                    "[agent-search-eval]     judge [{}/{}] {} → score={} ({}ms){}",
                    i + 1,
                    pool_size,
                    truncate(&hit.subject, 50),
                    judgment.score,
                    elapsed,
                    judgment
                        .error
                        .as_ref()
                        .map(|e| format!(" ERR: {}", e))
                        .unwrap_or_default()
                );
                judgments.insert(eid.clone(), judgment);
            }
        }

        // Compute per-mode metrics.
        let relevant_in_pool = judgments.values().filter(|j| j.is_clearly_relevant()).count();
        let relevant_in_pool_loose = judgments.values().filter(|j| j.is_relevant()).count();
        for mo in &mut mode_outcomes {
            mo.metrics = compute_metrics(
                &mo.hits,
                &judgments,
                cfg.top_k,
                pool_size,
                relevant_in_pool,
                relevant_in_pool_loose,
            );
            eprintln!(
                "[agent-search-eval]   mode={}  P@{k}={:.2}  R@{k}={:.2}  F1={:.2}  MRR={:.2}  (relevant_in_top={}, relevant_in_pool={})",
                mo.mode.as_str(),
                mo.metrics.precision_at_k,
                mo.metrics.recall_at_k,
                mo.metrics.f1_at_k,
                mo.metrics.mrr,
                mo.metrics.relevant_in_top_k,
                mo.metrics.relevant_in_pool,
                k = cfg.top_k,
            );
        }

        outcomes.push(CaseOutcome {
            case: case.clone(),
            mode_outcomes,
            judgments,
        });
    }

    // ── 6. Render report ────────────────────────────────────────────────────
    let judge_model = judge
        .as_ref()
        .map(|j| j.model_name().to_string())
        .unwrap_or_else(|| "<no judge>".into());
    let path = report::render(&cfg.out_dir, &outcomes, judge_enabled, &judge_model, cfg.top_k)?;
    eprintln!("\n[agent-search-eval] report written to {}", path.display());
    Ok(path)
}

fn compute_metrics(
    hits: &[AgentSearchHit],
    judgments: &HashMap<String, Judgment>,
    k: usize,
    pool_size: usize,
    relevant_in_pool: usize,
    _relevant_in_pool_loose: usize,
) -> ModeMetrics {
    let top_ids: Vec<&String> = hits.iter().take(k).map(|h| &h.email_id).collect();
    let mut relevant_in_top = 0usize;
    let mut loose_relevant_in_top = 0usize;
    let mut mrr = 0.0f32;
    let mut seen: HashSet<&String> = HashSet::new();
    for (rank, eid) in top_ids.iter().enumerate() {
        if !seen.insert(*eid) {
            continue; // ignore dupes in top-K
        }
        if let Some(j) = judgments.get(*eid) {
            if j.is_clearly_relevant() {
                relevant_in_top += 1;
                if mrr == 0.0 {
                    mrr = 1.0 / (rank as f32 + 1.0);
                }
            }
            if j.is_relevant() {
                loose_relevant_in_top += 1;
            }
        }
    }

    let returned = top_ids.len().max(1);
    let precision = relevant_in_top as f32 / returned as f32;
    let recall = if relevant_in_pool == 0 {
        0.0
    } else {
        relevant_in_top as f32 / relevant_in_pool as f32
    };
    let f1 = if precision + recall > 0.0 {
        2.0 * precision * recall / (precision + recall)
    } else {
        0.0
    };
    // Also surface loose metrics in the rationale via mrr fallback if no strict.
    let _ = loose_relevant_in_top;

    ModeMetrics {
        precision_at_k: precision,
        recall_at_k: recall,
        f1_at_k: f1,
        mrr,
        relevant_in_top_k: relevant_in_top,
        pool_size,
        relevant_in_pool,
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}
