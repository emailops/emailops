// Shortcut-variant runner.
//
// For each (shortcut, variant) pair:
//   1. Resolve the shortcut's account to an id.
//   2. Create a throwaway conversation in an isolated benchmark DB copy.
//   3. Run the real chat pipeline with the variant prompt as the user message.
//   4. Score the answer against the deterministic rubric AND the split judge.
//   5. Clean up the eval conversation (FK cascade removes messages + sources).
// Emits an HTML report at the end.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::db::Database;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::harness::{CaseOutcome, SourceSummary};
use crate::evals::shortcuts::case_loader::{load_shortcut_cases, ShortcutVariant};
use crate::evals::shortcuts::judge::{ShortcutJudge, VariantScores};
use crate::evals::shortcuts::metrics::{evaluate as run_rubric, RubricReport};
use crate::evals::shortcuts::report::{render, ReportShortcut, ReportVariant};
use crate::evals::{EvalError, EvalResult};
use crate::models::{ChatConversation, ChatMessage};
use crate::services::chat;

#[derive(Debug, Clone)]
pub struct ShortcutRunnerConfig {
    pub only_shortcut: Option<String>,
    pub only_variant: Option<String>,
    pub yes: bool,
    pub model_override: Option<String>,
    pub no_judge: bool,
    pub out_dir: PathBuf,
    pub cases_dir: PathBuf,
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
}

