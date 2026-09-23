//! Research mode: a slower chat turn that reads far more of the mailbox.
//!
//! A normal turn answers from ~8 retrieved sources or one page of 25 search
//! rows, which is fine for "what did X say?" and too thin for "what themes
//! came up with clients this quarter?". Research mode trades minutes for
//! coverage with a map-reduce over the mailbox:
//!
//! 1. **Gather** — page the planner's `search_emails` filter and/or pull a wide
//!    hybrid-retrieval set, merged and capped by [`ResearchBudget::max_emails`].
//! 2. **Map** — read the candidates in batches sized to the model's context
//!    window, one `complete_with_prefix` call per batch that extracts the
//!    findings relevant to the question, each tied to its `email://` id.
//! 3. **Reduce** — one final call writes the report from the notes.
//!
//! Every LLM call is a one-shot completion on the auxiliary prefix slot, so the
//! chat's own KV anchor (the `chat.system` prompt) is never touched: the next
//! ordinary turn still reuses it.
//!
//! Pure planners live at the top of the file and are unit-tested; the thin
//! executor [`run_research`] at the bottom does the I/O.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use serde_json::Value;

use super::planner::SearchPlan;
use crate::ai::provider::{AIProvider, CompletionOptions};
use crate::db::Database;
use crate::models::{LlmCallTrace, ResearchTrace};

// ── Budget ──────────────────────────────────────────────────────────────────

/// Conservative chars-per-token for mixed EN/ES mail on the Qwen tokenizer
/// (measured ~3.5-4); erring low keeps a batch from front-truncating.
const CHARS_PER_TOKEN: usize = 3;
/// Instructions + question of the map prompt, in tokens.
const MAP_OVERHEAD_TOKENS: usize = 700;
/// Findings one batch may produce. Bounds the notes a batch adds to the reduce.
const MAP_MAX_TOKENS: u32 = 400;
/// Instructions + question + coverage line of the reduce prompt, in tokens.
const REDUCE_OVERHEAD_TOKENS: usize = 700;
/// The final report's length.
const REDUCE_MAX_TOKENS: u32 = 1536;
/// Slack for tokenizer error and chat-template tokens.
const SAFETY_TOKENS: usize = 256;
/// Cleaned body kept per email: enough for the substance of most mail, small
/// enough that a batch holds several.
const CHARS_PER_EMAIL: usize = 1500;
/// A small model extracts less reliably from a long batch ("lost in the
/// middle"), so batches stay this small even when the window allows more.
const MAX_EMAILS_PER_BATCH: usize = 10;
/// Upper bound on emails per research turn: roughly 10 batches, a few minutes
/// on a 4B local model.
const MAX_RESEARCH_EMAILS: usize = 100;
/// Floor: below this the window cannot hold a useful batch.
const MIN_RESEARCH_EMAILS: usize = 10;
/// Hybrid-retrieval depth for research (a normal turn uses 8). Bounded by the
/// candidate pool retrieval fuses (vector + FTS), so larger buys nothing.
pub(crate) const RESEARCH_RAG_K: usize = 40;
/// `search_emails` page size (the tool's own maximum).
pub(crate) const SEARCH_PAGE: usize = 25;

/// How much one research turn reads, derived from the context window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResearchBudget {
    pub n_ctx: u32,
    /// Emails gathered at most.
    pub max_emails: usize,
    /// Cleaned body chars per email.
    pub chars_per_email: usize,
    /// Email chars one map batch may carry.
    pub batch_chars: usize,
    pub max_emails_per_batch: usize,
    pub map_max_tokens: u32,
    pub reduce_max_tokens: u32,
    /// Notes chars the reduce prompt can carry.
    pub notes_chars: usize,
}

/// Size a research turn to the window. Pure: `n_ctx` is the only input.
///
/// The binding constraint is the reduce: every batch contributes up to
/// `MAP_MAX_TOKENS` of notes and all of them must fit one reduce prompt, so the
/// number of batches — and therefore of emails — is capped by what the notes
/// budget holds.
pub(crate) fn plan_research_budget(n_ctx: u32) -> ResearchBudget {
    let window = n_ctx as usize;
    let batch_tokens = window.saturating_sub(MAP_OVERHEAD_TOKENS + MAP_MAX_TOKENS as usize + SAFETY_TOKENS);
    let batch_chars = (batch_tokens * CHARS_PER_TOKEN).min(CHARS_PER_EMAIL * MAX_EMAILS_PER_BATCH);
    let max_emails_per_batch = (batch_chars / CHARS_PER_EMAIL).clamp(1, MAX_EMAILS_PER_BATCH);
    let notes_tokens = window.saturating_sub(REDUCE_OVERHEAD_TOKENS + REDUCE_MAX_TOKENS as usize + SAFETY_TOKENS);
    let notes_chars = notes_tokens * CHARS_PER_TOKEN;
    let batch_notes_chars = MAP_MAX_TOKENS as usize * CHARS_PER_TOKEN;
    let max_batches = (notes_chars / batch_notes_chars).max(1);
    let max_emails = (max_batches * max_emails_per_batch).clamp(MIN_RESEARCH_EMAILS, MAX_RESEARCH_EMAILS);
    ResearchBudget {
        n_ctx,
        max_emails,
        chars_per_email: CHARS_PER_EMAIL,
        batch_chars,
        max_emails_per_batch,
        map_max_tokens: MAP_MAX_TOKENS,
        reduce_max_tokens: REDUCE_MAX_TOKENS,
        notes_chars,
    }
}

// ── Planner verdict ─────────────────────────────────────────────────────────

/// The query-planner verdicts that take a turn off the mailbox. A `Search`
/// or `Defer` verdict is a mailbox question and needs no ruling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlannerVerdict {
    AppHelp,
    FormFill,
}

