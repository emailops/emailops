//! Research mode: a chat turn that reads every email a question is about.
//!
//! A normal turn answers from ~8 retrieved sources or one page of 25 search
//! rows — right for "what did X say?", too thin for "list every contact
//! request". Research mode trades minutes (or hours) for coverage:
//!
//! 1. **Plan** — the query planner turns the question into a filter.
//! 2. **Gather** — every email that filter matches (whole threads), or, for a
//!    topic question, every email close enough in meaning. No cap: the user
//!    sees the count and the time estimate first and confirms
//!    ([`estimate`]), and can stop the run at any point ([`request_stop`]).
//! 3. **Map** — read the emails in batches sized to the context window, one
//!    completion per batch that keeps the findings relevant to the question,
//!    each tied to its `email://` id.
//! 4. **Condense** — when the notes outgrow one prompt, merge them in groups,
//!    in rounds, until they fit.
//! 5. **Reduce** — one completion writes the report from the notes.
//!
//! Every LLM call is a one-shot completion on the auxiliary prefix slot, so
//! the chat's own KV anchor (the `chat.system` prompt) survives for the next
//! ordinary turn. Pure planners live in `plan` / `prompts`; this file is the
//! thin executor.

mod control;
mod mode;
mod notes;
mod plan;
mod prompts;
mod reading;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;

pub use control::{confirm_exit, exit_decision, request_stop, running_runs, ExitDecision};
pub(crate) use control::{register_run, store_estimate, take_estimate};
use notes::{
    citation_targets, condense_shape, finish_report, notes_len, number_conversations, parse_condensed,
    render_citations, render_condense_input, render_match_list, render_notes, Note,
};
use plan::{
    for_every_user_address, merge_candidates, plan_batches, plan_condense_groups, plan_estimate, plan_gather,
    plan_research_budget, semantic_cutoff, GatherStep, CONDENSE_MAX_TOKENS, MAP_MAX_TOKENS,
};
use prompts::{
    cancelled_note, coverage_line, participant, recipients, report_facts, split_condense_prompt, split_map_prompt,
    split_reduce_prompt, DocMessage, ResearchDoc,
};
pub(crate) use reading::Match;
use reading::{collect_matches, enforce_direction, BatchLabels, Finding};

use super::planner::SearchPlan;
use crate::ai::provider::{AIProvider, CompletionOptions, CompletionResult};
use crate::db::emails::search::TagQuery;
use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Email, LlmCallTrace, ReportMode, ResearchEstimate, ResearchTrace, ToolCallTrace};

// ── Context window ──────────────────────────────────────────────────────────

/// Ollama's default `num_ctx` (see `ai::ollama`), and the window every other
/// backend is assumed to have.
const DEFAULT_N_CTX: u32 = 8192;

/// The window to size research batches to. Pure.
///
/// `reported` is the window the loaded model actually runs with
/// ([`AIProvider::context_window`]): for the embedded runtime that is the
/// `chat.n_ctx` setting after its clamps (KV cache that fits in RAM, the
/// model's trained window), which can be well below the setting. Before the
/// model has loaded there is none, and the setting — or the RAM tier the
/// runtime starts from — stands in; the HTTP backends run at their 8k default.
pub(crate) fn plan_n_ctx(
    reported: Option<u32>,
    provider: crate::ai::provider::ProviderType,
    n_ctx_override: u32,
    auto_tier: u32,
) -> u32 {
    if let Some(n) = reported.filter(|n| *n > 0) {
        return n;
    }
    match provider {
        crate::ai::provider::ProviderType::LlamaCpp if n_ctx_override > 0 => n_ctx_override,
        crate::ai::provider::ProviderType::LlamaCpp => auto_tier,
        _ => DEFAULT_N_CTX,
    }
}

/// Read the inputs of [`plan_n_ctx`]: the provider's live window, the
/// preferences and the machine. Call it once the model is loaded (after the
/// planner ran) so the live window is known.
pub(crate) fn resolve_n_ctx(db: &Database, provider: &dyn AIProvider) -> u32 {
    let n_ctx_override = db
        .get_preference("chat.n_ctx")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let auto_tier = crate::util::system::auto_n_ctx_tier(crate::util::system::total_ram_bytes());
    plan_n_ctx(
        provider.context_window(),
        provider.provider_type(),
        n_ctx_override,
        auto_tier,
    )
}

/// Where the measured speed of the last run is kept, for the next estimate.
const MS_PER_EMAIL_PREF: &str = "chat.research_ms_per_email";

// ── Gather ──────────────────────────────────────────────────────────────────

/// A safety net, not a product limit: far past any mailbox question the user
/// would confirm, and it keeps a runaway filter from loading the whole DB.
const GATHER_LIMIT: i32 = 50_000;
/// Vector candidates considered for a topic question before the band cut.
const SEMANTIC_POOL: usize = 1_000;
/// Keyword candidates for a topic question.
const KEYWORD_POOL: i32 = 1_000;
/// Similarity band below the best hit that still counts as on topic.
const SEMANTIC_BAND: f32 = 0.12;
/// `get_emails_by_ids` binds one parameter per id; stay far below SQLite's cap.
const ID_CHUNK: usize = 500;

/// A research run planned and gathered, before any reading: what the
/// estimate counts and what the run reads.
#[derive(Debug, Default)]
pub(crate) struct Prepared {
    pub plan: Option<SearchPlan>,
    /// Oldest first, so the notes — and the report — follow the timeline.
    pub email_ids: Vec<String>,
    pub planner_call: Option<LlmCallTrace>,
    /// One entry per search that ran, for the reasoning panel.
    pub gather_calls: Vec<ToolCallTrace>,
    pub search_hits: u32,
    pub semantic_hits: u32,
    pub gather_ms: i64,
    /// Every address the user sends from (see `Database::user_addresses`):
    /// who "I" is when reading, and every sender a filter on the user covers.
    pub user_addresses: Vec<String>,
    /// How the answer will be delivered — a list or count written in code,
    /// or a report.
    pub mode: ReportMode,
    pub mode_call: Option<LlmCallTrace>,
}

/// Everything planning and gathering read.
pub(crate) struct PrepareInput<'a> {
    pub db: &'a Arc<Database>,
    pub provider: &'a dyn AIProvider,
    pub account_id: &'a str,
    pub categories: &'a [String],
    pub question: &'a str,
    pub user_email: &'a str,
    pub today: &'a str,
}

/// Round index the gather searches carry in the trace: before the planner's
/// preseeded round (-1) and every map batch.
const GATHER_ROUND: i32 = -3;

/// Plan the question and gather every candidate.
pub(crate) async fn prepare(input: &PrepareInput<'_>) -> Prepared {
    let t = std::time::Instant::now();
    let (plan, planner_call) = plan_question(input).await;
    let (mode, mode_call) = classify_mode(input).await;
    let mut prepared = gather(input, plan).await;
    prepared.planner_call = planner_call;
    prepared.mode = mode;
    prepared.mode_call = mode_call;
    prepared.gather_ms = t.elapsed().as_millis() as i64;
    prepared
}

/// Longest the mode classifier may answer: `{"report": "analysis"}`.
const MODE_MAX_TOKENS: u32 = 24;
const MODE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// How the answer will be delivered (see `mode`). A missing prompt or a failed
/// call is logged and falls back to the report, which serves any question.
async fn classify_mode(input: &PrepareInput<'_>) -> (ReportMode, Option<LlmCallTrace>) {
    classify_question(input.db, input.provider, input.question).await
}