pub async fn run(cfg: ShortcutRunnerConfig) -> EvalResult<PathBuf> {
    // ── 1. Judge env gate ───────────────────────────────────────────────────
    let api_key = std::env::var("OPENROUTER_API_KEY").ok();
    let judge_model_env = std::env::var("OPENROUTER_JUDGE_MODEL").ok();

    let judge_enabled = !cfg.no_judge;
    if judge_enabled {
        if api_key.is_none() {
            return Err(EvalError::Config(
                "OPENROUTER_API_KEY is not set. Export it or pass --no-judge.".into(),
            ));
        }
        let model_name = judge_model_env
            .clone()
            .unwrap_or_else(|| "anthropic/claude-sonnet-4.5".into());
        eprintln!(
            "[shortcut-eval] judge ENABLED — will send prompt + response + sources to OpenRouter (model={}).",
            model_name
        );
        if !cfg.yes {
            return Err(EvalError::Aborted("judge requires --yes or --no-judge".into()));
        }
    } else {
        eprintln!("[shortcut-eval] judge DISABLED (--no-judge) — rubric only.");
    }

    // ── 2. Prepare DB ───────────────────────────────────────────────────────
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "shortcuts")?;
    let db = Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);

    // ── 3. Load cases + apply filters ───────────────────────────────────────
    let mut cases = load_shortcut_cases(&cfg.cases_dir)?;
    if let Some(id) = cfg.only_shortcut.as_deref() {
        cases.retain(|c| c.shortcut_id == id);
    }
    if let Some(vid) = cfg.only_variant.as_deref() {
        for c in cases.iter_mut() {
            c.variants.retain(|v| v.id == vid);
        }
        cases.retain(|c| !c.variants.is_empty());
    }
    if cases.is_empty() {
        return Err(EvalError::Config(
            "no shortcuts matched the --shortcut / --variant filters".into(),
        ));
    }
    let total_variants: usize = cases.iter().map(|c| c.variants.len()).sum();
    eprintln!(
        "[shortcut-eval] running {} shortcut(s), {} variant(s) total",
        cases.len(),
        total_variants
    );

    // ── 4. Account resolution per case ──────────────────────────────────────
    let enabled_accounts = db.list_accounts()?;

    // ── 6. Judge ────────────────────────────────────────────────────────────
    let judge = if judge_enabled {
        Some(ShortcutJudge::new(
            api_key.expect("api_key is Some when judge_enabled"),
            judge_model_env.clone(),
        ))
    } else {
        None
    };

    // ── 7. Run loop ─────────────────────────────────────────────────────────
    let mut report_shortcuts: Vec<ReportShortcut> = Vec::new();

    for case in &cases {
        // Resolve account — hard error if not found (rather than fall through
        // to "multiple accounts" since each shortcut file is explicit).
        let acct_hint = case.account.trim();
        let account = enabled_accounts
            .iter()
            .find(|a| a.id.eq_ignore_ascii_case(acct_hint) || a.email.eq_ignore_ascii_case(acct_hint));
        let account_id = match account {
            Some(a) => a.id.clone(),
            None => {
                return Err(EvalError::Config(format!(
                    "shortcut {}: account '{}' not found in DB",
                    case.shortcut_id, acct_hint
                )))
            }
        };

        let model = cfg.model_override.clone().unwrap_or_else(|| case.model.clone());

        eprintln!(
            "[shortcut-eval] ── {} — {} (account={}, model={}, {} variants)",
            case.shortcut_id,
            case.label,
            acct_hint,
            model,
            case.variants.len()
        );

        let mut report_variants: Vec<ReportVariant> = Vec::new();
        for variant in &case.variants {
            eprintln!("[shortcut-eval]    → variant {}", variant.id);
            match run_variant(db.clone(), &account_id, &model, variant).await {
                Ok(outcome) => {
                    let rubric = run_rubric(&case.rubric, &outcome.assistant_content);
                    let scores = if outcome.assistant_content.trim().is_empty() {
                        VariantScores {
                            error: Some("skipped: empty answer".into()),
                            ..Default::default()
                        }
                    } else {
                        match &judge {
                            Some(j) => j.score(case, variant, &outcome).await,
                            None => VariantScores::default(),
                        }
                    };
                    eprintln!(
                        "[shortcut-eval]       rubric {}/{} passed, composite={}",
                        rubric.passed_count(),
                        rubric.total(),
                        scores
                            .composite()
                            .map(|c| format!("{:.2}", c))
                            .unwrap_or_else(|| "-".into())
                    );
                    let conv_id = outcome.conversation_id.clone();
                    report_variants.push(ReportVariant {
                        variant: variant.clone(),
                        outcome: Some(outcome),
                        rubric,
                        scores,
                        error: None,
                    });
                    if !conv_id.is_empty() {
                        if let Err(e) = db.delete_chat_conversation(&conv_id) {
                            eprintln!(
                                "[shortcut-eval]       WARN: failed to delete eval conversation {}: {}",
                                conv_id, e
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[shortcut-eval]       ERROR: {}", e);
                    report_variants.push(ReportVariant {
                        variant: variant.clone(),
                        outcome: None,
                        rubric: RubricReport::default(),
                        scores: VariantScores::default(),
                        error: Some(format!("{}", e)),
                    });
                }
            }
        }

        report_shortcuts.push(ReportShortcut {
            case: case.clone(),
            variants: report_variants,
        });
    }

    // ── 8. Render ───────────────────────────────────────────────────────────
    let judge_model_for_report = judge_model_env.unwrap_or_else(|| "anthropic/claude-sonnet-4.5".to_string());
    let path = render(&cfg.out_dir, &report_shortcuts, judge_enabled, &judge_model_for_report)?;
    eprintln!("[shortcut-eval] report written to {}", path.display());
    Ok(path)
}

async fn run_variant(
    db: Arc<Database>,
    account_id: &str,
    model: &str,
    variant: &ShortcutVariant,
) -> EvalResult<CaseOutcome> {
    let start = Instant::now();

    // Evals must run on the app default provider (local llama.cpp), not whatever
    // the copied prod DB had configured. `run_chat_turn` selects the provider via
    // `load_provider(&db)` (the `model` arg is cosmetic), so pin the DB prefs here.
    crate::evals::shared::pin_eval_provider(&db, model)?;

    let conv: ChatConversation = db.create_chat_conversation(account_id, "New chat")?;
    let user_msg: ChatMessage = db.insert_chat_message(&conv.id, "user", &variant.prompt, None)?;
    let assistant_msg: ChatMessage = db.insert_chat_message(&conv.id, "assistant", "", Some(model))?;

    let history: Vec<ChatMessage> = Vec::new();
    let categories: Vec<String> = crate::services::chat::DEFAULT_RAG_CATEGORIES
        .iter()
        .map(|s| s.to_string())
        .collect();

    // See harness.rs: run_chat_turn now takes a ToolRegistry.
    let registry = std::sync::Arc::new(crate::services::chat::tools::default_registry());
    chat::run_chat_turn(
        db.clone(),
        registry,
        conv.id.clone(),
        user_msg.id.clone(),
        assistant_msg.id.clone(),
        account_id.to_string(),
        variant.prompt.clone(),
        model.to_string(),
        history,
        categories,
        None,
        None,
    )
    .await?;

    let wall_elapsed_ms = start.elapsed().as_millis() as i64;

    let messages = db.get_chat_messages(&conv.id)?;
    let assistant = messages.into_iter().find(|m| m.id == assistant_msg.id).ok_or_else(|| {
        EvalError::Config(format!(
            "assistant message {} missing from conversation {}",
            assistant_msg.id, conv.id
        ))
    })?;

    let final_title = db
        .get_chat_conversation(&conv.id)?
        .map(|c| c.title)
        .unwrap_or_else(|| "New chat".into());

    // Re-derive source body snippets the same way the chat eval does, so the
    // report can show exactly what context the model was given per variant.
    let sources_used: Vec<SourceSummary> = assistant
        .sources
        .iter()
        .map(|s| {
            let raw_body = db.get_email_body(&s.email_id).unwrap_or_default();
            let plain = crate::util::html::strip_html_for_fts(&raw_body);
            let snippet = chat::smart_body_slice(&plain, &variant.prompt, chat::FULL_BUDGET.source_body_chars);
            SourceSummary {
                citation_number: s.citation_number,
                email_id: s.email_id.clone(),
                subject: s.subject.clone(),
                sender: s.sender.clone(),
                sender_email: s.sender_email.clone(),
                relevance_score: s.relevance_score,
                body_snippet: snippet,
            }
        })
        .collect();

    Ok(CaseOutcome {
        conversation_id: conv.id,
        conversation_title: final_title,
        assistant_message_id: assistant.id,
        assistant_content: assistant.content,
        assistant_trace: assistant.trace,
        assistant_token_count: assistant.token_count,
        assistant_latency_ms: assistant.latency_ms,
        wall_elapsed_ms,
        sources_used,
    })
}
