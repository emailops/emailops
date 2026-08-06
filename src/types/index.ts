export type InboxLayout = 'split' | 'full-width';

export interface Account {
  id: string;
  provider: 'gmail' | 'imap';
  email: string;
  name: string;
  createdAt: number;
  sortOrder: number;
  enabled: boolean;
  syncFromTimestamp: number | null;
}

export interface AccountSettings {
  gmailCategories: string[];
  /** Categories for which attachments are downloaded to disk during sync. Empty = on-demand only. */
  autoDownloadAttachmentCategories: string[];
}

export interface EmailAttachmentMeta {
  id: string;
  emailId: string;
  accountId: string;
  providerAttachmentId: string;
  filename: string;
  mimeType: string;
  fileSize: number;
  /** Set when the file has been saved to disk (auto-download or previously downloaded). */
  filePath: string | null;
}

export interface Email {
  id: string;
  accountId: string;
  threadId: string;
  messageId: string | null;
  subject: string;
  sender: string;
  senderEmail: string;
  recipients: string[];
  cc: string[];
  body: string;
  snippet: string;
  timestamp: number;
  isRead: boolean;
  triageStatus: TriageStatus | null;
  category: EmailCategory;
  /** 'inbox' | 'sent' | 'spam' | 'trash' | `folder:<serverPath>` — which
   *  mailbox this email lives in (drives move-to-folder eligibility). */
  mailbox: string;
  /** The provider filed this under Sent. Independent of `mailbox`: Gmail
   *  labels self-sent mail INBOX *and* SENT, so `mailbox` reads 'inbox'. */
  isSent: boolean;
}

export type TriageStatus = 'action_needed' | 'fyi' | 'low_priority';

/** One attendee of a calendar event with their RSVP state. `response` is an
 *  open set — normalize through `attendeeStatusMeta` before mapping to UI. */
export interface CalendarAttendee {
  email: string;
  /** 'accepted' | 'declined' | 'tentative' | 'needsAction' | 'organizer' | provider-specific. */
  response: string;
}

/** One calendar an account can see — its own, shared with it, or subscribed.
 *  Mirrors the Rust `Calendar` struct in `src-tauri/src/models/mod.rs`.
 *  Refreshed from the provider on every sync except `isVisible`, which is the
 *  local show/hide toggle. */