/// [`classify_mode`] for one question — what the app runs, shared with the
/// classifier's eval.
pub(crate) async fn classify_question(
    db: &Database,
    provider: &dyn AIProvider,
    question: &str,
) -> (ReportMode, Option<LlmCallTrace>) {
    let template = match crate::services::prompts::get_template(db, "chat.research_mode") {
        Ok(t) => t,
        Err(e) => {
            super::emit_log(
                "error",
                &format!("research: mode prompt unavailable ({e}); writing a report"),
            );
            return (ReportMode::Analysis, None);
        }
    };
    let (prefix, suffix) = mode::split_mode_prompt(&template, question);
    let (result, trace) = complete(
        provider,
        (&prefix, &suffix),
        call_options(MODE_MAX_TOKENS, 0.0, Some(mode::mode_shape())),
        MODE_TIMEOUT,
        "research_mode",
        -2,
    )
    .await;
    let mode = match result {
        Ok(reply) => mode::parse_mode(&reply.text),
        Err(e) => {
            super::emit_log(
                "error",
                &format!("research: choosing the answer's form failed ({e}); writing a report"),
            );
            ReportMode::Analysis
        }
    };
    super::emit_log("info", &format!("research: answer form {mode:?}"));
    (mode, Some(trace))
}

/// The query planner's filter for the question; any other verdict (defer, app
/// help, a form) leaves the question to be gathered by meaning.
async fn plan_question(input: &PrepareInput<'_>) -> (Option<SearchPlan>, Option<LlmCallTrace>) {
    let template = match crate::services::prompts::get_template(input.db, "chat.query_plan") {
        Ok(t) => t,
        Err(e) => {
            super::emit_log(
                "error",
                &format!("research: planner prompt unavailable ({e}); gathering by meaning"),
            );
            return (None, None);
        }
    };
    let glossary = crate::services::classification::TagGlossary::load(input.db);
    let catalog = crate::services::forms::registry::catalog(input.db);
    let t = std::time::Instant::now();
    let run = super::planner::plan_search(
        input.provider,
        &template,
        input.user_email,
        input.today,
        input.question,
        &glossary,
        None,
        &catalog,
    )
    .await;
    let latency = t.elapsed().as_millis() as i64;
    let plan = match run.plan {
        super::planner::Plan::Search(plan) => Some(*plan),
        _ => None,
    };
    let output = match &plan {
        Some(p) => serde_json::to_string(p).unwrap_or_default(),
        None => format!("no filter ({}) — gathering by meaning", run.outcome.as_str()),
    };
    let call = LlmCallTrace {
        kind: "planner".to_string(),
        round: -2,
        latency_ms: latency,
        tool_calls_requested: 0,
        failed: false,
        prompt_tokens: Some(run.prompt_tokens),
        prefill_ms: run.prefill_ms,
        cached_prompt_tokens: run.cached_prompt_tokens,
        prefix_plan: run.aux_plan.map(str::to_string),
        sys_cached_before: None,
        sys_cached_after: None,
        system_prefix_tokens: None,
        stable_tokens: None,
        dropped_front_tokens: None,
        input: None,
        output: Some(output),
    };
    (plan, Some(call))
}

/// Run the gather steps for `plan` and order the result oldest first.
pub(crate) async fn gather(input: &PrepareInput<'_>, plan: Option<SearchPlan>) -> Prepared {
    let mut prepared = Prepared {
        user_addresses: user_addresses(input.db, input.account_id, input.user_email),
        ..Default::default()
    };
    let mut lists: Vec<Vec<String>> = Vec::new();
    for step in for_every_user_address(plan_gather(plan.as_ref(), input.question), &prepared.user_addresses) {
        let t = std::time::Instant::now();
        let (name, arguments, ids) = match &step {
            GatherStep::Filter(p) | GatherStep::FilterUntagged(p) => {
                let ids = gather_filter(input, p);
                prepared.search_hits += ids.len() as u32;
                ("search_emails", filter_arguments(p), ids)
            }
            GatherStep::Semantic { query, keywords } => {
                let ids = gather_semantic(input, query, keywords.as_deref()).await;
                prepared.semantic_hits += ids.len() as u32;
                (
                    "semantic_search",
                    serde_json::json!({ "query": query, "keywords": keywords, "band": SEMANTIC_BAND }),
                    ids,
                )
            }
        };
        let preview = format!("{} emails", ids.len());
        prepared.gather_calls.push(ToolCallTrace {
            name: name.to_string(),
            round: GATHER_ROUND,
            arguments,
            result_chars: preview.len() as i32,
            result_preview: preview,
            elapsed_ms: t.elapsed().as_millis() as i64,
        });
        lists.push(ids);
    }
    prepared.email_ids = oldest_first(input.db, merge_candidates(&lists));
    prepared.plan = plan;
    prepared
}

/// Every address the user sends from; just the account's when the lookup
/// fails, which is logged.
fn user_addresses(db: &Database, account_id: &str, account_email: &str) -> Vec<String> {
    match db.user_addresses(account_id) {
        Ok(addrs) if !addrs.is_empty() => addrs,
        Ok(_) => vec![account_email.to_lowercase()],
        Err(e) => {
            super::emit_log("error", &format!("research: could not list the user's addresses: {e}"));
            vec![account_email.to_lowercase()]
        }
    }
}

/// The plan as the `search_emails` arguments it stands for, for the trace.
fn filter_arguments(plan: &SearchPlan) -> Value {
    let mut args = plan.clone().into_tool_call().function.arguments;
    if let Some(obj) = args.as_object_mut() {
        obj.remove("limit");
        obj.remove("include_bodies");
    }
    args
}

/// Every email matching the filter, each thread expanded to its messages (a
/// conversation's replies carry as much of the answer as its first email).
fn gather_filter(input: &PrepareInput<'_>, plan: &SearchPlan) -> Vec<String> {
    let parse = |d: &Option<String>| d.as_deref().and_then(|s| super::parse_iso_date_secs(s).ok());
    let since = parse(&plan.since);
    let until = parse(&plan.until);
    let tags: Vec<TagQuery> = [("intent", &plan.intent), ("topic", &plan.topic)]
        .into_iter()
        .filter_map(|(kind, v)| v.as_ref().map(|v| TagQuery::typed(kind, v.trim().to_lowercase())))
        .collect();
    // Same rule as `search_emails`: a named sender / recipient / subject is
    // not narrowed by the chat's category scope.
    let explicit = plan.from.is_some() || plan.to.is_some() || plan.with.is_some() || plan.subject.is_some();
    let categories = (!explicit && !input.categories.is_empty()).then_some(input.categories);
    // "With X": X's name plus the addresses X writes from, either direction.
    let participants: Vec<String> = plan
        .with
        .as_deref()
        .map(|w| crate::services::emails::resolve_participant(input.db, input.account_id, w))
        .unwrap_or_default();
    let matches = crate::services::emails::search_emails_filtered(
        input.db,
        input.account_id,
        plan.query.as_deref().unwrap_or(""),
        categories,
        plan.from.as_deref(),
        plan.to.as_deref(),
        plan.subject.as_deref(),
        since,
        until,
        (!tags.is_empty()).then_some(tags.as_slice()),
        GATHER_LIMIT,
        false,
        plan.unread == Some(true),
        (!participants.is_empty()).then_some(participants.as_slice()),
    );
    let matches = match matches {
        Ok(m) => m,
        Err(e) => {
            super::emit_log("error", &format!("research: filter search failed: {e}"));
            return Vec::new();
        }
    };
    let in_window = |ts: i64| since.is_none_or(|s| ts >= s) && until.is_none_or(|u| ts < u);
    let mut ids = Vec::new();
    let mut seen_threads = HashSet::new();
    for email in matches {
        if !seen_threads.insert(email.thread_id.clone()) {
            continue;
        }
        match input.db.get_thread(input.account_id, &email.thread_id) {
            Ok(thread) if !thread.is_empty() => {
                ids.extend(thread.into_iter().filter(|e| in_window(e.timestamp)).map(|e| e.id));
            }
            _ => ids.push(email.id),
        }
    }
    ids
}