/// Whether a research turn keeps researching after the planner's verdict.
///
/// The toggle is the user saying the question is about their mailbox, so it
/// outranks an app-help verdict: "how have EmailOps downloads evolved?" names
/// the app, and the planner reads it as a question about EmailOps, while the
/// answer lives in weekly stats emails. Only a form fill stops it — "create a
/// lens for supplier invoices" is an action with nothing to read.
pub(crate) fn research_continues(verdict: PlannerVerdict) -> bool {
    verdict != PlannerVerdict::FormFill
}

// ── Gather ──────────────────────────────────────────────────────────────────

/// Which searches feed the candidate set.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GatherPlan {
    /// `search_emails` arguments to page through, without `offset`.
    pub search_args: Option<Value>,
    /// Hybrid-retrieval depth; 0 skips retrieval.
    pub rag_k: usize,
}

/// Decide how to gather candidates from the query planner's verdict.
///
/// A plan with a structural filter (sender, date window, tag…) states exactly
/// which mail the question is about, so only that filter is paged: semantic
/// neighbours from outside the window would dilute the notes. A keyword-only
/// plan, or no plan at all, is a topic question — retrieval ranks those by
/// meaning, and the keyword search (when there is one) adds exact hits.
pub(crate) fn plan_gather(plan: Option<&SearchPlan>) -> GatherPlan {
    match plan {
        Some(plan) => {
            let mut args = plan.clone().into_tool_call().function.arguments;
            if let Some(obj) = args.as_object_mut() {
                obj.insert("limit".into(), serde_json::json!(SEARCH_PAGE));
                // Bodies are fetched per email below, at the research budget;
                // the tool's own rendering is only mined for ids.
                obj.remove("include_bodies");
                // "The first email" asks for one row; research wants them all,
                // oldest first.
            }
            GatherPlan {
                search_args: Some(args),
                rag_k: if plan.has_structural_filter() {
                    0
                } else {
                    RESEARCH_RAG_K
                },
            }
        }
        None => GatherPlan {
            search_args: None,
            rag_k: RESEARCH_RAG_K,
        },
    }
}

/// The `search_emails` arguments for the page starting at `offset`.
pub(crate) fn search_page_args(base: &Value, offset: usize) -> Value {
    let mut args = base.clone();
    if let Some(obj) = args.as_object_mut() {
        obj.insert("offset".into(), serde_json::json!(offset));
    }
    args
}

/// Merge the two candidate lists, alternating so neither crowds the other
/// out, dropping repeats, and keeping at most `max`.
pub(crate) fn merge_candidates(search_ids: &[String], rag_ids: &[String], max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let longest = search_ids.len().max(rag_ids.len());
    for i in 0..longest {
        for list in [search_ids, rag_ids] {
            if let Some(id) = list.get(i) {
                if out.len() < max && !out.contains(id) {
                    out.push(id.clone());
                }
            }
        }
    }
    out
}

// ── Map ─────────────────────────────────────────────────────────────────────

/// One email as the map step reads it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResearchDoc {
    pub id: String,
    pub date: String,
    pub from: String,
    pub subject: String,
    pub body: String,
}

impl ResearchDoc {
    fn render(&self) -> String {
        format!(
            "EMAIL_ID: {}\nDate: {}\nFrom: {}\nSubject: {}\n{}\n",
            self.id, self.date, self.from, self.subject, self.body
        )
    }

    fn rendered_len(&self) -> usize {
        self.render().chars().count()
    }
}

/// Split documents (by rendered length) into consecutive batches of at most
/// `batch_chars` and `max_per_batch` each. A single document longer than the
/// budget still gets a batch of its own — the caller already cut bodies to
/// `chars_per_email`, so that only happens with a tiny window.
pub(crate) fn plan_batches(doc_lens: &[usize], batch_chars: usize, max_per_batch: usize) -> Vec<Range<usize>> {
    let mut batches = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (i, len) in doc_lens.iter().enumerate() {
        let full = i - start >= max_per_batch || (i > start && used + len > batch_chars);
        if full {
            batches.push(start..i);
            start = i;
            used = 0;
        }
        used += len;
    }
    if start < doc_lens.len() {
        batches.push(start..doc_lens.len());
    }
    batches
}

/// The map prompt's split point: everything above is the same on every batch
/// of every research turn, so it stays decoded in the one-shot prefix slot.
const MAP_MARKER: &str = "QUESTION: {{question}}";
/// Same for the reduce prompt.
const REDUCE_MARKER: &str = "QUESTION: {{question}}";

/// Render a template and cut it at `marker` into (invariant head, per-call
/// tail). A user-edited template without the marker still works; it only
/// forfeits the prefix cache.
fn split_at_marker(template: &str, marker: &str, vars: &HashMap<&str, String>) -> (String, String) {
    let (head, tail) = match template.find(marker) {
        Some(idx) => template.split_at(idx),
        None => (template, ""),
    };
    (
        crate::services::prompts::render(head, vars),
        crate::services::prompts::render(tail, vars),
    )
}

/// The map prompt for one batch, split for `complete_with_prefix`.
pub(crate) fn split_map_prompt(template: &str, question: &str, docs: &[ResearchDoc]) -> (String, String) {
    let emails = docs.iter().map(ResearchDoc::render).collect::<Vec<_>>().join("\n");
    let mut vars = HashMap::new();
    vars.insert("question", question.to_string());
    vars.insert("emails", emails);
    split_at_marker(template, MAP_MARKER, &vars)
}

/// What one batch yielded: the finding lines, and the emails they cite.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BatchNotes {
    pub lines: Vec<String>,
    pub cited: Vec<String>,
}