export interface Calendar {
  id: string;
  accountId: string;
  /** Provider calendar id: an address on Google, an opaque id on Graph. */
  providerCalendarId: string;
  name: string;
  /** Provider colour as "#rrggbb", or '' when the provider reported none —
   *  resolve through `calendarColor()` rather than reading this directly. */
  color: string;
  isPrimary: boolean;
  /** 'owner' | 'writer' | 'reader' | 'freeBusyReader' */
  accessRole: string;
  isVisible: boolean;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

/** A calendar event synced from Gmail / Outlook. Mirrors the Rust
 *  `CalendarEvent` struct in `src-tauri/src/models/mod.rs` (camelCase serde).
 *  `description` may contain raw provider HTML (Graph) — treat as UNTRUSTED
 *  and render as plain text only, never via dangerouslySetInnerHTML. */
export interface CalendarEvent {
  id: string;
  accountId: string;
  providerEventId: string;
  calendarId: string;
  title: string;
  description: string;
  location: string;
  /** UTC epoch seconds. */
  startTime: number;
  /** UTC epoch seconds (exclusive end). */
  endTime: number;
  isAllDay: boolean;
  /** Provider's original IANA timezone (e.g. "Europe/Madrid"); empty when unknown. */
  timezone: string;
  organizer: string;
  attendees: CalendarAttendee[];
  /** Extracted join URL (https only, validated backend-side). */
  meetingLink: string | null;
  /** "meet" | "teams" | "webex" | "zoom" | "gotomeeting" | "jitsi" | "other" */
  meetingPlatform: string | null;
  /** "confirmed" | "tentative" */
  status: string;
  /** Provider web link to open the event in the provider's own calendar UI. */
  htmlLink: string | null;
  /** Set when the upcoming-meeting notification fired for this event. */
  notifiedAt: number | null;
  /** Non-null when the event is an instance of a recurring series (the
   *  provider id of the series master). */
  recurringEventId: string | null;
  createdAt: number;
  updatedAt: number;
}

/** A calendar invite (.ics) attached to an email, as parsed by
 *  `get_calendar_invite`. Times are UTC epoch seconds. */
export interface CalendarInvite {
  uid: string;
  summary: string;
  location: string;
  organizer: string;
  startTime: number;
  endTime: number;
  isAllDay: boolean;
  /** Raw RRULE value, e.g. "FREQ=WEEKLY;BYDAY=TU"; null when one-off. */
  recurrence: string | null;
  /** iCalendar METHOD — "REQUEST" | "CANCEL" | … */
  method: string;
}
export type EmailCategory = 'primary' | 'social' | 'updates' | 'forums' | 'promotions';

export interface DraftAttachment {
  id: string;
  draftId: string;
  filePath: string;
  filename: string;
  mimeType: string;
}

export interface Draft {
  id: string;
  emailId: string | null;
  accountId: string;
  toAddresses: string[];
  ccAddresses: string[];
  subject: string;
  body: string;
  bodyHtml: string | null;
  aiGenerated: boolean;
  status: 'draft' | 'sent';
  /** Id of the matching draft in the provider's Drafts folder, or null when local-only. */
  providerDraftId: string | null;
  attachments: DraftAttachment[];
  createdAt: number;
  updatedAt: number;
}

export interface EmailThread {
  threadId: string;
  emails: Email[];
  subject: string;
  participants: string[];
  lastTimestamp: number;
}

export interface SyncStatus {
  accountId: string;
  status: 'idle' | 'syncing' | 'error';
  lastSyncAt: number | null;
  error: string | null;
}

// Smart filters

export interface FilterSuggestion {
  value: string;
  count: number;
}

export interface QuickFilterStats {
  topDomains: FilterSuggestion[];
  topSenders: FilterSuggestion[];
}

export type FilterType = 'domain' | 'sender' | 'priority' | 'intent' | 'topic' | 'company' | 'attachment_ext';

export type ContactKind = 'person' | 'automated';

export type ContactSort = 'last' | 'total' | 'received' | 'sent' | 'name' | 'score';

export interface Contact {
  email: string;
  name: string;
  /** Total emails (received + sent). */
  emailCount: number;
  /** Unix timestamp (seconds) of the most recent inbound or outbound email. */
  lastTimestamp?: number | null;
  /** Inbound count (emails from this contact to the user). */
  receivedCount?: number;
  /** Outbound count (emails the user sent including this contact in to/cc). */
  sentCount?: number;
  firstTimestamp?: number | null;
  /** Most-frequent company tag (TLD-stripped). */
  company?: string | null;
  /** "person" | "automated". */
  kind?: ContactKind | string;
  /** Email domain (lowercased). */
  domain?: string;
  /** Relationship strength 0-100. */
  relationshipScore?: number;
}

export interface ContactsQuery {
  search?: string;
  kind?: ContactKind | 'all';
  company?: string;
  domain?: string;
  sort?: ContactSort;
  offset?: number;
  limit?: number;
}

export interface ContactsPage {
  items: Contact[];
  total: number;
  hasMore: boolean;
}

export interface ContactDetail {
  contact: Contact;
  aliases: string[];
}

export interface CompanyContactsGroup {
  company: string | null;
  contacts: Contact[];
  totalEmails: number;
  lastTimestamp: number | null;
}

export interface SmartFilter {
  type: FilterType;
  value: string;
  count: number;
}

export interface SmartFilterPref {
  id: string;
  filterType: string;
  filterValue: string;
  status: 'pinned' | 'removed';
  accountId: string;
}

export interface SmartFilterSuggestion {
  filterType: string;
  filterValue: string;
  count: number;
}

export interface ActiveFilter {
  type: FilterType;
  value: string;
}

export interface FilteredEmailsResult {
  emails: Email[];
  totalCount: number;
}

// Attachment types

export interface AttachmentRule {
  id: string;
  accountId: string;
  name: string;
  senderEmailPattern: string | null;
  subjectPattern: string | null;
  filenamePattern: string | null;
  tags: string[];
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface Attachment {
  id: string;
  accountId: string;
  emailId: string;
  ruleId: string;
  gmailAttachmentId: string;
  filename: string;
  mimeType: string;
  fileSize: number;
  filePath: string;
  tags: string[];
  senderEmail: string;
  subject: string;
  emailTimestamp: number;
  createdAt: number;
}

export interface AiConfig {
  provider: 'ollama' | 'openrouter' | 'llamacpp';
  model: string;
  embeddingModel: string;
  monthlyBudgetUsd: number;
  periodStart: number;
  hasApiKey: boolean;
  thinkingEnabled: boolean;
}

/** A model entry in the curated llama.cpp download catalog. */
export interface CatalogModel {
  id: string;
  displayName: string;
  kind: 'chat' | 'embedding';
  sizeBytes: number;
  contextWindow: number;
  license: string;
  minRamGb: number;
  recommended: boolean;
  supportsTools: boolean;
  /** True when this model's GGUF is already on disk. */
  isLocal: boolean;
  /** True when the local file is a symlink to a file elsewhere on disk
   * (via linkLocalModel) rather than a downloaded copy. Only meaningful
   * when isLocal is true. */
  isLinked: boolean;
  /** How this entry stands on this device — see `ai::model_fit` on the Rust
   * side. `tooLarge` only occurs where the memory limit is enforced by
   * killing the process (iOS). */
  fit: 'fits' | 'tight' | 'tooLarge' | 'noDiskSpace';
  /** Whether a download should be offered. The backend refuses the download
   * on the same decision, so this is presentation, not the guard. */
  downloadable: boolean;
}

/** A GGUF file that is already downloaded locally. */
export interface LocalModel {
  id: string;
  displayName: string;
  kind: 'chat' | 'embedding';
  path: string;
  sizeBytes: number;
  isLinked: boolean;
}

/** Progress event payload emitted during a model download. */
export interface ModelDownloadProgress {
  modelId: string;
  downloadedBytes: number;
  totalBytes: number;
  /** "downloading" | "verifying" | "complete" | "error" */
  status: string;
  error?: string;
}

export interface AiUsageSummary {
  totalCostUsd: number;
  totalPromptTokens: number;
  totalCompletionTokens: number;
  totalCalls: number;
  periodStart: number;
  budgetUsd: number;
}

export interface AiModelInfo {
  id: string;
  name: string;
  pricing: {
    prompt: number;
    completion: number;
    request: number;
  };
}

export interface EmailTag {
  emailId: string;
  tagType: string;
  tagValue: string;
  confidence: number | null;
  createdAt: number;
}

/** Priority scoring for a tag (per account). `priorityScore` is computed by
 *  the backend at query time from `sentCount`, `receivedCount`, and
 *  `lastActivityAt` — so the formula can be tuned without a migration. */
export interface TagPriority {
  tagType: string;
  tagValue: string;
  sentCount: number;
  receivedCount: number;
  lastActivityAt: number | null;
  priorityScore: number;
}

export interface ClassificationConfig {
  enabled: boolean;
  classifyPrevious: boolean;
  intents: string[];
  topics: string[];
  /** Gmail inbox categories to classify. Empty = all. Default: ["primary"]. */
  categories: string[];
}

export interface ClassificationRule {
  id: string;
  accountId: string;
  name: string;
  senderPattern: string | null;
  subjectPattern: string | null;
  priority: string;
  intent: string;
  topic: string;
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface AiLogEntry {
  provider: string;
  model: string;
  operation: string;
  promptTokens: number;
  completionTokens: number;
  costUsd: number;
  status: string;
  timestamp: number;
}

// Chat-with-your-emails

export interface ChatConversation {
  id: string;
  accountId: string;
  title: string;
  createdAt: number;
  updatedAt: number;
}

export interface ChatMessageSource {
  citationNumber: number;
  emailId: string;
  relevanceScore: number | null;
  subject: string;
  sender: string;
  senderEmail: string;
  timestamp: number;
  /** The chunk of body text that was actually fed to the LLM for this citation. */
  bodyExcerpt?: string | null;
}

export type ChatRole = 'user' | 'assistant' | 'system';

export interface ChatMessage {
  id: string;
  conversationId: string;
  role: ChatRole;
  content: string;
  model: string | null;
  tokenCount: number | null;
  latencyMs: number | null;
  createdAt: number;
  sources: ChatMessageSource[];
  /** Reasoning trace attached after streaming completes (null for older rows). */
  trace?: ChatTrace | null;
  /** Structural allowlist: email IDs every tool in this turn handed to the
   *  LLM. The MarkdownContent renderer validates `email://EMAIL_ID` links
   *  against it — ids the tools never produced are dropped (and a warning
   *  logged) so hallucinated references can't open arbitrary emails.
   *  Empty on user / system messages and on pre-migration assistant rows. */
  referencedEmailIds?: string[];
  /** Same shape as `referencedEmailIds` but for draft IDs — the allowlist
   *  for `draft://DRAFT_ID` re-open-the-draft chips. */
  referencedDraftIds?: string[];
}

export interface ChatStreamEvent {
  messageId: string;
  conversationId: string;
  token: string;
  done: boolean;
  error?: string | null;
  tokenCount?: number | null;
  latencyMs?: number | null;
  /** When true, `token` REPLACES the bubble content instead of appending.
   *  Used by the backend's contradiction-guard retry, whose corrected answer
   *  must overwrite the wrong answer that already streamed live. */
  replace?: boolean | null;
}

export interface ChatSourcesEvent {
  messageId: string;
  conversationId: string;
  sources: ChatMessageSource[];
}

/** Coarse processing stage of an in-flight chat turn. Drives the LM Studio-style
 *  "Processing…" status the bubble shows before any answer tokens arrive.
 *  Mirrors the Rust `ChatPhase` enum (camelCase serde). */
export type ChatPhase =
  | 'routing'
  | 'retrieving'
  | 'searchingContacts'
  | 'searchingEmails'
  | 'retrievingEmail'
  | 'generatingDraft'
  | 'runningTools'
  | 'generating';

export interface ChatPhaseEvent {
  messageId: string;
  conversationId: string;
  phase: ChatPhase;
}

/** Trace + email-ref allowlist payload, fired on `chat-trace` once the
 *  turn finalizes. Mirrors `ChatTraceEvent` in Rust. The email refs
 *  piggyback on this event because both are computed at the same end-of-turn
 *  point — see the note in `ChatTraceEvent`'s Rust doc-comment. */

export interface SendChatResponse {
  userMessage: ChatMessage;
  assistantMessage: ChatMessage;
}

// Chat reasoning trace — mirrors the Rust `ChatTrace` and related structs in
// `src-tauri/src/models/mod.rs`. Rendered as a collapsible block in the UI so
// the user can see how an answer was produced.

export type RouteMode = 'rag_first' | 'tools_first';

export interface RouteDecision {
  mode: RouteMode;
  reason: string;
  matchedKeywords: string[];
  /** "heuristic" | "llm" | "forced" */
  classifier: string;
}

export interface RetrievalTrace {
  vectorHits: number;
  ftsHits: number;
  fusedTopK: number;
  elapsedMs: number;
  vectorFallback: boolean;
  /** Gmail categories searched for this turn. Empty = all categories. */
  categories?: string[];
  /** Candidates collapsed away by thread-dedup. */
  threadDedupCollapsed?: number;
  /** Sub-step timings (ms). Undefined when the step didn't run. */
  embeddingMs?: number | null;
  vecSearchMs?: number | null;
  ftsSearchMs?: number;
  fetchMs?: number;
  expansionMs?: number;
}

export interface ToolCallTrace {
  name: string;
  /** Tool-loop round that issued this call (0-based). -1 for preseeded
   * shortcut tools that run before the LLM loop. Lets the UI render tool and
   * LLM calls in true execution order. */
  round: number;
  arguments: unknown;
  resultPreview: string;
  resultChars: number;
  elapsedMs: number;
}

/** One LLM round-trip — either a tool-call round or the final streaming call. */
export interface LlmCallTrace {
  /** "tool_round" | "final_stream" */
  kind: string;
  /** Round index within the tool loop (0-based). -1 for the final stream. */
  round: number;
  latencyMs: number;
  /** Number of tool calls the model requested in this round (0 for final stream). */
  toolCallsRequested?: number;
  /** True if this call errored (timed out, parser failed, etc.). */
  failed?: boolean;
  /** Prompt tokens evaluated for this call (prefill size). Absent when the
   * provider doesn't report token counts. */
  promptTokens?: number | null;
  /** Wall-clock ms spent in prompt prefill before the first sampled token.
   * Only the embedded llama.cpp backend reports this. */
  prefillMs?: number | null;
  /** Prompt tokens served from a reused KV-cache prefix instead of being
   * re-evaluated. Absent when the provider doesn't report cache reuse. */
  cachedPromptTokens?: number | null;
  /** Which `PrefixPlan` the actor took for this call: `"Extend"` (within-
   *  conversation suffix-append), `"RestartFromAnchor"` (different prompt
   *  but the system anchor matched — the cross-conversation good case), or
   *  `"ColdPrefill"` (anchor stale / route flip / first call). Embedded
   *  llama.cpp only; HTTP providers and old persisted traces leave it null. */
  prefixPlan?: string | null;
  /** Token length of the system anchor (seq 2) BEFORE this call. When this
   *  is > 0 and the plan is `ColdPrefill`, the anchor was wiped mid-call —
   *  the route-flip failure mode the UI surfaces with a 🔥 badge. */
  sysCachedBefore?: number | null;
  /** Token length of the system anchor AFTER this call. */
  sysCachedAfter?: number | null;
  /** Length of the invariant system prefix the actor used (the `sys_tok`
   *  boundary from `system_prefix_bytes`). 0 when no anchor info was passed. */
  systemPrefixTokens?: number | null;
  /** Token boundary up to which seq 0 holds the stable prompt prefix.
   *  Everything past it is the volatile generation header on seq 1. */
  stableTokens?: number | null;
  /** Tokens dropped from the FRONT of the prompt to fit n_ctx. Non-zero
   *  when the prompt grew past `n_ctx − generation reserve` and had to be
   *  truncated. When this is > 0, the cold prefill is caused by running
   *  out of context, not by anchor / sys_prefix_bytes failures. */
  droppedFrontTokens?: number | null;
  /** Dev-only: full formatted prompt sent to the model for this call.
   * Populated only in debug builds (cfg(debug_assertions)). */
  input?: string | null;
  /** Dev-only: model's response (content + tool_calls, or streamed answer). */
  output?: string | null;
}

export interface ChatTrace {
  route: RouteDecision;
  retrieval?: RetrievalTrace | null;
  toolCalls: ToolCallTrace[];
  model: string;
  totalElapsedMs: number;
  /** Time spent in the full tool-call loop (all rounds combined). */
  toolLoopMs?: number;
  /** Time spent streaming the final assistant answer. Undefined when the
   * answer came directly from the tool loop. */
  llmStreamingMs?: number | null;
  /** Per-LLM-call latency breakdown — each tool round + the final stream. */
  llmCalls?: LlmCallTrace[];
}

export interface ChatRenamedEvent {
  conversationId: string;
  title: string;
}

export interface ChatTraceEvent {
  messageId: string;
  conversationId: string;
  trace: ChatTrace;
  /** Email IDs aggregated across every tool call this turn. Frontend uses
   *  it as the `email://EMAIL_ID` validation allowlist. Empty on the
   *  RagFirst route (no tools ran). */
  referencedEmailIds?: string[];
  /** Same for draft IDs (`draft://DRAFT_ID` allowlist). */
  referencedDraftIds?: string[];
}

// ── Memory subsystem ─────────────────────────────────────────────────────────

/** Pending action item persisted in the memory subsystem. */
export interface PendingTask {
  id: string;
  accountId: string;
  title: string;
  detail: string | null;
  /** "extracted" | "chat" | "user" */
  source: string;
  sourceEmailId: string | null;
  sourceThreadId: string | null;
  assignee: string;
  /** "open" | "done" | "snoozed" | "dismissed" */
  status: string;
  /** "low" | "normal" | "high" */
  priority: string;
  dueAt: number | null;
  completedAt: number | null;
  /** Company tag derived from the recipient domain stem (TLD stripped). */
  company: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface CreatePendingTaskRequest {
  accountId: string;
  title: string;
  detail?: string | null;
  priority?: string | null;
  dueAt?: number | null;
  sourceEmailId?: string | null;
  sourceThreadId?: string | null;
  source?: string | null;
  company?: string | null;
}

/** Per-thread state: who owes the next reply, deadlines, latest touchpoints. */
export interface ThreadState {
  accountId: string;
  threadId: string;
  /** "user" | "them" | "resolved" | "unknown" */
  awaiting: string;
  lastInboundAt: number | null;
  lastOutboundAt: number | null;
  lastTouchedAt: number;
  summary: string | null;
  commitment: string | null;
  deadlineAt: number | null;
  participants: string[];
  updatedAt: number;
}

export interface TaskCountsSummary {
  totalOpen: number;
  overdue: number;
  dueToday: number;
  awaitingThem: number;
}

/** A durable memory fact about the user, a contact, a domain, or a project. */
export interface MemoryFact {
  id: string;
  accountId: string;
  /** "user" | "contact" | "domain" | "project" */
  subjectKind: string;
  subjectKey: string;
  fact: string;
  /** "extraction" | "user" | "consolidation" */
  source: string;
  sourceEmailId: string | null;
  confidence: number;
  score: number;
  /** "candidate" | "promoted" | "retired" */
  status: string;
  lastUsedAt: number | null;
  /** "personal" | "professional" | null */
  domain: string | null;
  /** "atemporal" | "deciduous" | null */
  vigency: string | null;
  /** Company tag derived from the recipient domain stem (TLD stripped). */
  company: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface MemoryCountsSummary {
  total: number;
  promoted: number;
  candidate: number;
}

/** User-tunable memory (fact-extraction) config. Mirrors
 *  `services::memory::config::MemoryConfig` on the Rust side. Task-only knobs
 *  (max-tasks-per-email, task backfill window) live on `TaskConfig` now. */
export interface MemoryConfig {
  enabled: boolean;
  /** Run per-email extraction during every sync. */
  extractOnSync: boolean;
  /** Gmail categories that may feed the memory. Empty = all. */
  categories: EmailCategory[];
  /** Case-insensitive substrings of sender addresses to skip (e.g. "noreply@"). */
  excludedSenders: string[];
  /** Classification tag values (intent/topic) that cause an email to be skipped. */
  excludedTags: string[];
  /** Minutes between consolidation ticks. 0 disables the periodic ticker. */
  consolidationIntervalMinutes: number;
  /** Score threshold at which a candidate fact is promoted. */
  promoteThreshold: number;
  /** Days after which low-scoring candidates are retired. */
  candidateTtlDays: number;
  /** Days of interaction events kept before the dream job prunes them. */
  eventRetentionDays: number;
  /** Max emails processed per backfill batch. */
  backfillBatchSize: number;
  /** When true, facts are only extracted from emails the account owner sent
   *  themselves. Thread summaries are unaffected. Default true. */
  extractFromSelfOnly: boolean;
  /** Preferred language for AI-generated output (extractor + chat).
   *  Passed to the LLM as a system directive. Default "Spanish". */
  aiOutputLanguage: string;
}

/** User-tunable task-extraction config. Mirrors
 *  `services::tasks::config::TaskConfig` on the Rust side. Independent
 *  from `MemoryConfig` so the user can run task extraction with a different
 *  category/exclusion set than fact extraction. */
export interface TaskConfig {
  enabled: boolean;
  extractOnSync: boolean;
  categories: EmailCategory[];
  excludedSenders: string[];
  excludedTags: string[];
  /** Max tasks one email may produce. Default 1. */
  maxTasksPerEmail: number;
  /** Only extract tasks from emails received in the last N days. 0 = no window. */
  backfillDays: number;
  /** When true, tasks are only extracted from emails the account owner sent
   *  themselves. Default true. */
  extractFromSelfOnly: boolean;
}

export interface BackfillStatus {
  running: boolean;
  /** Count of emails still eligible for extraction. */
  remaining: number;
}

// Dashboard

export interface CategoryCount {
  category: string;
  count: number;
}

export interface AccountDashboard {
  account: Account;
  sync: SyncStatus;
  /** MIN(timestamp) of locally synced emails, or `account.syncFromTimestamp` if none. */
  syncedSince: number | null;
  syncedCount: number;
  /** Cached server-side total. `null` until the user clicks "Refresh total". */
  serverTotal: number | null;
  serverTotalFetchedAt: number | null;
  categoryCounts: CategoryCount[];
  /** Locally-stored emails in mailbox='sent' for this account. */
  sentCount: number;
  classifiedCount: number;
  /** Eligible-for-classification denominator (categories filter applied). */
  classifiedEligible: number;
  memoryAnalyzedCount: number;
  /** Eligible-for-memory denominator (categories + extract_from_self_only). */
  memoryEligible: number;
  taskAnalyzedCount: number;
  /** Same eligibility rules as memory in v1. */
  taskEligible: number;
  /** Distinct emails for this account that have at least one embedding chunk. */
  embeddedCount: number;
  /** Eligible-for-embedding denominator (embeddings_categories filter). */
  embeddedEligible: number;
  /** Messages with a junk verdict. The denominator is `syncedCount`: scoring is
   *  deterministic and runs on every message, so there is no eligibility filter. */
  junkScoredCount: number;
  /** Flagged as impersonation. Counted separately from the other two because it
   *  is a security finding, not a tidiness one. Excludes `not_junk` overrides. */
  junkPhishingCount: number;
  junkSpamCount: number;
  junkGraymailCount: number;
}

export interface RefreshServerTotalResponse {
  /** `null` for providers that don't expose a total (IMAP, Outlook in v1). */
  count: number | null;
  fetchedAt: number | null;
}

export interface TaskInfo {
  id: number;
  name: string;
  startedAt: number;
}

export interface TaskHistoryEntry {
  id: number;
  name: string;
  startedAt: number;
  finishedAt: number;
  durationSecs: number;
  /** "ok" — ran to completion. "ko" — panicked. */
  status: 'ok' | 'ko';
}

export interface QueueStateSnapshot {
  name: string;
  concurrency: number;
  running: TaskInfo[];
  pending: TaskInfo[];
  /** Most recent first, capped at 5 server-side. */
  history: TaskHistoryEntry[];
}

export interface AllQueuesState {
  ai: QueueStateSnapshot;
  aiBackground: QueueStateSnapshot;
  db: QueueStateSnapshot;
  sync: QueueStateSnapshot;
}

export interface StorageStats {
  /** Total bytes under `app_data_dir` (recursive). */
  totalBytes: number;
  /** Size of `emailops.db`. */
  dbFileBytes: number;
  /** Size of `emailops.db-wal`. 0 when checkpointed. */
  walBytes: number;
  /** Size of `emailops.db-shm`. 0 when not present. */
  shmBytes: number;
  /** Size of the `attachments/` subtree. */
  attachmentsBytes: number;
  /** Size of the `models/` subtree (embedded llama-cpp model files). */
  modelsBytes: number;
  /** Size of the `backups/` subtree (periodic SQLite backups). */
  backupsBytes: number;
  /** Everything not accounted for in the buckets above. */
  otherBytes: number;
  /** Unix seconds when this snapshot was computed. */
  computedAt: number;
}

export interface EmbeddingsConfig {
  /** Email categories whose bodies should be embedded for semantic search. */
  categories: string[];
}

// ── Lenses ───────────────────────────────────────────────────────────────────

export type LensColumnType = 'string' | 'text' | 'number' | 'currency' | 'date' | 'boolean' | 'enum' | 'email' | 'url';

export interface LensColumn {
  key: string;
  label: string;
  type: LensColumnType;
  description: string;
  required: boolean;
  enumValues?: string[];
  /** When true, rows are deduplicated by this column's value (one row per unique value). */
  isUniqueKey?: boolean;
}

export interface LensSchema {
  columns: LensColumn[];
}

export type LensDirection = 'inbound' | 'outbound' | 'either';

export interface LensTagFilter {
  tagType: string;
  tagValue: string;
}

export interface LensDateRange {
  lastDays?: number | null;
  from?: number | null;
  to?: number | null;
}

export interface LensScope {
  accountIds?: string[] | null;
  mailboxes?: string[] | null;
  categories?: string[] | null;
  tags?: LensTagFilter[] | null;
  senderDomains?: string[] | null;
  senderEmails?: string[] | null;
  query?: string | null;
  /** When false, the keyword query searches subject only. Default true (subject+sender+body). */
  querySearchBody?: boolean;
  direction?: LensDirection | null;
  dateRange?: LensDateRange | null;
}

export interface Lens {
  id: string;
  name: string;
  icon: string | null;
  templateKey: string | null;
  accountId: string | null;
  scope: LensScope;
  schema: LensSchema;
  promptText: string;
  promptVersion: number;
  modelProvider: string | null;
  modelName: string | null;
  isEnabled: boolean;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

export interface LensSummary {
  id: string;
  name: string;
  icon: string | null;
  templateKey: string | null;
  accountId: string | null;
  isEnabled: boolean;
  sortOrder: number;
  rowCount: number;
  staleCount: number;
}

export interface LensRow {
  lensId: string;
  emailId: string;
  accountId: string;
  emailSubject: string;
  emailSender: string;
  emailTimestamp: number;
  data: Record<string, unknown>;
  hasOverrides: boolean;
  promptVersion: number;
  status: string;
  errorMessage: string | null;
  extractedAt: number;
}

export interface LensRowsPage {
  rows: LensRow[];
  total: number;
}

export interface LensSortSpec {
  columnKey: string;
  direction: 'asc' | 'desc';
}

export type LensRunKind = 'backfill' | 'incremental' | 'reextract' | 'single';

export interface LensRunHandle {
  runId: string;
  lensId: string;
}

export interface LensStatus {
  lensId: string;
  /** `'idle' | 'running' | 'error'`. */
  state: string;
  currentRunId: string | null;
  currentRunKind: string | null;
  processed: number;
  total: number;
  succeeded: number;
  failed: number;
  pendingReextract: number;
  lastError: string | null;
}

export interface LensRunHistoryEntry {
  id: string;
  kind: string;
  /** `'running' | 'success' | 'failed' | 'cancelled'`. */
  status: string;
  startedAt: number;
  finishedAt: number | null;
  processed: number;
  succeeded: number;
  failed: number;
  errorMessage: string | null;
}

export interface LensTemplate {
  key: string;
  name: string;
  icon: string;
  description: string;
  defaultScope: LensScope;
  schema: LensSchema;
  prompt: string;
}

export interface CreateLensInput {
  name: string;
  icon?: string | null;
  templateKey?: string | null;
  accountId?: string | null;
  scope: LensScope;
  schema: LensSchema;
  promptText: string;
  modelProvider?: string | null;
  modelName?: string | null;
}

export interface UpdateLensInput {
  name?: string;
  icon?: string | null;
  scope?: LensScope;
  schema?: LensSchema;
  promptText?: string;
  modelProvider?: string | null;
  modelName?: string | null;
  isEnabled?: boolean;
  sortOrder?: number;
}

export interface LensPreviewRow {
  emailId: string;
  emailSubject: string;
  emailSender: string;
  data: Record<string, unknown>;
  status: string;
  errorMessage: string | null;
}

// ── Junk detection ──────────────────────────────────────────────────────────
// Mirrors `services::junk::verdict` + `db::emails::junk::StoredJunkVerdict`.
// `src/types/generated/` is unused in this repo, so these are maintained here.

/**
 * `unknown` is deliberately distinct from `clean`: it means the evidence needed
 * to decide was unavailable (no captured headers yet, or a provider that
 * withheld them). The UI must render nothing for it — never a "looks fine" badge.
 */
export type JunkBand = 'clean' | 'unknown' | 'uncertain' | 'junk';
export type JunkKind = 'legit' | 'spam' | 'phishing' | 'graymail';
export type JunkAxis = 'phishing' | 'spam' | 'graymail';
export type JunkMethod = 'deterministic' | 'statistical' | 'llm';

/** One piece of evidence. `code` is a closed enum rendered through i18n. */
export interface JunkReason {
  code: string;
  axis: JunkAxis;
  weight: number;
  detail?: string | null;
}

export interface JunkVerdict {
  emailId: string;
  spamScore: number;
  phishScore: number;
  grayScore: number;
  band: JunkBand;
  primaryKind: JunkKind;
  reasons: JunkReason[];
  method: JunkMethod;
  modelVersion: number;
  scoredAt: number;
  /** 'not_junk' is permanent and outranks every score. */
  userOverride?: string | null;
}

/** What the inbox does with a flagged message. Never deletes, never moves on the
 *  server — the strongest option still leaves the message where the server put it. */
export type JunkFlaggedAction = 'dim' | 'hide';

export interface JunkConfig {
  enabled: boolean;
  /** Off by default: this axis accuses a message of impersonation and has too
   *  little ground truth behind it to be trusted with that. */
  phishingEnabled: boolean;
  flaggedAction: JunkFlaggedAction;
}

export interface JunkModelInfo {
  axis: string;
  positives: number;
  negatives: number;
  /** Trained and in use are not the same thing: below the label floor a model
   *  exists but is not allowed to vote. */
  inUse: boolean;
  trainedAt: number | null;
}

export interface JunkStats {
  scored: number;
  unscored: number;
  phishing: number;
  spam: number;
  graymail: number;
  markedJunk: number;
  markedNotJunk: number;
  models: JunkModelInfo[];
}