/// Emails close in meaning to the question (within the band of the best hit),
/// plus every exact keyword hit.
async fn gather_semantic(input: &PrepareInput<'_>, query: &str, keywords: Option<&str>) -> Vec<String> {
    let categories = (!input.categories.is_empty()).then_some(input.categories);
    let mut ids = Vec::new();
    match input.provider.embed(query).await {
        Ok(emb) => {
            let req = crate::services::retrieval::VectorRequest {
                account_id: input.account_id,
                embedding: &emb.embedding,
                categories,
                limit: SEMANTIC_POOL,
            };
            match crate::services::retrieval::fetch_vector(input.db, req) {
                Ok(mut hits) => {
                    hits.sort_by(|a, b| b.1.total_cmp(&a.1));
                    let sims: Vec<f32> = hits.iter().map(|h| h.1).collect();
                    let keep = semantic_cutoff(&sims, SEMANTIC_BAND);
                    ids.extend(hits.into_iter().take(keep).map(|h| h.0));
                }
                Err(e) => super::emit_log("error", &format!("research: vector search failed: {e}")),
            }
        }
        Err(e) => super::emit_log("error", &format!("research: embedding the question failed: {e}")),
    }
    if let Some(keywords) = keywords.filter(|k| !k.trim().is_empty()) {
        let req = crate::services::retrieval::FtsRequest {
            account_id: input.account_id,
            query: keywords,
            categories,
            sender_email_eq: None,
            limit: KEYWORD_POOL,
        };
        match crate::services::retrieval::fetch_fts(input.db, req) {
            Ok(hits) => ids.extend(hits.into_iter().map(|h| h.0)),
            Err(e) => super::emit_log("error", &format!("research: keyword search failed: {e}")),
        }
    }
    merge_candidates(&[ids])
}

/// Load emails by id in chunks (SQLite binds one parameter per id).
fn load_emails(db: &Database, ids: &[String]) -> Vec<Email> {
    let mut out = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(ID_CHUNK) {
        match db.get_emails_by_ids(chunk) {
            Ok(emails) => out.extend(emails),
            Err(e) => super::emit_log(
                "error",
                &format!("research: loading {} emails failed: {e}", chunk.len()),
            ),
        }
    }
    out
}

fn oldest_first(db: &Database, ids: Vec<String>) -> Vec<String> {
    let mut emails = load_emails(db, &ids);
    emails.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then_with(|| a.id.cmp(&b.id)));
    emails.into_iter().map(|e| e.id).collect()
}

// ── Estimate ────────────────────────────────────────────────────────────────

/// Plan and gather, then report what the run would read and how long it would
/// take. The gathered set is kept (see `control`) so the confirmed run reads
/// exactly what was counted.
pub(crate) async fn estimate(input: &PrepareInput<'_>) -> ResearchEstimate {
    let prepared = prepare(input).await;
    // After the planner ran, so the model is loaded and reports its window.
    let budget = plan_research_budget(resolve_n_ctx(input.db, input.provider));
    let ms_per_email = input
        .db
        .get_preference(MS_PER_EMAIL_PREF)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok());
    // Batched exactly as the run will batch them: whole conversations, each
    // read for what its messages add.
    let docs = load_docs(input.db, &prepared.email_ids, &budget, &prepared.user_addresses);
    let emails: usize = docs.iter().map(|d| d.messages.len()).sum();
    let lens: Vec<usize> = docs.iter().map(ResearchDoc::rendered_len).collect();
    let batches = plan_batches(&lens, budget.batch_chars, budget.max_emails_per_batch).len();
    let seconds = plan_estimate(emails, ms_per_email);
    let filter = prepared.plan.as_ref().map(filter_arguments);
    let mode = prepared.mode;
    super::emit_log(
        "info",
        &format!("research: estimate {emails} emails, {batches} batches, ~{seconds}s, {mode:?}"),
    );
    ResearchEstimate {
        estimate_id: store_estimate(input.account_id, input.question, prepared),
        emails: emails as u32,
        batches: batches as u32,
        seconds,
        mode,
        filter,
    }
}

// ── Corrections ─────────────────────────────────────────────────────────────

/// The question a corrected research asks: the original question, and what the
/// user said was wrong with the answer — every step (planner, reading, report)
/// sees both. Pure.
pub(crate) fn compose_research_question(original: &str, reason: &str) -> String {
    format!(
        "{}\n\n(The user said the previous answer to this question was wrong: {})",
        original.trim(),
        reason.trim()
    )
}

/// The user question a rejected answer replied to: the last user message before
/// it in the conversation.
pub fn original_question(db: &Database, conversation_id: &str, rejected_message_id: &str) -> Option<String> {
    let messages = db.get_chat_messages(conversation_id).ok()?;
    let at = messages.iter().position(|m| m.id == rejected_message_id)?;
    messages[..at]
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
}

/// The research question for a turn: the question itself, or — when the turn
/// retries a rejected answer — the original question plus the correction.
pub fn research_question(
    db: &Database,
    conversation_id: &str,
    content: &str,
    correction: Option<&crate::models::ChatCorrection>,
) -> String {
    match correction.and_then(|c| {
        original_question(db, conversation_id, &c.rejected_message_id).map(|q| compose_research_question(&q, &c.reason))
    }) {
        Some(q) => q,
        None => content.to_string(),
    }
}

/// The Tauri/CLI entry point for an estimate: resolves the account's address,
/// the provider, the window and today's date, then [`estimate`]s.
pub async fn estimate_for_account(
    db: &Arc<Database>,
    account_id: &str,
    categories: &[String],
    question: &str,
) -> Result<ResearchEstimate> {
    let provider = crate::services::ai::AiService::load_provider(db)?;
    let user_email = db.get_account(account_id)?.map(|a| a.email).unwrap_or_default();
    let now = crate::services::clock::now_secs() + i64::from(crate::services::clock::utc_offset_secs());
    let today = chrono::DateTime::from_timestamp(now, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let input = PrepareInput {
        db,
        provider: provider.as_ref(),
        account_id,
        categories,
        question,
        user_email: &user_email,
        today: &today,
    };
    Ok(estimate(&input).await)
}

// ── Run ─────────────────────────────────────────────────────────────────────

/// One map call may not stall the turn forever; a batch that times out is
/// skipped and counted as failed.
const MAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
/// Condense and report calls write more.
const REDUCE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
/// Condense rounds before the notes are trimmed to fit instead. Each round
/// divides the notes by several times, so this is never reached in practice.
const MAX_CONDENSE_ROUNDS: usize = 4;

/// Everything a research run reads.
pub(crate) struct ResearchInput<'a> {
    pub db: &'a Arc<Database>,
    pub provider: &'a dyn AIProvider,
    pub question: &'a str,
    pub prepared: &'a Prepared,
    pub n_ctx: u32,
    pub language_instruction: &'a str,
    /// ISO code of the report language, for the full list's heading.
    pub language_code: &'a str,
    pub map_template: &'a str,
    pub condense_template: &'a str,
    pub reduce_template: &'a str,
    /// Raised by the chat's Cancel button: the run ends without a report.
    pub stop: &'a AtomicBool,
}

/// Where a research run is, for the progress indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResearchStage {
    Gathering,
    Reading,
    Condensing,
    Writing,
}

impl ResearchStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Gathering => "gathering",
            Self::Reading => "reading",
            Self::Condensing => "condensing",
            Self::Writing => "writing",
        }
    }
}