/// Keep the findings of a map reply that cite an email of the batch.
///
/// A finding with no citation to a batch email is dropped rather than trusted:
/// it is either chatter ("Here are the findings:"), a "nothing relevant" in
/// some wording, or a claim the reduce could not link — and an unlinked claim
/// in the final report is exactly what research mode must not produce.
pub(crate) fn parse_map_notes(reply: &str, batch_ids: &[String]) -> BatchNotes {
    let mut notes = BatchNotes::default();
    for raw in reply.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let cited: Vec<&String> = batch_ids.iter().filter(|id| line.contains(id.as_str())).collect();
        if cited.is_empty() {
            continue;
        }
        let body = line.trim_start_matches(['-', '*', '•', ' ']).trim();
        notes.lines.push(format!("- {body}"));
        for id in cited {
            if !notes.cited.contains(id) {
                notes.cited.push(id.clone());
            }
        }
    }
    notes
}

/// Join every batch's findings into the notes block, trimmed to `max_chars`.
///
/// When the notes overflow, each batch keeps an equal share of its leading
/// lines (a model lists its strongest findings first) instead of the last
/// batches being cut off entirely — the tail of the candidate list is still
/// part of what the user asked to have read.
pub(crate) fn assemble_notes(batches: &[BatchNotes], max_chars: usize) -> String {
    let total: usize = batches
        .iter()
        .flat_map(|b| b.lines.iter())
        .map(|l| l.chars().count() + 1)
        .sum();
    let non_empty = batches.iter().filter(|b| !b.lines.is_empty()).count().max(1);
    let share = if total <= max_chars {
        usize::MAX
    } else {
        max_chars / non_empty
    };
    let mut out = String::new();
    for batch in batches {
        let mut used = 0;
        for line in &batch.lines {
            let len = line.chars().count() + 1;
            if used + len > share {
                break;
            }
            used += len;
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The reduce prompt, split for `complete_with_prefix`. `coverage` is the
/// per-turn "read N emails, M relevant" line — it rides in the tail so the
/// head stays identical across research turns.
pub(crate) fn split_reduce_prompt(
    template: &str,
    language_instruction: &str,
    question: &str,
    coverage: &str,
    notes: &str,
) -> (String, String) {
    let mut vars = HashMap::new();
    vars.insert("language_instruction", language_instruction.to_string());
    vars.insert("question", question.to_string());
    vars.insert("coverage", coverage.to_string());
    vars.insert("notes", notes.to_string());
    split_at_marker(template, REDUCE_MARKER, &vars)
}

/// The coverage line the report ends on.
pub(crate) fn coverage_line(analyzed: usize, relevant: usize, batches: usize, failed_batches: usize) -> String {
    let mut line = format!("read {analyzed} emails in {batches} batches; {relevant} of them had relevant findings");
    if failed_batches > 0 {
        line.push_str(&format!(" ({failed_batches} batches could not be read)"));
    }
    line
}

// ── Report post-processing ──────────────────────────────────────────────────

/// Longest link label built from a subject.
const MAX_LABEL_CHARS: usize = 60;

/// A subject as a Markdown link label: no brackets (they would end the label),
/// whitespace collapsed, cut to [`MAX_LABEL_CHARS`].
fn link_label(subject: &str) -> String {
    let cleaned: String = subject.replace(['[', ']'], "");
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    if words.is_empty() {
        return "email".to_string();
    }
    let joined = words.join(" ");
    if joined.chars().count() <= MAX_LABEL_CHARS {
        joined
    } else {
        let cut: String = joined.chars().take(MAX_LABEL_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

/// Turn the bare references a report makes into real email links.
///
/// The notes cite as `(email://ID)` and a small model copies that shape (or
/// writes `[email://ID]`) instead of `[label](email://ID)`, which the chat only
/// renders as a clickable chip in the link form. Each bare reference to an
/// email that was read becomes a link labelled with its subject; proper links
/// and ids that were never read are left untouched (the link allowlist drops
/// the latter downstream).
pub(crate) fn relink_bare_refs(answer: &str, subjects: &HashMap<String, String>) -> String {
    use std::sync::OnceLock;
    static BARE_RE: OnceLock<regex::Regex> = OnceLock::new();
    // Hard-coded literal that cannot fail by construction.
    #[allow(clippy::unwrap_used)]
    let re = BARE_RE.get_or_init(|| regex::Regex::new(r"\[email://([^\]\s]+)\]|\(email://([^)\s]+)\)").unwrap());
    let mut out = String::with_capacity(answer.len());
    let mut last = 0;
    for caps in re.captures_iter(answer) {
        let Some(whole) = caps.get(0) else { continue };
        let Some(id) = caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str()) else {
            continue;
        };
        // `(email://ID)` right after `]` is already the target of a link.
        let is_link_target = caps.get(2).is_some() && answer[..whole.start()].ends_with(']');
        let Some(subject) = subjects.get(id).filter(|_| !is_link_target) else {
            continue;
        };
        out.push_str(&answer[last..whole.start()]);
        out.push_str(&format!("[{}](email://{id})", link_label(subject)));
        last = whole.end();
    }
    out.push_str(&answer[last..]);
    out
}

// ── Context window ──────────────────────────────────────────────────────────

/// Ollama's default `num_ctx` (see `ai::ollama`), and the window every other
/// backend is assumed to have.
const DEFAULT_N_CTX: u32 = 8192;

/// The window to size research batches to. Pure: the embedded runtime uses the
/// `chat.n_ctx` override when set, else the machine's RAM tier; the HTTP
/// backends run at their 8k default.
pub(crate) fn plan_n_ctx(provider: crate::ai::provider::ProviderType, n_ctx_override: u32, auto_tier: u32) -> u32 {
    match provider {
        crate::ai::provider::ProviderType::LlamaCpp if n_ctx_override > 0 => n_ctx_override,
        crate::ai::provider::ProviderType::LlamaCpp => auto_tier,
        _ => DEFAULT_N_CTX,
    }
}

/// Read the inputs of [`plan_n_ctx`] from the preferences and the machine.
pub(crate) fn resolve_n_ctx(db: &Database, provider: crate::ai::provider::ProviderType) -> u32 {
    let n_ctx_override = db
        .get_preference("chat.n_ctx")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let auto_tier = crate::util::system::auto_n_ctx_tier(crate::util::system::total_ram_bytes());
    plan_n_ctx(provider, n_ctx_override, auto_tier)
}

// ── Executor ────────────────────────────────────────────────────────────────

/// One map call may not stall the turn forever; a batch that times out is
/// skipped and counted as failed.
const MAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
/// The report is longer than a batch's findings.
const REDUCE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Everything a research run reads.
pub(crate) struct ResearchInput<'a> {
    pub db: &'a Arc<Database>,
    pub provider: &'a dyn AIProvider,
    pub account_id: &'a str,
    pub categories: &'a [String],
    pub question: &'a str,
    /// The query planner's filter for this question, when it produced one.
    pub plan: Option<&'a SearchPlan>,
    pub n_ctx: u32,
    pub language_instruction: &'a str,
    pub map_template: &'a str,
    pub reduce_template: &'a str,
}

/// Where a research run is, for the progress indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResearchStage {
    Gathering,
    Reading,
    Writing,
}

impl ResearchStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Gathering => "gathering",
            Self::Reading => "reading",
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
    result: Option<&crate::ai::provider::CompletionResult>,
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
        input: None,
        output: result.map(|r| r.text.clone()),
    }
}

/// Page the planner's `search_emails` filter until it runs dry or `max` ids
/// are collected. The tool is called directly (not through the model) so the
/// filter semantics — tag ranking, category scope, junk exclusion — are the
/// ones a normal turn gets.
async fn gather_search(input: &ResearchInput<'_>, base: &Value, max: usize) -> Vec<String> {
    use super::tools::Tool;
    let ctx = super::tools::ToolCtx {
        db: input.db,
        account_id: input.account_id,
        categories: input.categories,
        page: None,
    };
    let tool = super::tools::search_emails::SearchEmailsTool;
    let mut ids: Vec<String> = Vec::new();
    let mut offset = 0;
    while ids.len() < max {
        let refs = match tool.execute(&ctx, search_page_args(base, offset)).await {
            Ok(out) => out.email_refs,
            Err(e) => {
                super::emit_log(
                    "error",
                    &format!("research: search page at offset {offset} failed: {e}"),
                );
                break;
            }
        };
        let page_len = refs.len();
        let before = ids.len();
        for id in refs {
            if ids.len() < max && !ids.contains(&id) {
                ids.push(id);
            }
        }
        if page_len < SEARCH_PAGE || ids.len() == before {
            break;
        }
        offset += SEARCH_PAGE;
    }
    ids
}

/// Load the emails and cut each body to the research budget, in `ids` order.
fn load_docs(db: &Arc<Database>, ids: &[String], chars_per_email: usize) -> Vec<ResearchDoc> {
    let emails = match db.get_emails_by_ids(ids) {
        Ok(emails) => emails,
        Err(e) => {
            super::emit_log("error", &format!("research: loading {} emails failed: {e}", ids.len()));
            return Vec::new();
        }
    };
    let by_id: HashMap<&str, &crate::models::Email> = emails.iter().map(|e| (e.id.as_str(), e)).collect();
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

/// Gather → map → reduce. Never fails the turn on its own: a search or a batch
/// that errors is logged and skipped, and only a failed reduce comes back as
/// `answer: None` for the caller to report.
pub(crate) async fn run_research(
    input: ResearchInput<'_>,
    on_progress: &(dyn Fn(ResearchProgress) + Sync),
) -> ResearchRun {
    let budget = plan_research_budget(input.n_ctx);
    let mut run = ResearchRun {
        trace: ResearchTrace {
            n_ctx: budget.n_ctx,
            max_emails: budget.max_emails as u32,
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

    // ── Gather ──
    progress(ResearchStage::Gathering, 0, 0, 0, 0);
    let t_gather = std::time::Instant::now();
    let gather = plan_gather(input.plan);
    let search_ids = match &gather.search_args {
        Some(base) => gather_search(&input, base, budget.max_emails).await,
        None => Vec::new(),
    };
    let rag_ids: Vec<String> = if gather.rag_k > 0 {
        match super::retrieval::retrieve_context_full(
            input.db,
            input.provider,
            input.account_id,
            input.question,
            input.categories,
            gather.rag_k,
        )
        .await
        {
            Ok((sources, _, _)) => sources.into_iter().map(|s| s.email.id).collect(),
            Err(e) => {
                super::emit_log("error", &format!("research: retrieval failed: {e}"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    run.trace.search_hits = search_ids.len() as u32;
    run.trace.retrieval_hits = rag_ids.len() as u32;
    let ids = merge_candidates(&search_ids, &rag_ids, budget.max_emails);
    let docs = load_docs(input.db, &ids, budget.chars_per_email);
    run.trace.gather_ms = t_gather.elapsed().as_millis() as i64;
    super::emit_log(
        "info",
        &format!(
            "research: gathered {} emails ({} search + {} rag, cap {}) [{}ms]",
            docs.len(),
            search_ids.len(),
            rag_ids.len(),
            budget.max_emails,
            run.trace.gather_ms
        ),
    );

    // ── Map ──
    let t_map = std::time::Instant::now();
    let lens: Vec<usize> = docs.iter().map(ResearchDoc::rendered_len).collect();
    let batches = plan_batches(&lens, budget.batch_chars, budget.max_emails_per_batch);
    run.trace.batches = batches.len() as u32;
    let mut notes: Vec<BatchNotes> = Vec::with_capacity(batches.len());
    progress(ResearchStage::Reading, 0, batches.len(), 0, docs.len());
    for (i, range) in batches.iter().enumerate() {
        let batch = &docs[range.clone()];
        let batch_ids: Vec<String> = batch.iter().map(|d| d.id.clone()).collect();
        let (prefix, suffix) = split_map_prompt(input.map_template, input.question, batch);
        let opts = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(budget.map_max_tokens),
            think: Some(false),
        };
        let t_call = std::time::Instant::now();
        let result =
            tokio::time::timeout(MAP_TIMEOUT, input.provider.complete_with_prefix(&prefix, &suffix, opts)).await;
        let latency = t_call.elapsed().as_millis() as i64;
        match result {
            Ok(Ok(reply)) => {
                let parsed = parse_map_notes(&reply.text, &batch_ids);
                super::emit_log(
                    "debug",
                    &format!(
                        "research: batch {}/{} → {} findings [{latency}ms]",
                        i + 1,
                        batches.len(),
                        parsed.lines.len()
                    ),
                );
                run.llm_calls
                    .push(call_trace("research_map", i as i32, latency, Some(&reply)));
                notes.push(parsed);
            }
            Ok(Err(e)) => {
                super::emit_log(
                    "error",
                    &format!("research: batch {}/{} failed: {e}", i + 1, batches.len()),
                );
                run.trace.failed_batches += 1;
                run.llm_calls.push(call_trace("research_map", i as i32, latency, None));
            }
            Err(_) => {
                super::emit_log(
                    "error",
                    &format!(
                        "research: batch {}/{} timed out after {}s",
                        i + 1,
                        batches.len(),
                        MAP_TIMEOUT.as_secs()
                    ),
                );
                run.trace.failed_batches += 1;
                run.llm_calls.push(call_trace("research_map", i as i32, latency, None));
            }
        }
        progress(ResearchStage::Reading, i + 1, batches.len(), range.end, docs.len());
    }
    run.trace.map_ms = t_map.elapsed().as_millis() as i64;
    run.analyzed = docs.iter().map(|d| d.id.clone()).collect();
    for batch in &notes {
        run.trace.findings += batch.lines.len() as u32;
        for id in &batch.cited {
            if !run.relevant.contains(id) {
                run.relevant.push(id.clone());
            }
        }
    }
    run.trace.emails_analyzed = docs.len() as u32;
    run.trace.relevant_emails = run.relevant.len() as u32;

    // ── Reduce ──
    progress(
        ResearchStage::Writing,
        batches.len(),
        batches.len(),
        docs.len(),
        docs.len(),
    );
    let t_reduce = std::time::Instant::now();
    let notes_block = assemble_notes(&notes, budget.notes_chars);
    let notes_block = if notes_block.trim().is_empty() {
        "(no relevant findings in the emails read)".to_string()
    } else {
        notes_block
    };
    let coverage = coverage_line(
        docs.len(),
        run.relevant.len(),
        batches.len(),
        run.trace.failed_batches as usize,
    );
    let (prefix, suffix) = split_reduce_prompt(
        input.reduce_template,
        input.language_instruction,
        input.question,
        &coverage,
        &notes_block,
    );
    let opts = CompletionOptions {
        temperature: Some(0.2),
        max_tokens: Some(budget.reduce_max_tokens),
        think: Some(false),
    };
    let result = tokio::time::timeout(
        REDUCE_TIMEOUT,
        input.provider.complete_with_prefix(&prefix, &suffix, opts),
    )
    .await;
    let latency = t_reduce.elapsed().as_millis() as i64;
    run.trace.reduce_ms = latency;
    match result {
        Ok(Ok(reply)) if !reply.text.trim().is_empty() => {
            run.llm_calls
                .push(call_trace("research_reduce", -1, latency, Some(&reply)));
            let subjects: HashMap<String, String> = docs.iter().map(|d| (d.id.clone(), d.subject.clone())).collect();
            run.answer = Some(relink_bare_refs(reply.text.trim(), &subjects));
        }
        Ok(Ok(reply)) => {
            run.llm_calls
                .push(call_trace("research_reduce", -1, latency, Some(&reply)));
            run.error = Some("the research report came back empty".to_string());
        }
        Ok(Err(e)) => {
            run.llm_calls.push(call_trace("research_reduce", -1, latency, None));
            run.error = Some(format!("writing the research report failed: {e}"));
        }
        Err(_) => {
            run.llm_calls.push(call_trace("research_reduce", -1, latency, None));
            run.error = Some(format!(
                "writing the research report timed out after {}s",
                REDUCE_TIMEOUT.as_secs()
            ));
        }
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── budget ──

    #[test]
    fn budget_notes_fit_the_reduce_window() {
        for n_ctx in [8192u32, 16384, 32768] {
            let b = plan_research_budget(n_ctx);
            let batches = b.max_emails.div_ceil(b.max_emails_per_batch);
            let notes_tokens = batches * b.map_max_tokens as usize;
            let reduce_tokens = notes_tokens + REDUCE_OVERHEAD_TOKENS + b.reduce_max_tokens as usize;
            assert!(
                reduce_tokens <= n_ctx as usize,
                "n_ctx={n_ctx}: {batches} batches of notes need {reduce_tokens} tokens"
            );
        }
    }

    #[test]
    fn budget_batches_fit_the_map_window() {
        for n_ctx in [8192u32, 16384, 32768] {
            let b = plan_research_budget(n_ctx);
            let batch_tokens = b.batch_chars / CHARS_PER_TOKEN + MAP_OVERHEAD_TOKENS + b.map_max_tokens as usize;
            assert!(
                batch_tokens <= n_ctx as usize,
                "n_ctx={n_ctx}: batch needs {batch_tokens}"
            );
            assert!(b.max_emails_per_batch * b.chars_per_email <= b.batch_chars);
        }
    }

    #[test]
    fn budget_reads_more_with_a_bigger_window_up_to_the_cap() {
        let small = plan_research_budget(8192);
        let large = plan_research_budget(32768);
        assert!(
            small.max_emails >= 50,
            "8k still reads a useful set: {}",
            small.max_emails
        );
        assert!(large.max_emails >= small.max_emails);
        assert_eq!(large.max_emails, MAX_RESEARCH_EMAILS);
    }

    #[test]
    fn budget_never_reads_fewer_than_the_floor() {
        // A window too small for the notes still gathers the floor; the reduce
        // trims notes (assemble_notes) instead of failing.
        let b = plan_research_budget(4096);
        assert!(b.max_emails >= MIN_RESEARCH_EMAILS);
        assert!(b.max_emails_per_batch >= 1);
    }

    // ── gather ──

    fn plan(f: impl FnOnce(&mut SearchPlan)) -> SearchPlan {
        let mut p = SearchPlan::default();
        f(&mut p);
        p
    }

    #[test]
    fn gather_with_a_structural_filter_pages_the_search_only() {
        let p = plan(|p| {
            p.intent = Some("billing".into());
            p.since = Some("2026-06-23".into());
        });
        let g = plan_gather(Some(&p));
        assert_eq!(g.rag_k, 0);
        let args = g.search_args.expect("search args");
        assert_eq!(args["intent"], "billing");
        assert_eq!(args["since"], "2026-06-23");
        assert_eq!(args["limit"], SEARCH_PAGE);
        assert!(args.get("include_bodies").is_none());
    }

    #[test]
    fn gather_with_a_keyword_plan_combines_search_and_retrieval() {
        let p = plan(|p| p.query = Some("migración".into()));
        let g = plan_gather(Some(&p));
        assert_eq!(g.rag_k, RESEARCH_RAG_K);
        assert_eq!(g.search_args.expect("search args")["query"], "migración");
    }

    #[test]
    fn gather_without_a_plan_uses_retrieval_only() {
        let g = plan_gather(None);
        assert_eq!(g.search_args, None);
        assert_eq!(g.rag_k, RESEARCH_RAG_K);
    }

    #[test]
    fn gather_does_not_limit_an_oldest_plan_to_one_row() {
        let p = plan(|p| {
            p.from = Some("alice@example.com".into());
            p.order = Some("oldest".into());
        });
        let args = plan_gather(Some(&p)).search_args.expect("search args");
        assert_eq!(args["limit"], SEARCH_PAGE);
        assert_eq!(args["order"], "oldest");
    }

    #[test]
    fn search_page_args_sets_the_offset() {
        let base = serde_json::json!({"from": "a@x.com", "limit": 25});
        let page = search_page_args(&base, 50);
        assert_eq!(page["offset"], 50);
        assert_eq!(page["from"], "a@x.com");
    }

    #[test]
    fn merge_alternates_dedupes_and_caps() {
        let merged = merge_candidates(&ids(&["s1", "s2", "x"]), &ids(&["r1", "x", "r2", "r3"]), 5);
        assert_eq!(merged, ids(&["s1", "r1", "s2", "x", "r2"]));
    }

    #[test]
    fn merge_with_one_side_empty_keeps_the_other() {
        assert_eq!(merge_candidates(&[], &ids(&["r1", "r2"]), 10), ids(&["r1", "r2"]));
        assert_eq!(merge_candidates(&ids(&["s1"]), &[], 10), ids(&["s1"]));
    }

    // ── batches ──

    #[test]
    fn batches_respect_the_char_budget() {
        assert_eq!(plan_batches(&[400, 400, 400, 400], 1000, 10), vec![0..2, 2..4]);
    }

    #[test]
    fn batches_respect_the_count_cap() {
        assert_eq!(plan_batches(&[10; 5], 10_000, 2), vec![0..2, 2..4, 4..5]);
    }

    #[test]
    fn an_oversized_document_gets_its_own_batch() {
        assert_eq!(plan_batches(&[100, 5000, 100], 1000, 10), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn no_documents_no_batches() {
        assert!(plan_batches(&[], 1000, 10).is_empty());
    }

    // ── map prompt / notes ──

    fn doc(id: &str) -> ResearchDoc {
        ResearchDoc {
            id: id.into(),
            date: "2026-09-01".into(),
            from: "Alice <alice@example.com>".into(),
            subject: "Invoice 42".into(),
            body: "Please pay by Friday.".into(),
        }
    }

    #[test]
    fn map_prompt_keeps_the_batch_out_of_the_prefix() {
        let tmpl = "Extract findings.\n\nQUESTION: {{question}}\n\nEMAILS:\n{{emails}}";
        let (prefix, suffix) = split_map_prompt(tmpl, "¿qué facturas?", &[doc("e1"), doc("e2")]);
        assert_eq!(prefix, "Extract findings.\n\n");
        assert!(suffix.starts_with("QUESTION: ¿qué facturas?"));
        assert!(suffix.contains("EMAIL_ID: e1") && suffix.contains("EMAIL_ID: e2"));
        // The prefix is identical for another question and batch.
        let (other, _) = split_map_prompt(tmpl, "other", &[doc("e9")]);
        assert_eq!(prefix, other);
    }

    #[test]
    fn map_prompt_without_marker_still_renders_everything() {
        let (prefix, suffix) = split_map_prompt("Q={{question}} E={{emails}}", "q", &[doc("e1")]);
        assert!(prefix.contains("Q=q") && prefix.contains("EMAIL_ID: e1"));
        assert!(suffix.is_empty());
    }

    #[test]
    fn notes_keep_only_findings_citing_a_batch_email() {
        let reply = "Here are the findings:\n\
- Alice asks to pay invoice 42 by Friday (email://e1)\n\
* Bob confirms the refund (email://e2) (email://e1)\n\
- Something about email://zzz\n\
NONE";
        let notes = parse_map_notes(reply, &ids(&["e1", "e2"]));
        assert_eq!(
            notes.lines,
            vec![
                "- Alice asks to pay invoice 42 by Friday (email://e1)".to_string(),
                "- Bob confirms the refund (email://e2) (email://e1)".to_string(),
            ]
        );
        assert_eq!(notes.cited, ids(&["e1", "e2"]));
    }

    #[test]
    fn a_none_reply_yields_no_notes() {
        assert_eq!(parse_map_notes("NONE", &ids(&["e1"])), BatchNotes::default());
        assert_eq!(parse_map_notes("", &ids(&["e1"])), BatchNotes::default());
    }

    #[test]
    fn notes_fit_whole_when_under_budget() {
        let b = vec![
            BatchNotes {
                lines: vec!["- a (email://1)".into()],
                cited: ids(&["1"]),
            },
            BatchNotes {
                lines: vec!["- b (email://2)".into()],
                cited: ids(&["2"]),
            },
        ];
        assert_eq!(assemble_notes(&b, 1000), "- a (email://1)\n- b (email://2)\n");
    }

    #[test]
    fn overflowing_notes_keep_a_share_of_every_batch() {
        let batch = |tag: &str| BatchNotes {
            lines: (0..10).map(|i| format!("- {tag}{i} ..........")).collect(),
            cited: vec![],
        };
        let notes = assemble_notes(&[batch("a"), batch("b")], 100);
        assert!(notes.chars().count() <= 100, "{notes}");
        assert!(notes.contains("- a0") && notes.contains("- b0"), "{notes}");
        assert!(!notes.contains("- a9"));
    }

    #[test]
    fn reduce_prompt_keeps_per_turn_content_out_of_the_prefix() {
        let tmpl = "Write the report. {{language_instruction}}\n\nQUESTION: {{question}}\nCOVERAGE: {{coverage}}\nNOTES:\n{{notes}}";
        let (prefix, suffix) = split_reduce_prompt(tmpl, "Reply in Spanish.", "q?", "read 10", "- n (email://1)");
        assert_eq!(prefix, "Write the report. Reply in Spanish.\n\n");
        assert!(suffix.contains("q?") && suffix.contains("read 10") && suffix.contains("email://1"));
    }

    #[test]
    fn the_default_prompts_split_on_their_markers() {
        use crate::services::prompts::defaults::{CHAT_RESEARCH_MAP, CHAT_RESEARCH_REDUCE};
        let (prefix, suffix) = split_map_prompt(CHAT_RESEARCH_MAP, "Q?", &[doc("e1")]);
        assert!(!prefix.contains("Q?") && !prefix.contains("e1"));
        assert!(suffix.contains("Q?") && suffix.contains("EMAIL_ID: e1"));
        assert!(!suffix.contains("{{"), "unrendered placeholder: {suffix}");

        let (prefix, suffix) = split_reduce_prompt(
            CHAT_RESEARCH_REDUCE,
            "Reply in Spanish.",
            "Q?",
            "read 5",
            "- n (email://e1)",
        );
        assert!(prefix.contains("Reply in Spanish."));
        for per_turn in ["Q?", "read 5", "email://e1)"] {
            assert!(!prefix.contains(per_turn), "{per_turn} leaked into the cached prefix");
            assert!(suffix.contains(per_turn));
        }
        assert!(!prefix.contains("{{") && !suffix.contains("{{"));
    }

    fn subjects() -> HashMap<String, String> {
        HashMap::from([
            ("e1".to_string(), "Invoice 42".to_string()),
            ("e2".to_string(), "Re: [Q3] refund ]".to_string()),
        ])
    }

    #[test]
    fn relink_turns_bracketed_ids_into_subject_links() {
        let out = relink_bare_refs("Pay by Friday [email://e1].", &subjects());
        assert_eq!(out, "Pay by Friday [Invoice 42](email://e1).");
    }

    #[test]
    fn relink_turns_parenthesised_ids_into_subject_links() {
        let out = relink_bare_refs("Pay by Friday (email://e1) and refund (email://e2)", &subjects());
        assert_eq!(
            out,
            "Pay by Friday [Invoice 42](email://e1) and refund [Re: Q3 refund](email://e2)"
        );
    }

    #[test]
    fn relink_leaves_proper_links_and_unknown_ids_alone() {
        let text = "See [the invoice](email://e1), and [email://zzz].";
        assert_eq!(relink_bare_refs(text, &subjects()), text);
    }

    #[test]
    fn relink_labels_an_email_without_subject_generically() {
        let map = HashMap::from([("e3".to_string(), "   ".to_string())]);
        assert_eq!(relink_bare_refs("x [email://e3]", &map), "x [email](email://e3)");
    }

    #[test]
    fn coverage_mentions_failed_batches_only_when_some_failed() {
        assert_eq!(
            coverage_line(40, 12, 4, 0),
            "read 40 emails in 4 batches; 12 of them had relevant findings"
        );
        assert!(coverage_line(40, 12, 4, 1).ends_with("(1 batches could not be read)"));
    }

    // ── planner verdicts ──

    #[test]
    fn research_survives_every_verdict_but_a_form_fill() {
        // "How have EmailOps downloads evolved?" reads to the planner as a
        // question about the app; with the toggle on it is about mail.
        assert!(research_continues(PlannerVerdict::AppHelp));
        // "Create a lens for supplier invoices" is an action, not a question.
        assert!(!research_continues(PlannerVerdict::FormFill));
    }

    // ── context window ──

    #[test]
    fn n_ctx_follows_the_override_then_the_ram_tier_on_llamacpp() {
        use crate::ai::provider::ProviderType;
        assert_eq!(plan_n_ctx(ProviderType::LlamaCpp, 12288, 32768), 12288);
        assert_eq!(plan_n_ctx(ProviderType::LlamaCpp, 0, 16384), 16384);
        assert_eq!(plan_n_ctx(ProviderType::Ollama, 32768, 32768), DEFAULT_N_CTX);
        assert_eq!(plan_n_ctx(ProviderType::OpenRouter, 0, 32768), DEFAULT_N_CTX);
    }

    // ── executor (fake provider + in-memory DB) ──

    fn seed(db: &Database, n: usize) {
        use rusqlite::params;
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
             VALUES ('acct', 'gmail', 'me@example.com', 'Me', 0)",
            [],
        )
        .unwrap();
        for i in 0..n {
            let id = format!("e{i:02}");
            conn.execute(
                "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                 VALUES (?1,'acct',?2,?3,'Supplier','billing@supplier.example','supplier.example',
                         '[]','[]','snip',?4,0,'primary',0)",
                params![id, format!("t{i:02}"), format!("Invoice {i}"), 1_780_000_000 + i as i64],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1,?2,'Supplier','Invoice due')",
                params![id, format!("Invoice {i}")],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO email_bodies(email_id, body) VALUES (?1, ?2)",
                params![id, format!("Invoice {i} for 100 EUR is due on Friday.")],
            )
            .unwrap();
        }
    }

    fn input<'a>(
        db: &'a Arc<Database>,
        provider: &'a dyn AIProvider,
        plan: Option<&'a SearchPlan>,
        categories: &'a [String],
    ) -> ResearchInput<'a> {
        ResearchInput {
            db,
            provider,
            account_id: "acct",
            categories,
            question: "¿Qué facturas me ha enviado el proveedor?",
            plan,
            n_ctx: 8192,
            language_instruction: "Reply in Spanish.",
            map_template: crate::services::prompts::defaults::CHAT_RESEARCH_MAP,
            reduce_template: crate::services::prompts::defaults::CHAT_RESEARCH_REDUCE,
        }
    }

    #[tokio::test]
    async fn research_reads_every_page_in_batches_and_reports_from_the_notes() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 30);
        let provider = crate::ai::provider::FakeAiProvider::new();
        // Three batches of 10: only the batch holding e29 (newest-first → the
        // first batch) can keep a finding that cites it.
        for _ in 0..3 {
            provider.push_completion("Findings:\n- Invoice 29 for 100 EUR is due Friday (email://e29)");
        }
        provider.push_completion("Tienes 30 facturas; la más reciente es [Invoice 29](email://e29).");
        let plan = SearchPlan {
            from: Some("billing@supplier.example".into()),
            ..Default::default()
        };
        let categories: Vec<String> = Vec::new();
        let events = std::sync::Mutex::new(Vec::new());
        let run = run_research(input(&db, &provider, Some(&plan), &categories), &|p| {
            events.lock().unwrap().push(p)
        })
        .await;

        assert_eq!(run.analyzed.len(), 30, "all three search pages were read");
        assert_eq!(run.trace.search_hits, 30);
        assert_eq!(run.trace.retrieval_hits, 0, "a structural filter skips retrieval");
        assert_eq!(run.trace.batches, 3);
        assert_eq!(run.trace.failed_batches, 0);
        assert_eq!(run.relevant, vec!["e29".to_string()]);
        assert_eq!(run.trace.findings, 1);
        assert_eq!(
            run.answer.as_deref(),
            Some("Tienes 30 facturas; la más reciente es [Invoice 29](email://e29).")
        );
        assert_eq!(
            run.llm_calls.iter().map(|c| c.kind.as_str()).collect::<Vec<_>>(),
            ["research_map", "research_map", "research_map", "research_reduce"]
        );

        let calls = provider.prefix_completion_calls();
        assert_eq!(calls.len(), 4);
        assert!(
            calls[..3].iter().all(|(prefix, _)| prefix == &calls[0].0),
            "map prefix is invariant"
        );
        assert!(calls[3].1.contains("email://e29"), "the reduce reads the notes");

        let events = events.into_inner().unwrap();
        assert_eq!(events.first().map(|e| e.stage), Some(ResearchStage::Gathering));
        assert_eq!(events.last().map(|e| e.stage), Some(ResearchStage::Writing));
        assert!(events
            .iter()
            .any(|e| e.stage == ResearchStage::Reading && e.batch == 3 && e.emails_read == 30));
    }

    #[tokio::test]
    async fn a_failing_provider_leaves_no_answer_but_does_not_panic() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed(&db, 5);
        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.fail_completions(Some("model crashed"));
        let plan = SearchPlan {
            from: Some("billing@supplier.example".into()),
            ..Default::default()
        };
        let categories: Vec<String> = Vec::new();
        let run = run_research(input(&db, &provider, Some(&plan), &categories), &|_| {}).await;
        assert_eq!(run.trace.batches, 1);
        assert_eq!(run.trace.failed_batches, 1);
        assert!(run.answer.is_none());
        assert!(
            run.error.as_deref().unwrap_or("").contains("model crashed"),
            "{:?}",
            run.error
        );
    }
}
