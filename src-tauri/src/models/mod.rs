use serde::{Deserialize, Serialize};

// ts-rs is optional (feature "ts"). Suppress the "unused import" warning when
// the feature is off — the cfg_attr macros reference it only when enabled.
#[cfg(feature = "ts")]
use ts_rs::TS;

pub mod error;
pub mod lens;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub provider: String,
    pub email: String,
    pub name: String,
    pub created_at: i64,
    pub sort_order: i32,
    pub enabled: bool,
    pub sync_from_timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct Email {
    pub id: String,
    pub account_id: String,
    pub thread_id: String,
    pub message_id: Option<String>,
    pub subject: String,
    pub sender: String,
    pub sender_email: String,
    pub recipients: Vec<String>,
    pub cc: Vec<String>,
    pub body: String,
    pub snippet: String,
    pub timestamp: i64,
    pub is_read: bool,
    pub triage_status: Option<String>,
    pub category: String, // primary, social, updates, forums, promotions
    /// Provider mailbox: 'inbox' | 'sent' | 'spam' | 'trash'. Defaults to 'inbox'
    /// for emails ingested before the column was added.
    #[serde(default = "default_mailbox")]
    pub mailbox: String,
}

fn default_mailbox() -> String {
    "inbox".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub account_id: String,
    pub status: String,
    pub last_sync_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

// Logging

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLogEvent {
    pub level: String,
    pub source: String,
    pub message: String,
}

// Smart filter types

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterSuggestion {
    pub value: String,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickFilterStats {
    pub top_domains: Vec<FilterSuggestion>,
    pub top_senders: Vec<FilterSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartFilterPref {
    pub id: String,
    pub filter_type: String,
    pub filter_value: String,
    pub status: String,
    pub account_id: String,
}

/// A suggestion stored in the DB (filter_type + filter_value + count)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartFilterSuggestion {
    pub filter_type: String,
    pub filter_value: String,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilteredEmailsResult {
    pub emails: Vec<Email>,
    pub total_count: i32,
}

// Attachment types

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRule {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub sender_email_pattern: Option<String>,
    pub subject_pattern: Option<String>,
    pub filename_pattern: Option<String>,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub account_id: String,
    pub email_id: String,
    pub rule_id: String,
    pub gmail_attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub file_size: i64,
    pub file_path: String,
    pub tags: Vec<String>,
    pub sender_email: String,
    pub subject: String,
    pub email_timestamp: i64,
    pub created_at: i64,
}

// Email classification tags

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailTag {
    pub email_id: String,
    pub tag_type: String,
    pub tag_value: String,
    pub confidence: Option<f64>,
    pub created_at: i64,
}

// Classification rules (user-configurable)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationRule {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub sender_pattern: Option<String>,
    pub subject_pattern: Option<String>,
    pub priority: String,
    pub intent: String,
    pub topic: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

// Attachment metadata for all attachments discovered during sync

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailAttachmentMeta {
    pub id: String,
    pub email_id: String,
    pub account_id: String,
    pub provider_attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub file_size: i64,
    /// Set when the attachment has been downloaded to disk (auto-download or on-demand save).
    pub file_path: Option<String>,
}

// Account settings (per-account user preferences stored in user_preferences table)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSettings {
    /// Gmail inbox categories to sync. Ignored for IMAP accounts.
    /// Defaults to ["primary"] only — users opt into noisier categories
    /// (Updates / Promotions / Social / Forums) via the account settings dialog.
    pub gmail_categories: Vec<String>,
    /// Categories for which attachments are downloaded to disk during sync.
    /// Empty by default — attachments are only fetched on demand.
    #[serde(default)]
    pub auto_download_attachment_categories: Vec<String>,
}

impl Default for AccountSettings {
    fn default() -> Self {
        Self {
            // Conservative default: only Primary. Pulling Updates by default
            // surprised users (newsletters/notifications appearing in the
            // inbox even though only "Primary" was checked in the inbox
            // header filter — that header is a client-side view filter,
            // distinct from this server-side download filter).
            gmail_categories: vec!["primary".to_string()],
            auto_download_attachment_categories: vec![],
        }
    }
}

// Contact type

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub email: String,
    pub name: String,
    /// Total emails (received + sent). Kept for backward compat with the chat
    /// `search_contacts` tool, which surfaces "count" in its result preview.
    pub email_count: i32,
    /// Unix timestamp (seconds) of the most recent inbound or outbound email
    /// with this contact. Always populated by `list_contacts`; `None` only on
    /// the legacy `get_contacts` path.
    #[serde(default)]
    pub last_timestamp: Option<i64>,
    /// Inbound count (emails from this contact to the user).
    #[serde(default)]
    pub received_count: i32,
    /// Outbound count (emails the user sent that included this contact in
    /// `to`/`cc`).
    #[serde(default)]
    pub sent_count: i32,
    /// Earliest inbound or outbound timestamp.
    #[serde(default)]
    pub first_timestamp: Option<i64>,
    /// Most-frequent company tag observed on this contact's emails (matches
    /// `email_tags.tag_value` where `tag_type = 'company'`). TLD-stripped
    /// stem, e.g. `acme`. `None` when the contact has no tagged emails.
    #[serde(default)]
    pub company: Option<String>,
    /// Heuristic classification: "person" | "automated".
    #[serde(default)]
    pub kind: String,
    /// Email domain (lowercased, stripped of trailing punctuation).
    #[serde(default)]
    pub domain: String,
    /// Relationship strength score in [0, 100]. Blends frequency, recency, and
    /// bidirectionality so frequent two-way correspondents float to the top.
    #[serde(default)]
    pub relationship_score: f64,
}

/// Filter/sort/page parameters for `list_contacts`. The frontend builds this
/// from its toolbar state; missing fields fall back to sensible defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsQuery {
    /// Free-text search; tokenised with the same logic as `search_contacts`.
    #[serde(default)]
    pub search: Option<String>,
    /// "person" | "automated" | "all" (default).
    #[serde(default)]
    pub kind: Option<String>,
    /// Filter to a specific company tag (TLD-stripped stem).
    #[serde(default)]
    pub company: Option<String>,
    /// Filter to a specific email domain (with TLD).
    #[serde(default)]
    pub domain: Option<String>,
    /// Sort key: "last" | "total" | "received" | "sent" | "name" | "score".
    #[serde(default)]
    pub sort: Option<String>,
    /// Page offset (0-based).
    #[serde(default)]
    pub offset: Option<i32>,
    /// Page size; capped at 200 server-side.
    #[serde(default)]
    pub limit: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsPage {
    pub items: Vec<Contact>,
    pub total: i32,
    pub has_more: bool,
}

/// Detail view payload for the contact drawer header (Phase 3.1) plus quick
/// actions (Phase 3.5). Aliases are inferred from emails that share this
/// contact's display name.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDetail {
    pub contact: Contact,
    /// Other email addresses observed under the same display name. Empty when
    /// the contact's name is missing or when no other addresses match.
    pub aliases: Vec<String>,
}

