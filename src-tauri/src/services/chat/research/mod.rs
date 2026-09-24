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
mod plan;
mod prompts;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;

pub use control::{confirm_exit, exit_decision, request_stop, running_runs, ExitDecision};
pub(crate) use control::{register_run, store_estimate, take_estimate};
use plan::{
    merge_candidates, plan_batches, plan_condense_groups, plan_estimate, plan_gather, plan_research_budget,
    semantic_cutoff, GatherStep, CONDENSE_MAX_TOKENS, MAP_MAX_TOKENS, REDUCE_MAX_TOKENS,
};
use prompts::{
    assemble_notes, collect_matches, coverage_line, join_notes, notes_len, parse_map_notes, plan_report_shape,
    relink_bare_refs, render_match_list, report_facts, split_condense_prompt, split_map_prompt, split_reduce_prompt,
    BatchNotes, ReportShape, ResearchDoc,
};

use super::planner::SearchPlan;
use crate::ai::provider::{AIProvider, CompletionOptions, CompletionResult};
use crate::db::emails::search::TagQuery;
use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Email, LlmCallTrace, ResearchEstimate, ResearchTrace, ToolCallTrace};

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
    let mut prepared = gather(input, plan).await;
    prepared.planner_call = planner_call;
    prepared.gather_ms = t.elapsed().as_millis() as i64;
    prepared
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
    let mut prepared = Prepared::default();
    let mut lists: Vec<Vec<String>> = Vec::new();
    for step in plan_gather(plan.as_ref(), input.question) {
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
    let explicit = plan.from.is_some() || plan.to.is_some() || plan.subject.is_some();
    let categories = (!explicit && !input.categories.is_empty()).then_some(input.categories);
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
    let emails = prepared.email_ids.len();
    let (batches, seconds) = plan_estimate(emails, &budget, ms_per_email);
    let filter = prepared.plan.as_ref().map(filter_arguments);
    super::emit_log(
        "info",
        &format!("research: estimate {emails} emails, {batches} batches, ~{seconds}s"),
    );
    ResearchEstimate {
        estimate_id: store_estimate(input.account_id, input.question, prepared),
        emails: emails as u32,
        batches: batches as u32,
        seconds,
        filter,
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
    /// Raised by the chat's Stop button: stop reading, write the report.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResearchProgress {
    pub stage: ResearchStage,
    pub batch: usize,
    pub batches: usize,
    pub emails_read: usize,
    pub emails_total: usize,
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
async fn complete(
    provider: &dyn AIProvider,
    prompt: (&str, &str),
    max_tokens: u32,
    temperature: f64,
    timeout: std::time::Duration,
    kind: &str,
    round: i32,
) -> (std::result::Result<CompletionResult, String>, LlmCallTrace) {
    let opts = CompletionOptions {
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
        think: Some(false),
    };
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

/// Load the emails and cut each body to the research budget, in `ids` order.
fn load_docs(db: &Database, ids: &[String], chars_per_email: usize) -> Vec<ResearchDoc> {
    let emails = load_emails(db, ids);
    let by_id: HashMap<&str, &Email> = emails.iter().map(|e| (e.id.as_str(), e)).collect();
    ids.iter()
        .filter_map(|id| by_id.get(id.as_str()))
        .map(|email| {
            let body = match db.get_email_body(&email.id) {
                Ok(raw) if !raw.trim().is_empty() => {
                    crate::services::thread_clean::clean_email_body(&raw, chars_per_email)
                }
                Ok(_) => email.snippet.clone(),
                Err(e) => {
                    super::emit_log("debug", &format!("research: body of {} unavailable: {e}", email.id));
                    email.snippet.clone()
                }
            };
            ResearchDoc {
                id: email.id.clone(),
                thread_id: email.thread_id.clone(),
                date: chrono::DateTime::from_timestamp(email.timestamp, 0)
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_default(),
                from: if email.sender.is_empty() || email.sender == email.sender_email {
                    email.sender_email.clone()
                } else {
                    format!("{} <{}>", email.sender, email.sender_email)
                },
                subject: email.subject.clone(),
                body,
            }
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
            n_ctx: budget.n_ctx,
            planned_emails: prepared.email_ids.len() as u32,
            search_hits: prepared.search_hits,
            semantic_hits: prepared.semantic_hits,
            gather_ms: prepared.gather_ms,
            ..Default::default()
        },
        ..Default::default()
    };
    let progress = |stage, batch, batches, emails_read, emails_total| {
        on_progress(ResearchProgress {
            stage,
            batch,
            batches,
            emails_read,
            emails_total,
        })
    };

    // ── Map ──
    progress(ResearchStage::Reading, 0, 0, 0, prepared.email_ids.len());
    let docs = load_docs(input.db, &prepared.email_ids, budget.chars_per_email);
    let lens: Vec<usize> = docs.iter().map(ResearchDoc::rendered_len).collect();
    let batches = plan_batches(&lens, budget.batch_chars, budget.max_emails_per_batch);
    let t_map = std::time::Instant::now();
    let mut notes: Vec<BatchNotes> = Vec::with_capacity(batches.len());
    let mut read = 0;
    progress(ResearchStage::Reading, 0, batches.len(), 0, docs.len());
    for (i, range) in batches.iter().enumerate() {
        if input.stop.load(Ordering::Relaxed) {
            run.trace.stopped = true;
            super::emit_log(
                "info",
                &format!("research: stopped by the user after {read} of {} emails", docs.len()),
            );
            break;
        }
        let batch = &docs[range.clone()];
        let batch_ids: Vec<String> = batch.iter().map(|d| d.id.clone()).collect();
        let (prefix, suffix) = split_map_prompt(input.map_template, input.question, batch);
        let (result, trace) = complete(
            input.provider,
            (&prefix, &suffix),
            MAP_MAX_TOKENS,
            0.0,
            MAP_TIMEOUT,
            "research_map",
            i as i32,
        )
        .await;
        run.llm_calls.push(trace);
        match result {
            Ok(reply) => {
                let parsed = parse_map_notes(&reply.text, &batch_ids);
                super::emit_log(
                    "debug",
                    &format!(
                        "research: batch {}/{} → {} findings",
                        i + 1,
                        batches.len(),
                        parsed.lines.len()
                    ),
                );
                notes.push(parsed);
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
        progress(ResearchStage::Reading, i + 1, batches.len(), read, docs.len());
    }
    run.trace.map_ms = t_map.elapsed().as_millis() as i64;
    run.analyzed = docs[..read].iter().map(|d| d.id.clone()).collect();
    run.trace.findings = notes.iter().map(|b| b.lines.len() as u32).sum();
    // The matches come from the map notes, before any condense round: the
    // list and the counts must not depend on how the notes were merged.
    let matches = collect_matches(&docs[..read], &notes);
    run.relevant = matches.iter().map(|m| m.id.clone()).collect();
    run.trace.emails_analyzed = read as u32;
    run.trace.relevant_emails = matches.len() as u32;
    let shape = plan_report_shape(input.question);

    // ── Condense ──
    let t_condense = std::time::Instant::now();
    for round in 0..MAX_CONDENSE_ROUNDS {
        let lens: Vec<usize> = notes.iter().map(notes_len).collect();
        let Some(groups) = plan_condense_groups(&lens, budget.notes_chars) else {
            break;
        };
        if groups.len() == notes.len() && notes.len() == 1 {
            break; // one group that alone overflows: assemble_notes trims it
        }
        let mut merged = Vec::with_capacity(groups.len());
        for (j, group) in groups.iter().enumerate() {
            progress(ResearchStage::Condensing, j, groups.len(), read, docs.len());
            let block = join_notes(&notes[group.clone()]);
            let (prefix, suffix) = split_condense_prompt(input.condense_template, input.question, &block);
            let (result, trace) = complete(
                input.provider,
                (&prefix, &suffix),
                CONDENSE_MAX_TOKENS,
                0.0,
                REDUCE_TIMEOUT,
                "research_condense",
                (round * 1000 + j) as i32,
            )
            .await;
            run.llm_calls.push(trace);
            run.trace.condense_calls += 1;
            match result {
                Ok(reply) => {
                    let parsed = parse_map_notes(&reply.text, &run.analyzed);
                    // A condense that lost every citation would erase the
                    // group; keep the originals for the report to trim.
                    if parsed.lines.is_empty() {
                        merged.extend(notes[group.clone()].iter().cloned());
                    } else {
                        merged.push(parsed);
                    }
                }
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
        read,
        docs.len(),
    );
    let notes_block = assemble_notes(&notes, budget.notes_chars);
    let notes_block = if notes_block.trim().is_empty() {
        "(no relevant findings in the emails read)".to_string()
    } else {
        notes_block
    };
    let coverage = coverage_line(
        read,
        docs.len(),
        run.relevant.len(),
        run.trace.batches as usize,
        run.trace.failed_batches as usize,
    );
    let facts = report_facts(&matches, shape);
    let (prefix, suffix) = split_reduce_prompt(
        input.reduce_template,
        input.language_instruction,
        input.question,
        &coverage,
        &facts,
        &notes_block,
    );
    let t_reduce = std::time::Instant::now();
    let (result, trace) = complete(
        input.provider,
        (&prefix, &suffix),
        REDUCE_MAX_TOKENS,
        0.2,
        REDUCE_TIMEOUT,
        "research_reduce",
        -1,
    )
    .await;
    run.llm_calls.push(trace);
    run.trace.reduce_ms = t_reduce.elapsed().as_millis() as i64;
    // Built in code, not written by the model: every match, however many.
    let full_list = if shape == ReportShape::FullList {
        render_match_list(&matches, input.language_code)
    } else {
        String::new()
    };
    match result {
        Ok(reply) if !reply.text.trim().is_empty() => {
            let subjects: HashMap<String, String> = docs.iter().map(|d| (d.id.clone(), d.subject.clone())).collect();
            let prose = relink_bare_refs(reply.text.trim(), &subjects);
            run.answer = Some(if full_list.is_empty() {
                prose
            } else {
                format!("{prose}\n\n{full_list}")
            });
        }
        // The report failed but the list stands on its own: ship it.
        _ if !full_list.is_empty() => run.answer = Some(full_list),
        Ok(_) => run.error = Some("the research report came back empty".to_string()),
        Err(e) => run.error = Some(format!("writing the research report failed: {e}")),
    }

    // The next estimate uses what this machine actually took.
    if read > 0 {
        let ms = (run.trace.map_ms + run.trace.condense_ms + run.trace.reduce_ms) as u64 / read as u64;
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

    #[tokio::test]
    async fn research_reads_in_batches_and_reports_from_the_notes() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 29);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        // 30 emails → three batches; only the one holding e05 keeps its finding.
        for _ in 0..3 {
            provider.push_completion("Findings:\n- Invoice 5 for 100 EUR is due Friday (email://e05)");
        }
        provider.push_completion("Tienes 29 facturas; ver [Invoice 5](email://e05).");
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
    async fn a_list_question_gets_every_match_listed_and_counted_exactly() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 24);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        assert_eq!(prepared.email_ids.len(), 25);
        let budget = plan_research_budget(16384);
        // Every email of every batch is a match: 25 of them, more than a
        // model-written list would hold.
        for batch in prepared.email_ids.chunks(budget.max_emails_per_batch) {
            let reply: String = batch
                .iter()
                .map(|id| format!("- Invoice request (email://{id})\n"))
                .collect();
            provider.push_completion(reply);
        }
        provider.push_completion("Resumen: el proveedor envió facturas mensuales.");
        let stop = AtomicBool::new(false);
        let mut input = run_input(&db, &provider, &prepared, 16384, &stop);
        input.question = "Dame una lista con todas las facturas del proveedor";
        let run = run_research(input, &|_| {}).await;

        let answer = run.answer.expect("an answer");
        assert!(answer.starts_with("Resumen:"), "{answer}");
        assert!(answer.contains("### Lista completa (25)"), "{answer}");
        assert!(answer.contains("\n25. "), "every match is listed: {answer}");
        let calls = provider.prefix_completion_calls();
        let reduce_prompt = &calls.last().expect("reduce").1;
        assert!(
            reduce_prompt.contains("25 emails with relevant findings, in 24 conversations"),
            "{reduce_prompt}"
        );
        assert!(reduce_prompt.contains("appended"), "{reduce_prompt}");
        assert_eq!(run.trace.relevant_emails, 25);
    }

    #[tokio::test]
    async fn stop_ends_the_reading_and_still_writes_a_report() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 29);
        let provider = crate::ai::provider::FakeAiProvider::new();
        let categories: Vec<String> = Vec::new();
        let prepared = gather(&prepare_input(&db, &provider, &categories), Some(supplier_plan())).await;
        provider.push_completion("- Invoice 0 (email://e00)");
        provider.push_completion("Informe parcial [Invoice 0](email://e00).");
        let stop = AtomicBool::new(false);
        // Raise Stop once the first batch has been read.
        let on_progress = |p: ResearchProgress| {
            if p.stage == ResearchStage::Reading && p.batch == 1 {
                stop.store(true, Ordering::Relaxed);
            }
        };
        let run = run_research(run_input(&db, &provider, &prepared, 16384, &stop), &on_progress).await;

        assert!(run.trace.stopped);
        assert_eq!(run.trace.batches, 1);
        assert_eq!(run.trace.emails_analyzed, 10);
        assert_eq!(run.trace.planned_emails, 30);
        assert_eq!(run.answer.as_deref(), Some("Informe parcial [Invoice 0](email://e00)."));
        let reduce_prompt = &provider.prefix_completion_calls()[1].1;
        assert!(
            reduce_prompt.contains("stopped by the user after reading 10 of 30"),
            "{reduce_prompt}"
        );
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
        let batches: Vec<&[String]> = prepared.email_ids.chunks(budget.max_emails_per_batch).collect();
        for batch in &batches {
            // A long finding citing an email of its own batch, so it is kept.
            let note = format!("- {} (email://{})", "x".repeat(budget.notes_chars / 2), batch[0]);
            provider.push_completion(note);
        }
        let batches = batches.len();
        for _ in 0..batches {
            provider.push_completion("- merged (email://e00)");
        }
        provider.push_completion("Informe [Invoice 0](email://e00).");
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
        assert_eq!(est.batches, 5);
        assert!(est.seconds > 0);
        assert_eq!(
            est.filter.as_ref().and_then(|f| f["from"].as_str()),
            Some("billing@supplier.example")
        );
        let prepared = take_estimate(&est.estimate_id, "acct", input.question).expect("kept for the run");
        assert_eq!(prepared.email_ids.len(), 41);
        assert!(prepared.planner_call.is_some());
    }
}