/// Matches shown in the progress while a research reads — enough to see
/// whether it is finding the right mail, few enough to scan.
pub(crate) const RECENT_MATCHES: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResearchProgress {
    pub stage: ResearchStage,
    pub batch: usize,
    pub batches: usize,
    pub emails_read: usize,
    pub emails_total: usize,
    /// Matches found so far.
    pub matches: usize,
    /// The latest [`RECENT_MATCHES`] of them, newest last — so the user can
    /// see early whether the run is on track and cancel it if not.
    pub recent: Vec<Match>,
}

/// What a research run produced.
#[derive(Debug, Default)]
pub(crate) struct ResearchRun {
    /// The report. `None` when the reduce call failed; `error` says why.
    pub answer: Option<String>,
    pub error: Option<String>,
    /// Every email that was read — the link allowlist.
    pub analyzed: Vec<String>,
    /// The emails a finding cites, in finding order — the answer's sources
    /// when it links none itself.
    pub relevant: Vec<String>,
    pub trace: ResearchTrace,
    pub llm_calls: Vec<LlmCallTrace>,
}

fn call_trace(
    kind: &str,
    round: i32,
    latency_ms: i64,
    prompt: (&str, &str),
    result: Option<&CompletionResult>,
) -> LlmCallTrace {
    LlmCallTrace {
        kind: kind.to_string(),
        round,
        latency_ms,
        tool_calls_requested: 0,
        failed: result.is_none(),
        prompt_tokens: result.map(|r| r.prompt_tokens),
        prefill_ms: result.and_then(|r| r.prefill_ms),
        cached_prompt_tokens: result.and_then(|r| r.cached_prompt_tokens),
        prefix_plan: result.and_then(|r| r.aux_plan).map(str::to_string),
        sys_cached_before: None,
        sys_cached_after: None,
        system_prefix_tokens: None,
        stable_tokens: None,
        dropped_front_tokens: None,
        // Prompts carry mail bodies: captured in dev builds only, like the
        // tool rounds' prompts.
        input: cfg!(debug_assertions).then(|| format!("{}{}", prompt.0, prompt.1)),
        output: result.map(|r| r.text.clone()),
    }
}

/// One completion with a timeout, traced.
/// Options for one research call: no thinking, and — for the reading and
/// condense steps — the JSON shape the reply must take.
fn call_options(
    max_tokens: u32,
    temperature: f64,
    shape: Option<crate::ai::json_shape::JsonShape>,
) -> CompletionOptions {
    CompletionOptions {
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
        think: Some(false),
        json_shape: shape,
    }
}

async fn complete(
    provider: &dyn AIProvider,
    prompt: (&str, &str),
    opts: CompletionOptions,
    timeout: std::time::Duration,
    kind: &str,
    round: i32,
) -> (std::result::Result<CompletionResult, String>, LlmCallTrace) {
    let t = std::time::Instant::now();
    let result = tokio::time::timeout(timeout, provider.complete_with_prefix(prompt.0, prompt.1, opts)).await;
    let latency = t.elapsed().as_millis() as i64;
    let result = match result {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("timed out after {}s", timeout.as_secs())),
    };
    let trace = call_trace(kind, round, latency, prompt, result.as_ref().ok());
    (result, trace)
}

/// Longest a single conversation may run in a batch, as a multiple of the
/// per-email budget: long threads get more room than one email, not unbounded.
const CONVERSATION_EMAILS_BUDGET: usize = 4;

/// Load the gathered emails as conversations — oldest first, by the thread's
/// first gathered email — each read once through the shared thread reader:
/// every message's new content only, one budget for the conversation.
fn load_docs(
    db: &Database,
    ids: &[String],
    budget: &plan::ResearchBudget,
    user_addresses: &[String],
) -> Vec<ResearchDoc> {
    use crate::services::thread_reader::{read_thread, ReadOptions, ThreadMessage};
    let emails = load_emails(db, ids);
    let by_id: HashMap<&str, &Email> = emails.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut order: Vec<&str> = Vec::new();
    let mut threads: HashMap<&str, Vec<&Email>> = HashMap::new();
    for email in ids.iter().filter_map(|id| by_id.get(id.as_str())) {
        threads
            .entry(email.thread_id.as_str())
            .or_insert_with(|| {
                order.push(email.thread_id.as_str());
                Vec::new()
            })
            .push(email);
    }
    let date = |ts: i64| {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    };
    order
        .into_iter()
        .filter_map(|thread_id| {
            let mut members = threads.remove(thread_id)?;
            members.sort_by_key(|e| e.timestamp);
            let messages: Vec<ThreadMessage> = members
                .iter()
                .map(|e| {
                    let body = db.get_email_body(&e.id).unwrap_or_else(|err| {
                        super::emit_log("debug", &format!("research: body of {} unavailable: {err}", e.id));
                        String::new()
                    });
                    ThreadMessage::from_email(e, body)
                })
                .collect();
            let cap =
                (budget.chars_per_email * messages.len().clamp(1, CONVERSATION_EMAILS_BUDGET)).min(budget.batch_chars);
            let read = read_thread(&messages, &ReadOptions::budget(cap));
            Some(ResearchDoc {
                thread_id: thread_id.to_string(),
                subject: members.first().map(|e| e.subject.clone()).unwrap_or_default(),
                messages: read
                    .messages
                    .into_iter()
                    .map(|m| DocMessage {
                        from: participant(&m.sender, &m.sender_email, user_addresses),
                        from_user: plan::is_user_address(&m.sender_email, user_addresses),
                        to: by_id
                            .get(m.id.as_str())
                            .map(|e| recipients(&e.recipients, user_addresses))
                            .unwrap_or_default(),
                        id: m.id,
                        date: date(m.timestamp),
                        text: if m.text.is_empty() {
                            "(no new content)".to_string()
                        } else {
                            m.text
                        },
                    })
                    .collect(),
            })
        })
        .collect()
}