/// One company group for the by-company view (Phase 4.5).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyContactsGroup {
    /// Company tag stem (e.g. "acme") or `None` for contacts with no company.
    pub company: Option<String>,
    pub contacts: Vec<Contact>,
    /// Total emails across all contacts in this group.
    pub total_emails: i32,
    /// Latest interaction across the group.
    pub last_timestamp: Option<i64>,
}

// Draft types

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: String,
    pub email_id: Option<String>,
    pub account_id: String,
    pub to_addresses: Vec<String>,
    pub subject: String,
    pub body: String,
    pub ai_generated: bool,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveDraftRequest {
    pub id: Option<String>,
    pub email_id: Option<String>,
    pub account_id: String,
    pub to_addresses: Vec<String>,
    pub subject: String,
    pub body: String,
}

// AI config and usage types

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    pub provider: String,
    pub model: String,
    pub embedding_model: String,
    pub api_key_id: Option<String>,
    pub monthly_budget_usd: f64,
    pub period_start: i64,
    pub thinking_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageSummary {
    pub total_cost_usd: f64,
    pub total_prompt_tokens: u32,
    pub total_completion_tokens: u32,
    pub total_calls: u32,
    pub period_start: i64,
    pub budget_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiLogEvent {
    pub provider: String,
    pub model: String,
    pub operation: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub status: String,
    pub timestamp: i64,
}

// Chat-with-your-emails types

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConversation {
    pub id: String,
    pub account_id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageSource {
    pub citation_number: i32,
    pub email_id: String,
    pub relevance_score: Option<f32>,
    /// Denormalized email metadata so the UI can render sources without extra queries.
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub sender_email: String,
    #[serde(default)]
    pub timestamp: i64,
    /// The actual chunk of body text that was fed to the LLM for this citation.
    /// Persisted so the "sources used" panel can show what the model saw, not just
    /// the email metadata. `None` for pre-migration rows.
    #[serde(default)]
    pub body_excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String, // "user" | "assistant" | "system"
    pub content: String,
    pub model: Option<String>,
    pub token_count: Option<i32>,
    pub latency_ms: Option<i64>,
    pub created_at: i64,
    #[serde(default)]
    pub sources: Vec<ChatMessageSource>,
    /// Reasoning trace (routing decision, retrieval stats, tool calls) assembled
    /// during `run_chat_turn`. Only populated on assistant messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<ChatTrace>,
    /// Email IDs the chat-turn tools handed to the LLM. Used by the
    /// frontend as an allowlist when rendering `email://EMAIL_ID` markdown
    /// links — links pointing at ids not in this list are dropped (and a
    /// warning logged) so hallucinated references don't open arbitrary
    /// emails. Empty on user / system rows and on pre-migration rows.
    #[serde(default)]
    pub referenced_email_ids: Vec<String>,
    /// Same shape for draft ids — the allowlist for `draft://DRAFT_ID`
    /// "Re-open draft" chips. Populated when a turn touches the drafts
    /// table (generate_email_draft / list_drafts).
    #[serde(default)]
    pub referenced_draft_ids: Vec<String>,
}

// ── Reasoning trace ────────────────────────────────────────────────────────

/// Which retrieval/tool path the router picked for a given chat turn.
///
/// The router is strictly *one path or the other* — never both. This avoids
/// the failure mode where RAG retrieves stale sources AND tools are exposed,
/// confusing the model into ignoring the sources and re-searching anyway.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    /// Run RAG retrieval, answer from sources only. No tools exposed.
    RagFirst,
    /// Skip RAG, go straight to the tool loop. Sources are whatever the
    /// model retrieves via tool calls this turn.
    ToolsFirst,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteDecision {
    pub mode: RouteMode,
    /// Short human-readable explanation (e.g. "heuristic: keyword 'latest' → tools_first").
    pub reason: String,
    /// Tokens/phrases the heuristic matched. Empty for LLM-routed decisions.
    #[serde(default)]
    pub matched_keywords: Vec<String>,
    /// Which classifier actually made the decision.
    pub classifier: String, // "heuristic" | "llm" | "forced"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalTrace {
    pub vector_hits: i32,
    pub fts_hits: i32,
    pub fused_top_k: i32,
    pub elapsed_ms: i64,
    /// True if vector search fell back to FTS-only (timeout or error).
    #[serde(default)]
    pub vector_fallback: bool,
    /// Gmail categories that were searched this turn. Empty = all categories.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Number of candidates collapsed away by thread-dedup (informational).
    #[serde(default)]
    pub thread_dedup_collapsed: i32,
    /// Sub-step timings (ms). `None` when the step didn't run (e.g. embedding
    /// skipped on vector fallback). Lets the UI render a per-step timeline
    /// instead of just the retrieval total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vec_search_ms: Option<i64>,
    #[serde(default)]
    pub fts_search_ms: i64,
    #[serde(default)]
    pub fetch_ms: i64,
    #[serde(default)]
    pub expansion_ms: i64,
    /// Milliseconds spent in the optional LLM reranker stage. `None` when
    /// reranking was skipped (candidate pool too small or disabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rerank_ms: Option<i64>,
    /// True when the reranker exceeded its budget and we fell back to pure
    /// RRF ordering. Surfaced so the eval report can show "why did the
    /// order not change?".
    #[serde(default)]
    pub rerank_timed_out: bool,
    /// Milliseconds spent in the query-rewrite / HyDE pre-step. `None` when
    /// skipped (short query or timeout).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_rewrite_ms: Option<i64>,
    /// The expanded retrieval query after HyDE/rewrite. Empty = original query used.
    #[serde(default)]
    pub expanded_query: String,
    /// Number of citations in the final answer that referenced a source
    /// number outside the retrieved set (hallucinated citations). Populated
    /// by the post-stream validator; 0 = all citations valid, -1 = not run.
    #[serde(default = "default_cite_check")]
    pub invalid_citations: i32,
}

fn default_cite_check() -> i32 {
    -1
}

/// One round-trip to the LLM. Emitted for every `chat_with_tools` round in the
/// tool loop plus the final streaming call, so the UI timeline can show
/// per-call latencies separately from tool-execution time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmCallTrace {
    /// `"tool_round"` for each chat_with_tools call, `"final_stream"` for the
    /// streaming synthesis call.
    pub kind: String,
    /// Round index within the tool loop (0-based). `-1` for the final stream.
    pub round: i32,
    pub latency_ms: i64,
    /// How many tool calls the model asked for in this round. 0 for plain
    /// text answers or for the final stream.
    #[serde(default)]
    pub tool_calls_requested: i32,
    /// True when the call returned an error / timed out.
    #[serde(default)]
    pub failed: bool,
    /// Messages sent to the LLM at this round, formatted for tracing.
    /// Only populated when the `tracing` feature is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// Model's response: text content and/or tool-call JSON.
    /// Only populated when the `tracing` feature is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallTrace {
    pub name: String,
    /// Tool-loop round that issued this call (0-based). `-1` for preseeded
    /// shortcut tools that run before the LLM loop. Lets the UI render tool
    /// and LLM calls in true execution order. `#[serde(default)]` keeps old
    /// persisted traces (which lack the field) deserialising to `0`.
    #[serde(default)]
    pub round: i32,
    /// JSON arguments as sent to the tool.
    pub arguments: serde_json::Value,
    /// Truncated copy of the tool result (currently 16 KiB). Large enough for
    /// the eval report to show the full typical search_emails / get_thread
    /// output, small enough to bound the persisted trace JSON.
    pub result_preview: String,
    pub result_chars: i32,
    pub elapsed_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTrace {
    pub route: RouteDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval: Option<RetrievalTrace>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallTrace>,
    pub model: String,
    pub total_elapsed_ms: i64,
    /// Wall-clock time spent inside the tool-call loop (all rounds combined),
    /// including the model round-trips between tool calls.
    #[serde(default)]
    pub tool_loop_ms: i64,
    /// Time spent streaming the final assistant answer. `None` when the model
    /// answered synchronously inside the tool loop (no follow-up stream).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_streaming_ms: Option<i64>,
    /// Per-LLM-call timing breakdown — one entry per tool-round chat_with_tools
    /// call plus the final stream, so the UI can show each individually.
    #[serde(default)]
    pub llm_calls: Vec<LlmCallTrace>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamEvent {
    pub message_id: String,
    pub conversation_id: String,
    pub token: String,
    pub done: bool,
    /// Set on the final event when a fatal error occurred; the assistant row
    /// content will also be replaced with the error text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Completion token count (from Ollama eval_count). Only set when `done == true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<i32>,
    /// Wall-clock latency of the full turn (retrieval + streaming) in ms. Only set when `done == true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSourcesEvent {
    pub message_id: String,
    pub conversation_id: String,
    pub sources: Vec<ChatMessageSource>,
}

/// Coarse processing stage of an in-flight chat turn. Emitted on the
/// `chat-phase` event at each stage boundary so the UI can show an LM
/// Studio-style "Processing…" status before any answer tokens stream — the
/// most common "what is it doing / is it stuck?" question. Ordered by when
/// each stage occurs in `run_chat_turn`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatPhase {
    /// Classifying the route (RAG-first vs tools-first).
    Routing,
    /// Retrieving relevant emails (hybrid vector + FTS).
    Retrieving,
    /// Looking up contacts (`search_contacts`).
    SearchingContacts,
    /// Searching the mailbox (`search_emails`).
    SearchingEmails,
    /// Fetching a specific email or thread body (`get_email_body` / `get_thread`).
    RetrievingEmail,
    /// Generating an email draft (`generate_email_draft`).
    GeneratingDraft,
    /// Running any other tool in the loop — generic fallback when no
    /// tool-specific phase applies.
    RunningTools,
    /// Streaming the final assistant answer.
    Generating,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatPhaseEvent {
    pub message_id: String,
    pub conversation_id: String,
    pub phase: ChatPhase,
}

/// Emitted when the backend auto-derives a conversation title from the first
/// user turn, so the sidebar updates live.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRenamedEvent {
    pub conversation_id: String,
    pub title: String,
}

/// Emitted when a chat turn's reasoning trace is finalized. The UI appends
/// this to the assistant message and renders it as a collapsible section.
/// Also carries the structural email-ref allowlist — same fire-and-forget
/// delivery as the trace, so MessageBubble has both signals available the
/// moment streaming concludes. (The refs aren't *reasoning data* per se,
/// but the trace event is the natural last-event hook in the turn and
/// adding a parallel `chat-email-refs` event would duplicate plumbing for
/// a one-shot delivery.)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTraceEvent {
    pub message_id: String,
    pub conversation_id: String,
    pub trace: ChatTrace,
    /// Email IDs aggregated across every tool call in this turn. Frontend
    /// uses it as an allowlist when rendering `email://EMAIL_ID` markdown
    /// links the LLM emitted. Empty when no tools ran (RagFirst route).
    #[serde(default)]
    pub referenced_email_ids: Vec<String>,
    /// Same shape for draft ids — feeds the `draft://DRAFT_ID` chip
    /// validator.
    #[serde(default)]
    pub referenced_draft_ids: Vec<String>,
}

// ── Memory subsystem ───────────────────────────────────────────────────────

/// A durable fact stored in agent memory. Candidates are promoted to
/// `status="promoted"` by the consolidation job once their score crosses the
/// threshold; low-score candidates are retired after a TTL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFact {
    pub id: String,
    pub account_id: String,
    /// "user" | "contact" | "domain" | "project"
    pub subject_kind: String,
    pub subject_key: String,
    pub fact: String,
    /// "extraction" | "user" | "consolidation"
    pub source: String,
    pub source_email_id: Option<String>,
    pub confidence: f64,
    pub score: f64,
    /// "candidate" | "promoted" | "retired"
    pub status: String,
    pub last_used_at: Option<i64>,
    /// Life-context classification: "personal" | "professional". `None` when
    /// the extractor didn't classify (e.g. older rows, user-added facts).
    pub domain: Option<String>,
    /// Temporal classification: "atemporal" (stable over months/years, e.g.
    /// role, preference, project) | "deciduous" (useful only briefly, e.g.
    /// "out of office next Tuesday").
    pub vigency: Option<String>,
    /// Company tag derived from the recipient domain stem (TLD stripped,
    /// lower-cased). When an email has multiple recipients, the most frequent
    /// domain wins. `None` for facts extracted before this field existed or
    /// when no recipient domain could be determined.
    pub company: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Per-thread computed state. One row per (account, thread).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadState {
    pub account_id: String,
    pub thread_id: String,
    /// "user" | "them" | "resolved" | "unknown"
    pub awaiting: String,
    pub last_inbound_at: Option<i64>,
    pub last_outbound_at: Option<i64>,
    pub last_touched_at: i64,
    pub summary: Option<String>,
    pub commitment: Option<String>,
    pub deadline_at: Option<i64>,
    pub participants: Vec<String>,
    pub updated_at: i64,
}

/// Action item persisted in the memory subsystem. Surfaced in the Tasks panel
/// and to the agent via the list_pending_tasks tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTask {
    pub id: String,
    pub account_id: String,
    pub title: String,
    pub detail: Option<String>,
    /// "extracted" | "chat" | "user"
    pub source: String,
    pub source_email_id: Option<String>,
    pub source_thread_id: Option<String>,
    /// "me" or an email address.
    pub assignee: String,
    /// "open" | "done" | "snoozed" | "dismissed"
    pub status: String,
    /// "low" | "normal" | "high"
    pub priority: String,
    pub due_at: Option<i64>,
    pub completed_at: Option<i64>,
    /// Company tag derived from the recipient domain stem of the source email.
    /// Parallels `MemoryFact.company` and drives the task view quick-filter.
    pub company: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One row in the `interaction_events` log. Short-lived; the dream job prunes
/// rows older than 30 days.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionEvent {
    pub id: i64,
    pub account_id: String,
    /// "read" | "reply" | "draft" | "tag" | "search" | "chat_turn" | "archive"
    pub kind: String,
    pub email_id: Option<String>,
    pub thread_id: Option<String>,
    /// Free-form JSON payload; shape depends on `kind`.
    pub payload_json: Option<String>,
    pub created_at: i64,
}