/// Map → condense → reduce over a prepared set. Never fails the turn on its
/// own: a batch that errors is logged and skipped, and only a failed report
/// comes back as `answer: None` for the caller to surface.
pub(crate) async fn run_research(
    input: ResearchInput<'_>,
    on_progress: &(dyn Fn(ResearchProgress) + Sync),
) -> ResearchRun {
    let budget = plan_research_budget(input.n_ctx);
    let prepared = input.prepared;
    let mut run = ResearchRun {
        trace: ResearchTrace {
            mode: prepared.mode,
            n_ctx: budget.n_ctx,
            planned_emails: prepared.email_ids.len() as u32,
            search_hits: prepared.search_hits,
            semantic_hits: prepared.semantic_hits,
            gather_ms: prepared.gather_ms,
            ..Default::default()
        },
        ..Default::default()
    };
    let progress = |stage, batch, batches, emails_read, emails_total, found: &[Match]| {
        on_progress(ResearchProgress {
            stage,
            batch,
            batches,
            emails_read,
            emails_total,
            matches: found.len(),
            recent: found[found.len().saturating_sub(RECENT_MATCHES)..].to_vec(),
        })
    };
    // Matches as the batches find them, for the progress.
    let mut found: Vec<Match> = Vec::new();
    // Every reading verdict, held to the question's direction.
    let mut findings: Vec<Finding> = Vec::new();

    // ── Map ──
    progress(ResearchStage::Reading, 0, 0, 0, prepared.email_ids.len(), &found);
    let docs = load_docs(input.db, &prepared.email_ids, &budget, &prepared.user_addresses);
    let direction = plan::plan_direction(prepared.plan.as_ref(), &prepared.user_addresses);
    // Emails read after the first `n` conversations.
    let emails_in = |n: usize| docs[..n].iter().map(|d| d.messages.len()).sum::<usize>();
    let total_emails = emails_in(docs.len());
    let lens: Vec<usize> = docs.iter().map(ResearchDoc::rendered_len).collect();
    let batches = plan_batches(&lens, budget.batch_chars, budget.max_emails_per_batch);
    let t_map = std::time::Instant::now();
    // The report's notes, one list per batch.
    let mut notes: Vec<Vec<Note>> = Vec::with_capacity(batches.len());
    // Conversations read so far.
    let mut read = 0;
    progress(ResearchStage::Reading, 0, batches.len(), 0, total_emails, &found);
    for (i, range) in batches.iter().enumerate() {
        if input.stop.load(Ordering::Relaxed) {
            run.trace.stopped = true;
            super::emit_log(
                "info",
                &format!(
                    "research: cancelled by the user after {} of {total_emails} emails",
                    emails_in(read)
                ),
            );
            break;
        }
        let labels = BatchLabels::new(range.start, &docs[range.clone()]);
        let (prefix, suffix) = split_map_prompt(input.map_template, input.question, &labels.render(), direction);
        let (result, trace) = complete(
            input.provider,
            (&prefix, &suffix),
            call_options(MAP_MAX_TOKENS, 0.0, Some(labels.shape())),
            MAP_TIMEOUT,
            "research_map",
            i as i32,
        )
        .await;
        run.llm_calls.push(trace);
        // A reply cut at its output limit is incomplete JSON: the batch fails
        // loudly rather than losing its tail without a trace.
        let parsed = result.and_then(|reply| match reply.truncated {
            true => Err(format!("the reply stopped at its {MAP_MAX_TOKENS}-token limit")),
            false => labels.parse(&reply.text),
        });
        match parsed {
            Ok(batch_findings) => {
                // Held to who wrote each email before anything reads it.
                let held = enforce_direction(batch_findings, &docs, direction);
                super::emit_log(
                    "debug",
                    &format!("research: batch {}/{} → {} findings", i + 1, batches.len(), held.len()),
                );
                notes.push(held.iter().map(Note::from).collect());
                findings.extend(held);
                found = collect_matches(&docs[..range.end], &findings, direction);
            }
            Err(e) => {
                super::emit_log(
                    "error",
                    &format!("research: batch {}/{} failed: {e}", i + 1, batches.len()),
                );
                run.trace.failed_batches += 1;
            }
        }
        read = range.end;
        run.trace.batches += 1;
        super::emit_log(
            "info",
            &format!(
                "research: read {}/{total_emails} emails (batch {}/{}), {} conversations matched",
                emails_in(read),
                i + 1,
                batches.len(),
                found.len()
            ),
        );
        progress(
            ResearchStage::Reading,
            i + 1,
            batches.len(),
            emails_in(read),
            total_emails,
            &found,
        );
    }
    run.trace.map_ms = t_map.elapsed().as_millis() as i64;
    run.analyzed = docs[..read].iter().flat_map(|d| d.ids().cloned()).collect();
    run.trace.findings = findings.len() as u32;
    // The matches come from the reading verdicts, before any condense round:
    // the list and the counts must not depend on how the notes were merged.
    let matches = collect_matches(&docs[..read], &findings, direction);
    // One source per conversation: the answer never lists two emails of one
    // thread.
    run.relevant = matches.iter().map(|m| m.id.clone()).collect();
    let emails_read = emails_in(read);
    run.trace.emails_analyzed = emails_read as u32;
    run.trace.relevant_emails = matches.iter().map(|m| m.emails).sum::<usize>() as u32;
    // Cancelled: no condense, no report — say how far it got and stop.
    if run.trace.stopped {
        run.answer = Some(cancelled_note(input.language_code, emails_read, total_emails));
        return run;
    }

    // A list or a count is written in code from the matches: no condense, no
    // report call.
    if prepared.mode != ReportMode::Analysis {
        run.answer = Some(mode::list_answer(
            prepared.mode,
            &matches,
            emails_read,
            run.trace.batches as usize,
            run.trace.failed_batches as usize,
            input.language_code,
        ));
        return run;
    }

    // ── Condense ──
    let t_condense = std::time::Instant::now();
    for round in 0..MAX_CONDENSE_ROUNDS {
        let lens: Vec<usize> = notes.iter().map(|b| notes_len(b)).collect();
        let Some(groups) = plan_condense_groups(&lens, budget.notes_chars) else {
            break;
        };
        if groups.len() == notes.len() && notes.len() == 1 {
            break; // one group that alone overflows: assemble_notes trims it
        }
        let mut merged = Vec::with_capacity(groups.len());
        for (j, group) in groups.iter().enumerate() {
            progress(
                ResearchStage::Condensing,
                j,
                groups.len(),
                emails_read,
                total_emails,
                &found,
            );
            let group_notes: Vec<Note> = notes[group.clone()].iter().flatten().cloned().collect();
            let (prefix, suffix) = split_condense_prompt(
                input.condense_template,
                input.question,
                &render_condense_input(&group_notes),
            );
            let (result, trace) = complete(
                input.provider,
                (&prefix, &suffix),
                call_options(CONDENSE_MAX_TOKENS, 0.0, Some(condense_shape(group_notes.len()))),
                REDUCE_TIMEOUT,
                "research_condense",
                (round * 1000 + j) as i32,
            )
            .await;
            run.llm_calls.push(trace);
            run.trace.condense_calls += 1;
            let condensed = result.and_then(|reply| match reply.truncated {
                true => Err(format!("the reply stopped at its {CONDENSE_MAX_TOKENS}-token limit")),
                false => parse_condensed(&reply.text, &group_notes),
            });
            match condensed {
                // Merged notes carry every conversation of what they merged.
                Ok(parsed) if !parsed.is_empty() => merged.push(parsed),
                Ok(_) => merged.extend(notes[group.clone()].iter().cloned()),
                Err(e) => {
                    super::emit_log("error", &format!("research: condensing notes failed: {e}"));
                    merged.extend(notes[group.clone()].iter().cloned());
                }
            }
        }
        notes = merged;
    }
    run.trace.condense_ms = t_condense.elapsed().as_millis() as i64;

    // ── Reduce ──
    progress(
        ResearchStage::Writing,
        run.trace.batches as usize,
        batches.len(),
        emails_read,
        total_emails,
        &found,
    );
    // The conversations the notes cover, numbered for the report to cite.
    let order = number_conversations(&notes);
    let notes_block = if order.is_empty() {
        "(no relevant findings in the emails read)".to_string()
    } else {
        render_notes(&notes, &order, &docs, budget.notes_chars)
    };
    let coverage = coverage_line(
        emails_read,
        total_emails,
        run.trace.relevant_emails as usize,
        run.trace.batches as usize,
        run.trace.failed_batches as usize,
    );
    let facts = report_facts(&matches);
    let (prefix, suffix) = split_reduce_prompt(
        input.reduce_template,
        input.language_instruction,
        input.question,
        &coverage,
        &facts,
        &notes_block,
        direction,
    );
    // The report may use what its actual prompt leaves free in the window.
    let report_tokens = plan::plan_report_tokens(&budget, prefix.chars().count() + suffix.chars().count());
    let t_reduce = std::time::Instant::now();
    let (result, trace) = complete(
        input.provider,
        (&prefix, &suffix),
        call_options(report_tokens, 0.2, None),
        REDUCE_TIMEOUT,
        "research_reduce",
        -1,
    )
    .await;
    run.llm_calls.push(trace);
    run.trace.reduce_ms = t_reduce.elapsed().as_millis() as i64;
    match result {
        Ok(reply) if !reply.text.trim().is_empty() => {
            // The report cites conversations by number; code writes the links.
            let targets = citation_targets(&order, &docs, &matches, &findings);
            let prose = render_citations(reply.text.trim(), &targets);
            // `truncated`: the provider says the report stopped at its output
            // limit — the list of every match then closes the answer.
            run.answer = Some(finish_report(&prose, reply.truncated, &matches, input.language_code));
        }
        // The report failed but the matches stand on their own: list them.
        _ if !matches.is_empty() => run.answer = Some(render_match_list(&matches, input.language_code)),
        Ok(_) => run.error = Some("the research report came back empty".to_string()),
        Err(e) => run.error = Some(format!("writing the research report failed: {e}")),
    }

    // The next estimate uses what this machine actually took. Report runs
    // only (a list returns before this): a list's reading-only pace would make
    // the next report's estimate short.
    if emails_read > 0 {
        let ms = (run.trace.map_ms + run.trace.condense_ms + run.trace.reduce_ms) as u64 / emails_read as u64;
        if let Err(e) = input.db.set_preference(MS_PER_EMAIL_PREF, &ms.to_string()) {
            super::emit_log("debug", &format!("research: could not save the measured speed: {e}"));
        }
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n_ctx_is_the_window_the_runtime_reports_when_it_reports_one() {
        use crate::ai::provider::ProviderType;
        // The embedded runtime clamps the setting to what the KV cache fits
        // and the model was trained on; that clamped window is the truth.
        assert_eq!(plan_n_ctx(Some(15360), ProviderType::LlamaCpp, 32768, 32768), 15360);
        assert_eq!(plan_n_ctx(Some(4096), ProviderType::Ollama, 0, 16384), 4096);
    }

    #[test]
    fn before_the_model_loads_n_ctx_follows_the_setting_then_the_ram_tier() {
        use crate::ai::provider::ProviderType;
        assert_eq!(plan_n_ctx(None, ProviderType::LlamaCpp, 12288, 32768), 12288);
        assert_eq!(plan_n_ctx(None, ProviderType::LlamaCpp, 0, 16384), 16384);
        assert_eq!(plan_n_ctx(None, ProviderType::Ollama, 32768, 32768), DEFAULT_N_CTX);
        assert_eq!(
            plan_n_ctx(Some(0), ProviderType::LlamaCpp, 0, 16384),
            16384,
            "0 = not known yet"
        );
    }

    // ── executor (fake provider + in-memory DB) ──

    /// `n` emails from one supplier, one per thread, plus a reply in the first
    /// thread from someone else.
    fn seed(db: &Database, n: usize) {
        use rusqlite::params;
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
             VALUES ('acct', 'gmail', 'me@example.com', 'Me', 0)",
            [],
        )
        .unwrap();
        let insert = |id: String, thread: String, sender: &str, subject: String, ts: i64| {
            let domain = sender.rsplit_once('@').map(|(_, d)| d.to_string()).unwrap_or_default();
            conn.execute(
                "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                 VALUES (?1,'acct',?2,?3,?4,?4,?5,'[]','[]','snip',?6,0,'primary',0)",
                params![id, thread, subject, sender, domain, ts],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1,?2,?3,'Invoice due')",
                params![id, subject, sender],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO email_bodies(email_id, body) VALUES (?1, ?2)",
                params![id, format!("{subject}: 100 EUR due on Friday.")],
            )
            .unwrap();
        };
        for i in 0..n {
            insert(
                format!("e{i:02}"),
                format!("t{i:02}"),
                "billing@supplier.example",
                format!("Invoice {i}"),
                1_780_000_000 + i as i64,
            );
        }
        insert(
            "r00".into(),
            "t00".into(),
            "me@example.com",
            "Re: Invoice 0".into(),
            1_790_000_000,
        );
    }

    fn prepare_input<'a>(
        db: &'a Arc<Database>,
        provider: &'a dyn AIProvider,
        categories: &'a [String],
    ) -> PrepareInput<'a> {
        PrepareInput {
            db,
            provider,
            account_id: "acct",
            categories,
            question: "¿Qué facturas me ha enviado el proveedor?",
            user_email: "me@example.com",
            today: "2026-09-24",
        }
    }

    fn run_input<'a>(
        db: &'a Arc<Database>,
        provider: &'a dyn AIProvider,
        prepared: &'a Prepared,
        n_ctx: u32,
        stop: &'a AtomicBool,
    ) -> ResearchInput<'a> {
        use crate::services::prompts::defaults as d;
        ResearchInput {
            db,
            provider,
            question: "¿Qué facturas me ha enviado el proveedor?",
            prepared,
            n_ctx,
            language_instruction: "Reply in Spanish.",
            language_code: "es",
            map_template: d::CHAT_RESEARCH_MAP,
            condense_template: d::CHAT_RESEARCH_CONDENSE,
            reduce_template: d::CHAT_RESEARCH_REDUCE,
            stop,
        }
    }

    fn supplier_plan() -> SearchPlan {
        SearchPlan {
            from: Some("billing@supplier.example".into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn gather_reads_every_match_with_whole_threads_oldest_first() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 60);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;

        // 60 matches — past the 25-row page and the old 100 cap is irrelevant
        // here, the point is no page stops it — plus the reply in thread t00.
        assert_eq!(prepared.email_ids.len(), 61);
        assert_eq!(prepared.email_ids.first().map(String::as_str), Some("e00"));
        assert_eq!(
            prepared.email_ids.last().map(String::as_str),
            Some("r00"),
            "oldest first"
        );
        assert_eq!(prepared.gather_calls.len(), 1);
        assert_eq!(prepared.gather_calls[0].name, "search_emails");
        assert_eq!(prepared.gather_calls[0].round, GATHER_ROUND);
        assert_eq!(prepared.gather_calls[0].result_preview, "61 emails");
    }

    /// Emails per conversation, per batch, exactly as `run_research` will
    /// batch `prepared` on a window of `n_ctx`.
    fn batches_of(db: &Database, prepared: &Prepared, n_ctx: u32) -> Vec<Vec<usize>> {
        let budget = plan_research_budget(n_ctx);
        let docs = load_docs(db, &prepared.email_ids, &budget, &prepared.user_addresses);
        let lens: Vec<usize> = docs.iter().map(ResearchDoc::rendered_len).collect();
        plan_batches(&lens, budget.batch_chars, budget.max_emails_per_batch)
            .into_iter()
            .map(|range| docs[range].iter().map(|d| d.messages.len()).collect())
            .collect()
    }

    /// A reading reply giving every conversation of a batch a match verdict
    /// citing all its emails, by the batch's labels.
    fn all_match(emails_per_conversation: &[usize], text: &str) -> String {
        let mut next_email = 1;
        let entries: Vec<serde_json::Value> = emails_per_conversation
            .iter()
            .enumerate()
            .map(|(c, n)| {
                let emails: Vec<String> = (next_email..next_email + n).map(|e| format!("E{e}")).collect();
                next_email += n;
                serde_json::json!({"conversation": format!("C{}", c + 1), "text": text, "tag": "match", "emails": emails})
            })
            .collect();
        serde_json::json!({ "findings": entries }).to_string()
    }

    const NO_FINDINGS: &str = r#"{"findings":[]}"#;

    #[tokio::test]
    async fn research_reads_in_batches_and_reports_from_the_notes() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 29);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        // 30 emails → three batches. In the first, t00 (e00 and its reply) is
        // C1 with E1–E2, so Invoice 5 is C6 with E7; the others find nothing.
        assert_eq!(batches_of(&db, &prepared, 16384).len(), 3);
        provider.push_completion(
            r#"{"findings":[{"conversation":"C6","text":"Invoice 5 for 100 EUR is due Friday","tag":"match","emails":["E7"]}]}"#,
        );
        provider.push_completion(NO_FINDINGS);
        provider.push_completion(NO_FINDINGS);
        // The report cites the conversation by number; code writes the link.
        provider.push_completion("Tienes 29 facturas; ver [1].");
        let stop = AtomicBool::new(false);
        let run = run_research(run_input(&db, &provider, &prepared, 16384, &stop), &|_| {}).await;

        assert_eq!(run.analyzed.len(), 30);
        assert_eq!(run.trace.batches, 3);
        assert_eq!(run.relevant, vec!["e05".to_string()]);
        assert_eq!(run.trace.condense_calls, 0);
        assert!(!run.trace.stopped);
        assert_eq!(
            run.answer.as_deref(),
            Some("Tienes 29 facturas; ver [Invoice 5](email://e05).")
        );
        assert_eq!(
            run.llm_calls.iter().map(|c| c.kind.as_str()).collect::<Vec<_>>(),
            ["research_map", "research_map", "research_map", "research_reduce"]
        );
        let calls = provider.prefix_completion_calls();
        assert!(
            calls[..3].iter().all(|(prefix, _)| prefix == &calls[0].0),
            "map prefix is invariant"
        );
        assert!(
            db.get_preference(MS_PER_EMAIL_PREF).unwrap().is_some(),
            "speed recorded"
        );
    }

    #[tokio::test]
    async fn reading_asks_for_its_json_shape_and_the_report_for_free_text() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 4);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        provider.push_completion(NO_FINDINGS);
        provider.push_completion("Nada relevante.");
        let stop = AtomicBool::new(false);
        run_research(run_input(&db, &provider, &prepared, 16384, &stop), &|_| {}).await;

        let shapes = provider.completion_shapes();
        assert_eq!(shapes.len(), 2);
        let reading = shapes[0]
            .as_ref()
            .expect("the reading step asks for a shape")
            .to_json_schema();
        assert_eq!(
            reading["properties"]["findings"]["items"]["properties"]["conversation"]["enum"],
            serde_json::json!(["C1", "C2", "C3", "C4"]),
            "only this batch's conversations"
        );
        assert!(shapes[1].is_none(), "the report is prose");
    }

    #[tokio::test]
    async fn a_report_the_provider_cut_off_ends_with_every_match() {
        // The provider says the report stopped at its output limit: whatever
        // it had not reached is lost, so the list carries every match.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 29);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        provider.push_completion(
            r#"{"findings":[{"conversation":"C6","text":"Invoice 5 for 100 EUR is due Friday","tag":"match","emails":["E7"]}]}"#,
        );
        provider.push_completion(NO_FINDINGS);
        provider.push_completion(NO_FINDINGS);
        provider.push_truncated_completion("Tienes una factura pendiente.\n- Invoice 5 vence el vier");
        let stop = AtomicBool::new(false);
        let run = run_research(run_input(&db, &provider, &prepared, 16384, &stop), &|_| {}).await;

        let answer = run.answer.expect("an answer");
        assert!(
            answer.starts_with("Tienes una factura pendiente.\n\n### Lista completa (1)"),
            "{answer}"
        );
        assert!(!answer.contains("vence el vier"), "the broken line goes: {answer}");
    }

    #[tokio::test]
    async fn a_list_question_is_answered_in_code_without_a_report_call() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 24);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let mut prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        prepared.mode = ReportMode::List;
        assert_eq!(prepared.email_ids.len(), 25);
        // Every email is a match: 25 emails in 24 conversations (the reply
        // shares e00's thread), read 10 conversations per batch.
        for batch in batches_of(&db, &prepared, 16384) {
            provider.push_completion(all_match(&batch, "Invoice request"));
        }
        let stop = AtomicBool::new(false);
        let run = run_research(run_input(&db, &provider, &prepared, 16384, &stop), &|_| {}).await;

        assert!(
            run.llm_calls.iter().all(|c| c.kind == "research_map"),
            "reading only — no condense, no report"
        );
        assert_eq!(run.trace.mode, ReportMode::List);
        let answer = run.answer.expect("an answer");
        // 25 emails, but the reply in thread t00 is the same conversation:
        // 24 entries, the first one saying it holds two emails.
        assert!(
            answer.starts_with("24 conversaciones (25 correos) responden a tu pregunta.\n\n### Lista completa (24)"),
            "{answer}"
        );
        assert!(answer.contains("\n24. "), "every conversation is listed: {answer}");
        assert!(answer.contains("(2 correos)"), "{answer}");
        assert_eq!(run.trace.relevant_emails, 25);
        assert!(
            db.get_preference(MS_PER_EMAIL_PREF).unwrap().is_none(),
            "a list's reading-only pace does not set the report estimate"
        );
    }

    #[tokio::test]
    async fn the_estimate_chooses_the_answer_form_and_keeps_it_for_the_run() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 4);
        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.push_completion(r#"{"from": "billing@supplier.example"}"#);
        provider.push_completion(r#"{"report": "list"}"#);
        let categories: Vec<String> = Vec::new();
        let input = prepare_input(&db, &provider, &categories);
        let est = estimate(&input).await;
        assert_eq!(est.mode, ReportMode::List);
        let prepared = take_estimate(&est.estimate_id, "acct", input.question).expect("kept for the run");
        assert_eq!(prepared.mode, ReportMode::List);
        assert_eq!(
            prepared.mode_call.as_ref().map(|c| c.kind.as_str()),
            Some("research_mode")
        );
        let shapes = provider.completion_shapes();
        assert!(shapes[1].is_some(), "the classifier's reply shape is enforced");
    }

    #[tokio::test]
    async fn a_classifier_that_fails_leaves_the_report() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 4);
        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.push_completion(r#"{"from": "billing@supplier.example"}"#);
        provider.push_completion("not json");
        let categories: Vec<String> = Vec::new();
        let prepared = prepare(&prepare_input(&db, &provider, &categories)).await;
        assert_eq!(prepared.mode, ReportMode::Analysis);
    }

    #[tokio::test]
    async fn progress_shows_the_latest_matches_while_reading() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 29);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        // Every email matches: 30 emails in 29 conversations, more than the
        // 5 shown.
        for batch in batches_of(&db, &prepared, 16384) {
            provider.push_completion(all_match(&batch, "due Friday"));
        }
        provider.push_completion("Informe.");
        let stop = AtomicBool::new(false);
        let events = std::sync::Mutex::new(Vec::new());
        run_research(run_input(&db, &provider, &prepared, 16384, &stop), &|p| {
            events.lock().unwrap().push(p)
        })
        .await;

        let events = events.into_inner().unwrap();
        let after_first = events
            .iter()
            .find(|e| e.stage == ResearchStage::Reading && e.batch == 1)
            .expect("progress after the first batch");
        assert_eq!(after_first.matches, 10);
        assert_eq!(after_first.recent.len(), RECENT_MATCHES);
        assert_eq!(
            after_first.recent.last().map(|m| m.id.as_str()),
            Some("e09"),
            "newest last"
        );
        assert_eq!(after_first.recent[0].finding, "due Friday");
        let last = events
            .iter()
            .rfind(|e| e.stage == ResearchStage::Reading)
            .expect("reading events");
        // 30 emails in 29 conversations (e00 and its reply are one).
        assert_eq!(last.matches, 29);
    }

    #[tokio::test]
    async fn cancel_ends_the_run_without_a_report() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 29);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        provider.push_completion(
            r#"{"findings":[{"conversation":"C1","text":"Invoice 0","tag":"match","emails":["E1"]}]}"#,
        );
        let stop = AtomicBool::new(false);
        // Cancel once the first batch has been read.
        let on_progress = |p: ResearchProgress| {
            if p.stage == ResearchStage::Reading && p.batch == 1 {
                stop.store(true, Ordering::Relaxed);
            }
        };
        let run = run_research(run_input(&db, &provider, &prepared, 16384, &stop), &on_progress).await;

        assert!(run.trace.stopped);
        assert_eq!(run.trace.batches, 1);
        // The first batch is 10 conversations; one holds e00 and its reply.
        assert_eq!(run.trace.emails_analyzed, 11);
        // No condense, no report: only the one map call ran.
        assert_eq!(provider.prefix_completion_calls().len(), 1);
        assert!(!run.llm_calls.iter().any(|c| c.kind == "research_reduce"));
        let answer = run.answer.expect("a cancellation note");
        assert!(answer.contains("11") && answer.contains("30"), "{answer}");
    }

    #[tokio::test]
    async fn notes_that_overflow_the_report_prompt_are_condensed_first() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 59);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        // A tiny window: every batch's notes alone nearly fill the report
        // prompt, so the six batches must be merged before the report.
        let budget = plan_research_budget(4096);
        let batches = batches_of(&db, &prepared, 4096);
        for batch in &batches {
            // A long verdict on the batch's first conversation.
            provider.push_completion(all_match(&batch[..1], &"x".repeat(budget.notes_chars / 2)));
        }
        for _ in 0..batches.len() {
            provider.push_completion(r#"{"notes":[{"text":"merged","from":["N1"]}]}"#);
        }
        provider.push_completion("Informe [1].");
        let stop = AtomicBool::new(false);
        let run = run_research(run_input(&db, &provider, &prepared, 4096, &stop), &|_| {}).await;

        assert!(run.trace.condense_calls > 0, "{:?}", run.trace);
        assert!(run.llm_calls.iter().any(|c| c.kind == "research_condense"));
        assert_eq!(run.llm_calls.last().map(|c| c.kind.as_str()), Some("research_reduce"));
        assert_eq!(run.answer.as_deref(), Some("Informe [Invoice 0](email://e00)."));
    }

    #[tokio::test]
    async fn a_failing_provider_leaves_no_answer_but_does_not_panic() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 4);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        provider.fail_completions(Some("model crashed"));
        let stop = AtomicBool::new(false);
        let run = run_research(run_input(&db, &provider, &prepared, 16384, &stop), &|_| {}).await;
        assert_eq!(run.trace.failed_batches, 1);
        assert!(run.answer.is_none());
        assert!(
            run.error.as_deref().unwrap_or("").contains("model crashed"),
            "{:?}",
            run.error
        );
    }

    #[test]
    fn a_corrected_research_asks_the_original_question_with_the_correction() {
        let q = compose_research_question("¿cuántos presupuestos envié?", "faltan los de marzo");
        assert!(q.starts_with("¿cuántos presupuestos envié?"), "{q}");
        assert!(q.contains("faltan los de marzo"), "{q}");
    }

    #[test]
    fn the_original_question_is_the_user_message_before_the_rejected_answer() {
        let db = Database::new_for_testing().expect("test db");
        db.connection()
            .execute(
                "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acct', 'gmail', 'me@example.com', 'Me', 0)",
                [],
            )
            .unwrap();
        let conv = db.create_chat_conversation("acct", "t").expect("conversation");
        db.insert_chat_message(&conv.id, "user", "first question", None)
            .unwrap();
        db.insert_chat_message(&conv.id, "assistant", "first answer", None)
            .unwrap();
        db.insert_chat_message(&conv.id, "user", "research question", None)
            .unwrap();
        let rejected = db
            .insert_chat_message(&conv.id, "assistant", "wrong report", None)
            .unwrap();
        assert_eq!(
            original_question(&db, &conv.id, &rejected.id).as_deref(),
            Some("research question")
        );
        assert_eq!(original_question(&db, &conv.id, "missing"), None);
    }

    #[tokio::test]
    async fn an_estimate_counts_the_gathered_set_and_keeps_it_for_the_run() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 40);
        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.push_completion(r#"{"from": "billing@supplier.example"}"#);
        let categories: Vec<String> = Vec::new();
        let input = prepare_input(&db, &provider, &categories);
        let est = estimate(&input).await;
        assert_eq!(est.emails, 41);
        assert_eq!(est.batches, 4, "40 conversations, ten per batch");
        assert!(est.seconds > 0);
        assert_eq!(
            est.filter.as_ref().and_then(|f| f["from"].as_str()),
            Some("billing@supplier.example")
        );
        let prepared = take_estimate(&est.estimate_id, "acct", input.question).expect("kept for the run");
        assert_eq!(prepared.email_ids.len(), 41);
        assert!(prepared.planner_call.is_some());
    }

    #[tokio::test]
    async fn an_estimate_plans_batches_per_conversation_like_the_run() {
        // One long thread is one conversation: the run reads it in a single
        // batch, so the estimate must not count a batch per ten emails.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 0);
        {
            let conn = db.connection();
            for i in 0..30 {
                conn.execute(
                    "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                     VALUES (?1,'acct','big','Invoice run','billing@supplier.example',
                             'billing@supplier.example','supplier.example','[]','[]','snip',?2,0,'primary',0)",
                    rusqlite::params![format!("b{i:02}"), 1_780_000_000 + i as i64],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO email_bodies(email_id, body) VALUES (?1, ?2)",
                    rusqlite::params![format!("b{i:02}"), format!("Payment {i} of 100 EUR received.")],
                )
                .unwrap();
            }
        }
        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.push_completion(r#"{"from": "billing@supplier.example"}"#);
        let categories: Vec<String> = Vec::new();
        let est = estimate(&prepare_input(&db, &provider, &categories)).await;
        assert_eq!(est.emails, 30);
        assert_eq!(est.batches, 1, "one conversation, one batch");
    }

    #[tokio::test]
    async fn a_question_about_sent_mail_gathers_what_every_user_address_sent() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 0);
        {
            let conn = db.connection();
            for (id, sender) in [
                ("s1", "me@example.com"),
                ("s2", "me@work.example"),
                ("x1", "ana@client.example"),
            ] {
                conn.execute(
                    "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                     VALUES (?1,'acct',?1,'Quote',?2,?2,'d','[]','[]','snip',1780000000,1,'primary',
                             CASE WHEN ?2 = 'ana@client.example' THEN 'inbox' ELSE 'sent' END,0)",
                    rusqlite::params![id, sender],
                )
                .unwrap();
            }
        }
        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.push_completion(r#"{"from": "me@example.com"}"#);
        let categories: Vec<String> = Vec::new();
        let est = estimate(&prepare_input(&db, &provider, &categories)).await;
        let prepared = take_estimate(&est.estimate_id, "acct", "¿Qué facturas me ha enviado el proveedor?")
            .expect("kept for the run");
        let mut ids = prepared.email_ids.clone();
        ids.sort();
        // r00 is the seed's own reply from the account address.
        assert_eq!(ids, ["r00", "s1", "s2"], "the alias's mail too, never the client's");
    }

    #[tokio::test]
    async fn research_with_a_person_gathers_mail_either_way() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 0);
        {
            let conn = db.connection();
            let insert = |id: &str, sender: &str, address: &str, to: &str| {
                conn.execute(
                    "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                     VALUES (?1,'acct',?1,'Quote',?2,?3,'d',?4,'[]','snip',1780000000,1,'primary',0)",
                    rusqlite::params![id, sender, address, format!("[\"{to}\"]")],
                )
                .unwrap();
            };
            insert("a1", "Ana Ruiz", "ar@client.example", "me@example.com");
            insert("a2", "Me", "me@example.com", "ar@client.example");
            insert("b1", "Bob", "bob@x.example", "me@example.com");
        }
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let plan = SearchPlan {
            with: Some("Ana".into()),
            ..Default::default()
        };
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(plan)).await;
        let mut ids = prepared.email_ids.clone();
        ids.sort();
        assert_eq!(ids, ["a1", "a2"], "hers and the user's to her, not Bob's");
    }
}