// ── Tag priority ───────────────────────────────────────────────────────────

/// Priority scoring for a tag (per account). Raw signals are persisted in the
/// `tag_priority` table; `priority_score` is computed at read time by
/// `services::tag_priority::get_priorities` so the formula can be tuned
/// without a migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagPriority {
    pub tag_type: String,
    pub tag_value: String,
    pub sent_count: i32,
    pub received_count: i32,
    pub last_activity_at: Option<i64>,
    pub priority_score: f64,
}

/// Write-side input for a new pending task. Separate from `PendingTask` so
/// callers don't fabricate id/timestamps and to keep the Tauri command
/// surface small.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRequest {
    pub account_id: String,
    pub title: String,
    pub detail: Option<String>,
    pub priority: Option<String>,
    pub due_at: Option<i64>,
    pub source_email_id: Option<String>,
    pub source_thread_id: Option<String>,
    /// Defaults to "user" when omitted.
    pub source: Option<String>,
    /// Optional company tag; when the caller omits it we fall back to deriving
    /// from the source email (or leave it `NULL` for chat-originated tasks).
    pub company: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_phase_serializes_as_camel_case_strings() {
        // The frontend `ChatPhase` union and the i18n `chat:processing.*` keys
        // are keyed off these exact wire values — keep them in lockstep.
        assert_eq!(serde_json::to_value(ChatPhase::Routing).unwrap(), "routing");
        assert_eq!(serde_json::to_value(ChatPhase::Retrieving).unwrap(), "retrieving");
        assert_eq!(
            serde_json::to_value(ChatPhase::SearchingContacts).unwrap(),
            "searchingContacts"
        );
        assert_eq!(
            serde_json::to_value(ChatPhase::SearchingEmails).unwrap(),
            "searchingEmails"
        );
        assert_eq!(
            serde_json::to_value(ChatPhase::RetrievingEmail).unwrap(),
            "retrievingEmail"
        );
        assert_eq!(
            serde_json::to_value(ChatPhase::GeneratingDraft).unwrap(),
            "generatingDraft"
        );
        assert_eq!(serde_json::to_value(ChatPhase::RunningTools).unwrap(), "runningTools");
        assert_eq!(serde_json::to_value(ChatPhase::Generating).unwrap(), "generating");
    }

    #[test]
    fn chat_phase_event_uses_camel_case_field_names() {
        let event = ChatPhaseEvent {
            message_id: "m1".into(),
            conversation_id: "c1".into(),
            phase: ChatPhase::Generating,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["messageId"], "m1");
        assert_eq!(json["conversationId"], "c1");
        assert_eq!(json["phase"], "generating");
    }
}
