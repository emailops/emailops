import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { arch as osArch, platform as osPlatform, version as osVersion } from '@tauri-apps/plugin-os';
import type { CalendarDeleteScope, CalendarRecurrence } from '@/lib/calendarEvent';
import type {
  Account,
  AccountDashboard,
  AccountSettings,
  AiConfig,
  AiModelInfo,
  AiUsageSummary,
  AllQueuesState,
  Attachment,
  AttachmentRule,
  BackfillStatus,
  Calendar,
  CalendarEvent,
  CalendarInvite,
  CatalogModel,
  ChatConversation,
  ChatMessage,
  ClassificationConfig,
  ClassificationRule,
  CompanyContactsGroup,
  Contact,
  ContactDetail,
  ContactsPage,
  ContactsQuery,
  CreateLensInput,
  CreatePendingTaskRequest,
  Draft,
  DraftAttachment,
  Email,
  EmailAttachmentMeta,
  EmailCategory,
  EmailTag,
  EmbeddingsConfig,
  FilteredEmailsResult,
  FilterSuggestion,
  JunkConfig,
  JunkStats,
  JunkVerdict,
  Lens,
  LensPreviewRow,
  LensRowsPage,
  LensRunHandle,
  LensRunHistoryEntry,
  LensRunKind,
  LensSchema,
  LensScope,
  LensSortSpec,
  LensStatus,
  LensSummary,
  LensTemplate,
  LocalModel,
  MemoryConfig,
  MemoryCountsSummary,
  MemoryFact,
  PendingTask,
  QuickFilterStats,
  RefreshServerTotalResponse,
  SendChatResponse,
  SmartFilterPref,
  SmartFilterSuggestion,
  StorageStats,
  SyncStatus,
  TagPriority,
  TaskConfig,
  TaskCountsSummary,
  ThreadState,
  UpdateLensInput,
} from '@/types';

// Window management
export async function showMainWindow(): Promise<void> {
  return invoke('show_main_window');
}

// Account commands
export async function addAccount(provider: 'gmail' | 'outlook', syncFromTimestamp?: number | null): Promise<Account> {
  return invoke('add_account', { provider, syncFromTimestamp });
}

export interface ImapAccountConfig {
  host: string;
  port: number;
  username: string;
  password: string;
  smtpHost: string;
  smtpPort: number;
  displayName?: string;
  syncFromTimestamp?: number | null;
}

export async function testImapConnection(
  config: Omit<ImapAccountConfig, 'displayName' | 'syncFromTimestamp'>,
): Promise<void> {
  return invoke('test_imap_connection', {
    host: config.host,
    port: config.port,
    username: config.username,
    password: config.password,
    smtpHost: config.smtpHost,
    smtpPort: config.smtpPort,
  });
}

export async function addImapAccount(config: ImapAccountConfig): Promise<Account> {
  return invoke('add_imap_account', {
    host: config.host,
    port: config.port,
    username: config.username,
    password: config.password,
    smtpHost: config.smtpHost,
    smtpPort: config.smtpPort,
    displayName: config.displayName ?? null,
    syncFromTimestamp: config.syncFromTimestamp ?? null,
  });
}

export interface ImapCredentialsView {
  host: string;
  port: number;
  username: string;
  password: string;
  smtpHost: string;
  smtpPort: number;
}

/** Server settings for the re-auth dialog.
 *
 *  Deliberately carries NO password — only `hasPassword`. The stored secret is
 *  never sent to the webview (it would sit in the same renderer that displays
 *  untrusted email HTML); saving with an empty password reuses it instead.
 *
 *  The server fields come back populated from the DB mirror even when the
 *  keychain entry is missing *or* unreadable. `keychainError` is set only in the
 *  unreadable case, so the dialog can explain why rather than implying the user
 *  never saved a password. */
export interface ImapSettingsForEdit extends Omit<ImapCredentialsView, 'password'> {
  hasPassword: boolean;
  keychainError: string | null;
}

export async function getImapSettings(accountId: string): Promise<ImapSettingsForEdit> {
  return invoke('get_imap_settings', { accountId });
}

/** Cached connectivity probe result. The frontend uses this to seed its
 *  initial online state; live updates arrive via the `app-connectivity-changed`
 *  Tauri event. */
export async function isOnline(): Promise<boolean> {
  return invoke('is_online');
}

/** Save IMAP settings. An empty `password` means "keep the stored one" — the
 *  dialog never receives the current password, so it cannot echo it back. */
export async function updateImapCredentials(accountId: string, credentials: ImapCredentialsView): Promise<void> {
  return invoke('update_imap_credentials', {
    accountId,
    host: credentials.host,
    port: credentials.port,
    username: credentials.username,
    password: credentials.password,
    smtpHost: credentials.smtpHost,
    smtpPort: credentials.smtpPort,
  });
}

export async function listAccounts(): Promise<Account[]> {
  return invoke('list_accounts');
}

export async function removeAccount(accountId: string): Promise<void> {
  return invoke('remove_account', { accountId });
}

export async function reauthenticateAccount(accountId: string): Promise<void> {
  return invoke('reauthenticate_account', { accountId });
}

export async function reorderAccounts(accountIds: string[]): Promise<void> {
  return invoke('reorder_accounts', { accountIds });
}

export async function setAccountEnabled(accountId: string, enabled: boolean): Promise<void> {
  return invoke('set_account_enabled', { accountId, enabled });
}

export async function updateAccountSyncFrom(accountId: string, syncFromTimestamp?: number | null): Promise<Account> {
  return invoke('update_account_sync_from', { accountId, syncFromTimestamp });
}

export async function getAccountSettings(accountId: string): Promise<AccountSettings> {
  return invoke('get_account_settings', { accountId });
}

export async function setAccountSettings(accountId: string, settings: AccountSettings): Promise<void> {
  return invoke('set_account_settings', { accountId, settings });
}

/** Inbox category tabs to show for the given account. Provider-aware:
 *  Gmail returns the user's opt-in list, Outlook returns the fixed
 *  focused/other pair, IMAP returns []. */
export async function getAvailableCategories(accountId: string): Promise<string[]> {
  return invoke('get_available_categories', { accountId });
}

// Email commands
/** `folder:<serverPath>` addresses one custom IMAP folder. */
export type MailboxView = 'inbox' | 'sent' | 'spam' | 'deleted' | `folder:${string}`;

/** A custom IMAP folder discovered on the server, as stored by sync. */
export interface Folder {
  id: string;
  accountId: string;
  /** Raw IMAP wire name — build the MailboxView as `folder:${serverPath}`. */
  serverPath: string;
  /** Decoded UTF-8 path for display. */
  displayName: string;
  role: string;
  delimiter: string | null;
}

/** `accountId: null` selects the unified ("All accounts") view — emails
 *  merged across every enabled account. */
export async function getEmails(
  accountId: string | null,
  limit?: number,
  offset?: number,
  mailbox?: MailboxView,
): Promise<Email[]> {
  return invoke('get_emails', { accountId, limit, offset, mailbox });
}

/** Custom folders of one account, sorted by display name. Empty for
 *  Gmail/Outlook accounts (folder sync is IMAP-only). */
export async function getFolders(accountId: string): Promise<Folder[]> {
  return invoke('get_folders', { accountId });
}

/** Create a custom folder on the mail server (IMAP accounts only). `name` is
 *  a single path segment; placement and wire encoding are backend concerns. */
export async function createFolder(accountId: string, name: string): Promise<Folder> {
  return invoke('create_folder', { accountId, name });
}

/** Rename a custom folder on the mail server (IMAP accounts only). Local
 *  emails, tags and sync state migrate with it — nothing re-downloads. */
export async function renameFolder(accountId: string, folderId: string, newName: string): Promise<Folder> {
  return invoke('rename_folder', { accountId, folderId, newName });
}

/** Delete a custom folder on the mail server (IMAP accounts only). Its
 *  messages are removed on the server and locally — irreversible. */
export async function deleteFolder(accountId: string, folderId: string): Promise<void> {
  return invoke('delete_folder', { accountId, folderId });
}

/** Move an email between the inbox and a custom folder (IMAP accounts only).
 *  `targetMailbox` is `'inbox'` or `` `folder:${serverPath}` ``. */
export async function moveEmail(accountId: string, emailId: string, targetMailbox: MailboxView): Promise<void> {
  return invoke('move_email', { accountId, emailId, targetMailbox });
}

export async function getThread(accountId: string, threadId: string): Promise<Email[]> {
  return invoke('get_thread', { accountId, threadId });
}

export async function getEmailBody(accountId: string, emailId: string): Promise<string> {
  return invoke('get_email_body', { accountId, emailId });
}

export async function markAsRead(emailId: string): Promise<void> {
  return invoke('mark_as_read', { emailId });
}

export async function sendReply(
  emailId: string,
  body: string,
  fromAccountId?: string,
  toEmails?: string[],
  ccEmails?: string[],
  bodyHtml?: string,
  inlineImages?: EmailAttachment[],
  attachments?: EmailAttachment[],
): Promise<void> {
  return invoke('send_reply', {
    emailId,
    body,
    fromAccountId,
    toEmails,
    ccEmails,
    bodyHtml,
    inlineImages,
    attachments,
  });
}

export interface EmailAttachment {
  filename: string;
  mimeType: string;
  /** Base64-encoded file content */
  data: string;
  /** Required for inline images so the HTML body can reference them via cid:<contentId>. */
  contentId?: string;
  /** Marks the attachment as `Content-Disposition: inline` rather than `attachment`. */
  isInline?: boolean;
}

export async function sendNewEmail(
  accountId: string,
  toEmails: string[],
  ccEmails: string[],
  subject: string,
  body: string,
  attachments?: EmailAttachment[],
  bodyHtml?: string,
  inlineImages?: EmailAttachment[],
): Promise<void> {
  return invoke('send_new_email', {
    accountId,
    toEmails,
    ccEmails,
    subject,
    body,
    attachments,
    bodyHtml,
    inlineImages,
  });
}

export interface RecipientSuggestion {
  email: string;
  name: string;
  domainMatch: boolean;
}

export async function autocompleteRecipients(
  accountId: string,
  prefix: string,
  contextDomain?: string,
  limit?: number,
): Promise<RecipientSuggestion[]> {
  return invoke('autocomplete_recipients', { accountId, prefix, contextDomain, limit });
}

export async function deleteEmail(emailId: string): Promise<void> {
  return invoke('delete_email', { emailId });
}

/**
 * Past thread used as precedent in an AI-generated draft. Surfaced in the
 * reply panel so the user can audit what the model was grounded on.
 */
export interface DraftSource {
  emailId: string;
  threadId: string;
  subject: string;
  sender: string;
  senderEmail: string;
  timestamp: number;
  score: number;
  snippet: string;
  sentByUser: boolean;
}

export interface DraftGeneratedEvent {
  requestId: string;
  emailId: string;
  body: string;
  sources: DraftSource[];
}

export interface DraftFailedEvent {
  requestId: string;
  emailId: string;
  error: string;
}

/**
 * Kick off AI draft generation for `emailId`. Returns a `requestId` *immediately*
 * — the actual draft is delivered asynchronously via the `draft-generated`
 * (success) or `draft-failed` (error) Tauri event, both keyed by `requestId`.
 *
 * Heavy AI work runs on the backend's `ai_queue`; the frontend should listen
 * for the matching event rather than awaiting the draft inline.
 */
export async function generateDraft(emailId: string, instructions?: string | null): Promise<string> {
  return invoke('generate_draft', { emailId, instructions });
}

/**
 * Kick off AI draft generation for a brand-new email (no thread to reply to).
 * Drives the compose window's recipients + subject, plus an optional freeform
 * brief (`instructions` — typically whatever the user has jotted in the body).
 *
 * Same async contract as `generateDraft`: returns a `requestId` immediately;
 * the draft arrives via the `draft-generated` / `draft-failed` events keyed by
 * that id (with an empty `emailId`, since there is no inbound message).
 */
export async function generateNewDraft(
  accountId: string,
  to: string[],
  subject: string,
  instructions?: string | null,
): Promise<string> {
  return invoke('generate_new_draft', { accountId, to, subject, instructions });
}

// ── AI translation ───────────────────────────────────────────────────────────

export interface LanguageDetectedEvent {
  requestId: string;
  emailId: string;
  /** ISO 639-1 code, or "und" when detection failed (fail-closed: no button). */
  language: string;
  /** The user's preferred AI language code the detection was compared against. */
  preferredLanguage: string;
  needsTranslation: boolean;
}

export interface EmailTranslatedEvent {
  requestId: string;
  emailId: string;
  /** English name of the language translated into (e.g. "Spanish"). */
  targetLanguage: string;
  text: string;
  /** True when the email was longer than the model's input budget and only
   *  the beginning was translated. */
  truncated: boolean;
}

export interface ComposeTranslatedEvent {
  requestId: string;
  targetLanguage: string;
  text: string;
  truncated: boolean;
}

export interface TranslationFailedEvent {
  requestId: string;
  /** Empty for compose translations (no inbound email). */
  emailId: string;
  error: string;
}

/**
 * Lazily detect an email's language. Returns a `requestId` immediately; the
 * result arrives via the `language-detected` event. Detection failures are
 * logged backend-side only — no failure event, the Translate button simply
 * never appears.
 */
export async function detectEmailLanguage(emailId: string): Promise<string> {
  return invoke('detect_email_language', { emailId });
}

/**
 * Translate an email's body. `targetLanguage` omitted → the user's preferred
 * AI language. Returns a `requestId`; result on `email-translated`, failure
 * on `translation-failed`.
 */
export async function translateEmail(emailId: string, targetLanguage?: string | null): Promise<string> {
  return invoke('translate_email', { emailId, targetLanguage });
}

/**
 * Translate compose-draft plain text into `targetLanguage` (ISO code or a
 * typed language name, max 40 chars). Returns a `requestId`; result on
 * `compose-translated`, failure on `translation-failed` (empty `emailId`).
 */
export async function translateComposeText(text: string, targetLanguage: string): Promise<string> {
  return invoke('translate_compose_text', { text, targetLanguage });
}

export async function redownloadEmail(emailId: string): Promise<Email> {
  return invoke('redownload_email', { emailId });
}

/**
 * `accountId: null` counts across every enabled account. `mailbox` must match
 * the one passed to `getEmails` — the store compares list length against this
 * count to decide whether more pages remain.
 */
export async function getEmailCount(accountId: string | null, mailbox?: MailboxView): Promise<number> {
  return invoke('get_email_count', { accountId, mailbox });
}

// Sync commands
export async function syncAccount(accountId: string): Promise<void> {
  return invoke('start_sync_account', { accountId });
}

export async function syncAccountBlocking(accountId: string): Promise<void> {
  return invoke('sync_account', { accountId });
}

export async function getSyncStatus(accountId: string): Promise<SyncStatus> {
  return invoke('get_sync_status', { accountId });
}

// Search commands
export interface ParsedSearchQuery {
  keywords: string[];
  fromFilter: string | null;
  toFilter: string | null;
  subjectFilter: string | null;
  hasAttachment: boolean | null;
  isUnread: boolean | null;
  afterTimestamp: number | null;
  beforeTimestamp: number | null;
}

export type SearchMethod = 'rag' | 'ai_parsed' | 'pattern_parsed' | 'keyword_search';

export interface EmailWithScore extends Email {
  relevanceScore: number | null;
  matchReason: string | null;
}

export interface SearchResult {
  emails: EmailWithScore[];
  query: string;
  parsedQuery: ParsedSearchQuery | null;
  aiAvailable: boolean;
  searchMethod: SearchMethod;
}

/** `accountId: null` searches across every enabled account (unified view). */
export async function searchEmails(
  accountId: string | null,
  query?: string,
  useAi?: boolean,
  categories?: EmailCategory[],
): Promise<SearchResult> {
  return invoke('search_emails', { accountId, query, useAi, categories });
}

export async function checkAiAvailable(): Promise<boolean> {
  return invoke('check_ai_available');
}

export async function listOllamaModels(): Promise<string[]> {
  return invoke('list_ollama_models');
}

export async function getAiModel(): Promise<string> {
  return invoke('get_ai_model');
}

export async function setAiModel(model: string): Promise<void> {
  return invoke('set_ai_model', { model });
}

export async function generateEmbeddings(accountId?: string): Promise<void> {
  return invoke('start_generate_embeddings', { accountId });
}

export async function generateEmbeddingsBlocking(accountId?: string): Promise<number> {
  return invoke('generate_embeddings', { accountId });
}

export async function regenerateEmbeddings(accountId?: string): Promise<void> {
  return invoke('start_regenerate_embeddings', { accountId });
}

export async function regenerateEmbeddingsBlocking(accountId?: string): Promise<number> {
  return invoke('regenerate_embeddings', { accountId });
}

export async function rebuildFtsIndex(): Promise<number> {
  return invoke('rebuild_fts_index');
}

export async function getPendingEmbeddingsCount(accountId?: string): Promise<number> {
  return invoke('get_pending_embeddings_count', { accountId });
}

// Smart filter commands
export async function refreshFilterStats(accountId: string | null): Promise<QuickFilterStats> {
  return invoke('refresh_filter_stats', { accountId });
}

export async function getSavedSuggestions(accountId: string | null): Promise<SmartFilterSuggestion[]> {
  return invoke('get_saved_suggestions', { accountId });
}

/** `accountId: null` filters across every enabled account (unified view). */
export async function getFilteredEmails(
  accountId: string | null,
  domain?: string,
  senderEmail?: string,
  tagType?: string,
  tagValue?: string,
  limit?: number,
  offset?: number,
  attachmentExt?: string,
): Promise<FilteredEmailsResult> {
  return invoke('get_filtered_emails', {
    accountId,
    domain,
    senderEmail,
    tagType,
    tagValue,
    limit,
    offset,
    attachmentExt,
  });
}

export async function getFilterPrefs(accountId: string | null): Promise<SmartFilterPref[]> {
  return invoke('get_filter_prefs', { accountId });
}

export async function pinFilter(accountId: string | null, filterType: string, filterValue: string): Promise<void> {
  return invoke('pin_filter', { accountId, filterType, filterValue });
}

export async function removeFilter(accountId: string | null, filterType: string, filterValue: string): Promise<void> {
  return invoke('remove_filter', { accountId, filterType, filterValue });
}

export async function deleteFilterPref(
  accountId: string | null,
  filterType: string,
  filterValue: string,
): Promise<void> {
  return invoke('delete_filter_pref', { accountId, filterType, filterValue });
}

export async function getEmailInboxPosition(accountId: string | null, emailId: string): Promise<number> {
  return invoke('get_email_inbox_position', { accountId, emailId });
}

export interface SenderSuggestion {
  email: string;
  name: string;
}

export async function autocompleteSenders(
  accountId: string,
  prefix: string,
  limit?: number,
): Promise<SenderSuggestion[]> {
  return invoke('autocomplete_senders', { accountId, prefix, limit });
}

export async function getEmailById(accountId: string, emailId: string): Promise<Email> {
  return invoke('get_email_by_id', { accountId, emailId });
}

// Attachment rule commands

export async function createAttachmentRule(
  accountId: string,
  name: string,
  senderEmailPattern?: string | null,
  subjectPattern?: string | null,
  filenamePattern?: string | null,
  tags?: string[],
): Promise<AttachmentRule> {
  return invoke('create_attachment_rule', {
    accountId,
    name,
    senderEmailPattern,
    subjectPattern,
    filenamePattern,
    tags: tags ?? [],
  });
}

export async function updateAttachmentRule(
  ruleId: string,
  name: string,
  senderEmailPattern?: string | null,
  subjectPattern?: string | null,
  filenamePattern?: string | null,
  tags?: string[],
  enabled?: boolean,
): Promise<AttachmentRule> {
  return invoke('update_attachment_rule', {
    ruleId,
    name,
    senderEmailPattern,
    subjectPattern,
    filenamePattern,
    tags: tags ?? [],
    enabled: enabled ?? true,
  });
}

export async function deleteAttachmentRule(ruleId: string, accountId: string): Promise<void> {
  return invoke('delete_attachment_rule', { ruleId, accountId });
}

export async function listAttachmentRules(accountId: string): Promise<AttachmentRule[]> {
  return invoke('list_attachment_rules', { accountId });
}

export async function countAttachmentsForRule(ruleId: string): Promise<number> {
  return invoke('count_attachments_for_rule', { ruleId });
}

export async function getAttachments(
  accountId: string,
  tag?: string | null,
  limit?: number,
  offset?: number,
): Promise<Attachment[]> {
  return invoke('get_attachments', { accountId, tag, limit, offset });
}

export async function getAttachmentsForEmail(accountId: string, emailId: string): Promise<Attachment[]> {
  return invoke('get_attachments_for_email', { accountId, emailId });
}

export async function countAttachments(accountId: string, tag?: string | null): Promise<number> {
  return invoke('count_attachments', { accountId, tag });
}

export async function getAttachment(accountId: string, attachmentId: string): Promise<Attachment> {
  return invoke('get_attachment', { accountId, attachmentId });
}

export async function getAttachmentTags(accountId: string): Promise<string[]> {
  return invoke('get_attachment_tags', { accountId });
}

export async function getAttachmentFilePath(accountId: string, attachmentId: string): Promise<string> {
  return invoke('get_attachment_file_path', { accountId, attachmentId });
}

export async function getAttachmentData(accountId: string, attachmentId: string): Promise<string> {
  return invoke('get_attachment_data', { accountId, attachmentId });
}

export async function bulkDownloadAttachments(accountId: string, attachmentIds: string[]): Promise<string> {
  return invoke('bulk_download_attachments', { accountId, attachmentIds });
}

export async function applyRuleRetroactively(ruleId: string, accountId: string): Promise<number> {
  return invoke('apply_rule_retroactively', { ruleId, accountId });
}

export async function openAttachmentExternally(accountId: string, attachmentId: string): Promise<void> {
  return invoke('open_attachment_externally', { accountId, attachmentId });
}

export async function getEmailAttachmentMetas(accountId: string, emailId: string): Promise<EmailAttachmentMeta[]> {
  return invoke('get_email_attachment_metas', { accountId, emailId });
}

/** Fetch attachment bytes from the provider; returns standard base64. */
export async function fetchEmailAttachmentBytes(
  accountId: string,
  emailId: string,
  providerAttachmentId: string,
): Promise<string> {
  return invoke('fetch_email_attachment_bytes', { accountId, emailId, providerAttachmentId });
}

/** Open a locally-cached email attachment with the OS default app. */
export async function openEmailAttachmentMeta(accountId: string, metaId: string): Promise<void> {
  return invoke('open_email_attachment_meta', { accountId, metaId });
}

/** Save base64 attachment bytes into ~/Downloads; returns the saved file path. */
export async function saveAttachmentToDownloads(filename: string, dataBase64: string): Promise<string> {
  return invoke('save_attachment_to_downloads', { filename, dataBase64 });
}

/** Reveal a downloaded file (or the Downloads folder) in the OS file manager. */
export async function revealInFinder(path: string): Promise<void> {
  return invoke('reveal_in_finder', { path });
}

// AI provider commands
export async function getAiConfig(): Promise<AiConfig> {
  return invoke('get_ai_config');
}

/** App + OS facts for the feedback email's "technical info" line. */
export interface AppDiagnostics {
  appVersion: string;
  osPlatform: string;
  osVersion: string;
  arch: string;
  /** True when running Rosetta-translated on Apple Silicon. */
  translated: boolean;
}

/**
 * Gather app version (from the bundle) and OS platform/version/arch (from the
 * `os` plugin). The `os` plugin functions read values injected at startup, so
 * they are synchronous; only `getVersion()` is async.
 *
 * `arch` is the *compile-time* architecture of the running binary, so on macOS
 * `x86_64` alone cannot distinguish a real Intel Mac from a translated process
 * on Apple Silicon — a distinction that decides whether the embedded AI runtime
 * can work at all. `translated` resolves it; a failed probe reports false
 * rather than blocking the feedback form.
 */
export async function getAppDiagnostics(): Promise<AppDiagnostics> {
  return {
    appVersion: await getVersion(),
    osPlatform: osPlatform(),
    osVersion: osVersion(),
    arch: osArch(),
    translated: await invoke<boolean>('is_rosetta_translated').catch(() => false),
  };
}

export async function setAiConfig(
  provider: string,
  model: string,
  embeddingModel?: string | null,
  apiKey?: string | null,
  monthlyBudgetUsd?: number,
  thinkingEnabled?: boolean,
): Promise<void> {
  return invoke('set_ai_config', {
    provider,
    model,
    embeddingModel,
    apiKey,
    monthlyBudgetUsd: monthlyBudgetUsd ?? 0,
    thinkingEnabled: thinkingEnabled ?? false,
  });
}

export async function getAiUsage(): Promise<AiUsageSummary> {
  return invoke('get_ai_usage');
}

export async function resetAiUsage(): Promise<void> {
  return invoke('reset_ai_usage');
}

export async function listAiModels(): Promise<AiModelInfo[]> {
  return invoke('list_ai_models');
}

export async function listAiEmbeddingModels(): Promise<AiModelInfo[]> {
  return invoke('list_ai_embedding_models');
}

export async function testAiProvider(provider: string, model: string, apiKey?: string | null): Promise<string> {
  return invoke('test_ai_provider', { provider, model, apiKey });
}

export async function getEmbeddingsConfig(accountId: string): Promise<EmbeddingsConfig> {
  return invoke('get_embeddings_config', { accountId });
}

export async function setEmbeddingsConfig(accountId: string, config: EmbeddingsConfig): Promise<void> {
  return invoke('set_embeddings_config', { accountId, config });
}

// Embedded llama.cpp model catalog & download commands

export async function listCatalogModels(): Promise<CatalogModel[]> {
  return invoke('list_catalog_models');
}

export async function listLocalModels(): Promise<LocalModel[]> {
  return invoke('list_local_models');
}

export async function deleteLocalModel(modelId: string, kind: 'chat' | 'embedding'): Promise<void> {
  return invoke('delete_local_model', { modelId, kind });
}

export async function startModelDownload(modelId: string): Promise<void> {
  return invoke('start_model_download', { modelId });
}

export async function linkLocalModel(modelId: string, sourcePath: string): Promise<void> {
  return invoke('link_local_model', { modelId, sourcePath });
}

export async function cancelModelDownload(modelId: string): Promise<void> {
  return invoke('cancel_model_download', { modelId });
}

// ── Prompts (user-editable LLM prompt templates) ─────────────────────────

export interface PromptVariableInfo {
  name: string;
  description: string;
}

export interface PromptInfo {
  id: string;
  label: string;
  description: string;
  category: 'chat' | 'classification' | 'memory';
  advanced: boolean;
  defaultTemplate: string;
  currentTemplate: string;
  isOverridden: boolean;
  variables: PromptVariableInfo[];
}

export async function listPrompts(): Promise<PromptInfo[]> {
  return invoke('list_prompts');
}

export async function setPrompt(id: string, template: string): Promise<void> {
  return invoke('set_prompt', { id, template });
}

export async function resetPrompt(id: string): Promise<void> {
  return invoke('reset_prompt', { id });
}

// Classification commands
export async function getClassificationConfig(): Promise<ClassificationConfig> {
  return invoke('get_classification_config');
}

export async function setClassificationConfig(config: ClassificationConfig): Promise<void> {
  return invoke('set_classification_config', { config });
}

export async function classifyPreviousEmails(accountId: string): Promise<void> {
  return invoke('classify_previous_emails', { accountId });
}

export async function getEmailTags(emailId: string): Promise<EmailTag[]> {
  return invoke('get_email_tags', { emailId });
}

export async function getEmailTagsBatch(emailIds: string[]): Promise<EmailTag[]> {
  return invoke('get_email_tags_batch', { emailIds });
}

// ── Junk detection ──────────────────────────────────────────────────────────

/** Verdicts keyed by email id. A missing key means "not scored yet". */
export async function getJunkVerdicts(emailIds: string[]): Promise<Record<string, JunkVerdict>> {
  return invoke('get_junk_verdicts', { emailIds });
}

/**
 * Record the user's correction. `isJunk: false` is permanent — it survives
 * re-scoring, model bumps and backfills.
 */
export async function setJunkFeedback(accountId: string, emailId: string, isJunk: boolean): Promise<void> {
  return invoke('set_junk_feedback', { accountId, emailId, isJunk });
}

/** Fire-and-forget: scores previously-synced mail on the background queue. */
export async function backfillJunkScores(accountId: string): Promise<void> {
  return invoke('backfill_junk_scores', { accountId });
}

export async function countUnclassifiedEmails(accountId: string): Promise<number> {
  return invoke('count_unclassified_emails', { accountId });
}

export async function reclassifyAllEmails(accountId: string): Promise<void> {
  return invoke('reclassify_all_emails', { accountId });
}

export async function getTagPriorities(accountId: string | null, tagType: string, limit = 50): Promise<TagPriority[]> {
  return invoke('get_tag_priorities', { accountId, tagType, limit });
}

// Classification rules
export async function listClassificationRules(accountId: string): Promise<ClassificationRule[]> {
  return invoke('list_classification_rules', { accountId });
}

export async function createClassificationRule(
  accountId: string,
  name: string,
  senderPattern: string | null,
  subjectPattern: string | null,
  priority: string,
  intent: string,
  topic: string,
): Promise<ClassificationRule> {
  return invoke('create_classification_rule', {
    accountId,
    name,
    senderPattern,
    subjectPattern,
    priority,
    intent,
    topic,
  });
}

export async function updateClassificationRule(rule: ClassificationRule): Promise<void> {
  return invoke('update_classification_rule', { rule });
}

export async function deleteClassificationRule(ruleId: string, accountId: string): Promise<void> {
  return invoke('delete_classification_rule', { ruleId, accountId });
}

// Trusted senders — per-account allowlist for auto-loading remote images.
// Populated only via explicit user action on the blocked-images banner.
export async function addTrustedSender(accountId: string, senderEmail: string): Promise<void> {
  return invoke('add_trusted_sender', { accountId, senderEmail });
}

export async function removeTrustedSender(accountId: string, senderEmail: string): Promise<void> {
  return invoke('remove_trusted_sender', { accountId, senderEmail });
}

export async function listTrustedSenders(accountId: string): Promise<string[]> {
  return invoke('list_trusted_senders', { accountId });
}

export async function isSenderTrusted(accountId: string, senderEmail: string): Promise<boolean> {
  return invoke('is_sender_trusted', { accountId, senderEmail });
}

// App preferences
export async function getPref(key: string): Promise<string | null> {
  return invoke('get_pref', { key });
}

export async function setPref(key: string, value: string): Promise<void> {
  return invoke('set_pref', { key, value });
}

/**
 * The context window the embedded model uses on this machine when the
 * `chat.n_ctx` preference is unset (RAM-tiered: 8192 / 16384 / 32768).
 * Settings shows it as the auto default so saving without touching the field
 * never downgrades the machine's automatic choice.
 */
export async function getAutoNCtx(): Promise<number> {
  return invoke('get_auto_n_ctx');
}

// Returns the OS locale mapped to a supported UI language code
// ("en"/"es"/"fr"/"de"). Falls back to "en" when the OS locale is unset or
// names an unsupported language. Called by main.tsx during i18n bootstrap.
export async function getSystemLocale(): Promise<string> {
  return invoke('get_system_locale');
}

// Host capability probe — used by the onboarding wizard to recommend "Use AI"
// or "Plain email client". Keyed on RAM rather than on being a Mac, so a
// well-specced Linux or Windows machine is recommended AI too.
export interface AiCapability {
  /** True on Apple Silicon. Retained for Metal-specific copy only — branch on
   *  `localAiCapable` to decide whether local AI is viable. */
  appleSilicon: boolean;
  /** Enough RAM (and a 64-bit target) to run the smallest catalog chat model,
   *  AND this build actually contains the embedded runtime. */
  localAiCapable: boolean;
  /** Whether this binary was compiled with the embedded llama.cpp runtime.
   *  Builds without it cannot run local AI at any RAM size, so offering the
   *  option yields confusing Ollama connection errors. */
  embeddedAiAvailable: boolean;
  /** Physical RAM in whole GiB; 0 when the probe failed. */
  totalRamGb: number;
  /** RAM the smallest catalog chat model needs, so the UI can say why. */
  minRamGbForLocalAi: number;
  os: string;
  arch: string;
}

export async function detectAiCapability(): Promise<AiCapability> {
  return invoke('detect_ai_capability');
}

// Raw Tauri platform code (`macos` | `windows` | `linux` | …) for the host.
//
// Centralized here alongside the other OS-plugin reads. Falls back to an empty
// string outside a Tauri runtime (unit tests, a plain browser), which callers
// in src/lib/platform.ts treat as "not macOS" — the safe default.
export function currentPlatform(): string {
  try {
    return osPlatform();
  } catch {
    return '';
  }
}

// Build identity for the sidebar version label. `commit` is null when the
// binary was built outside a git checkout; `isRelease` is true when the built
// commit is tagged v{version} (i.e. a published release).
export interface BuildInfo {
  version: string;
  commit: string | null;
  isRelease: boolean;
}

export async function getBuildInfo(): Promise<BuildInfo> {
  return invoke('get_build_info');
}

// Latest-known newer release (mirrors `services::updates::UpdateAvailableEvent`).
// Derived from prefs the daily update check persists, so it survives restarts;
// null when the running build is up to date or no check has completed yet.
export interface AvailableUpdate {
  version: string;
  url: string;
}

export async function getAvailableUpdate(): Promise<AvailableUpdate | null> {
  return invoke('get_available_update');
}

// Security
export async function hasMainPassword(): Promise<boolean> {
  return invoke('has_main_password');
}

export async function setMainPassword(currentPassword: string | null, newPassword: string): Promise<void> {
  return invoke('set_main_password', { currentPassword, newPassword });
}

export async function verifyMainPassword(password: string): Promise<boolean> {
  return invoke('verify_main_password', { password });
}

export async function removeMainPassword(password: string): Promise<void> {
  return invoke('remove_main_password', { password });
}

// Contacts
export async function getContacts(accountId: string): Promise<Contact[]> {
  return invoke('get_contacts', { accountId });
}

export async function listContacts(accountId: string, query: ContactsQuery): Promise<ContactsPage> {
  return invoke('list_contacts', { accountId, query });
}

export async function getContactDetail(accountId: string, address: string): Promise<ContactDetail | null> {
  return invoke('get_contact_detail', { accountId, address });
}

export async function listContactsByCompany(accountId: string): Promise<CompanyContactsGroup[]> {
  return invoke('list_contacts_by_company', { accountId });
}

// Drafts
export async function listDrafts(accountId: string): Promise<Draft[]> {
  return invoke('list_drafts', { accountId });
}

export async function getDraft(draftId: string): Promise<Draft | null> {
  return invoke('get_draft', { draftId });
}

export async function listDraftAttachments(draftId: string): Promise<DraftAttachment[]> {
  return invoke('list_draft_attachments', { draftId });
}

export interface DraftAttachmentInput {
  filePath: string;
  filename?: string | null;
  mimeType?: string | null;
}

export interface SaveDraftRequest {
  id?: string;
  emailId?: string | null;
  accountId: string;
  toAddresses: string[];
  ccAddresses?: string[];
  subject: string;
  body: string;
  bodyHtml?: string | null;
  providerDraftId?: string | null;
  attachments?: DraftAttachmentInput[];
}

/**
 * Save (create or upsert) a draft. When the account's provider supports
 * server-side drafts (Gmail/Outlook), the backend also pushes it to the
 * provider's Drafts folder — best-effort, never blocking the local save.
 */
export async function saveDraft(req: SaveDraftRequest): Promise<Draft> {
  return invoke('save_draft', { req });
}

/** Send a saved draft, then remove it locally and from the provider. */
export async function sendDraft(draftId: string, accountId: string): Promise<void> {
  return invoke('send_draft', { draftId, accountId });
}

export async function deleteDraft(draftId: string, accountId: string): Promise<void> {
  return invoke('delete_draft', { draftId, accountId });
}

// Attachment extension stats
export async function getAttachmentExtStats(accountId: string): Promise<FilterSuggestion[]> {
  return invoke('get_attachment_ext_stats', { accountId });
}

// Chat-with-your-emails
export async function listChatConversations(accountId: string): Promise<ChatConversation[]> {
  return invoke('list_chat_conversations', { accountId });
}

export async function createChatConversation(accountId: string, title?: string): Promise<ChatConversation> {
  return invoke('create_chat_conversation', { accountId, title });
}

/**
 * Create a new chat session seeded with the cleaned content of an email
 * thread. The backend stores the cleaned thread as a role='system' message
 * and the chat is then constrained to that thread (no RAG retrieval, no
 * tools). Used by the "Chat about this thread" entry point in the inbox row
 * context menu.
 */
export async function createChatConversationWithThread(accountId: string, threadId: string): Promise<ChatConversation> {
  return invoke('create_chat_conversation_with_thread', { accountId, threadId });
}

export async function renameChatConversation(id: string, title: string): Promise<void> {
  return invoke('rename_chat_conversation', { id, title });
}

export async function deleteChatConversation(id: string): Promise<void> {
  return invoke('delete_chat_conversation', { id });
}

export async function getChatMessages(conversationId: string): Promise<ChatMessage[]> {
  return invoke('get_chat_messages', { conversationId });
}

export async function sendChatMessage(
  conversationId: string,
  content: string,
  categories?: EmailCategory[],
  /**
   * Thread the user has open in the main view. When set, the backend answers
   * from that thread instead of running retrieval (see `plan_turn_mode`), for
   * this turn only — it is never persisted onto the conversation.
   */
  contextThreadId?: string | null,
): Promise<SendChatResponse> {
  return invoke('send_chat_message', {
    conversationId,
    content,
    categories,
    contextThreadId: contextThreadId ?? null,
  });
}

/**
 * Fire-and-forget: seed the local model's chat prompt-prefix cache for this
 * account so the first turn skips most of its prefill. Called when the chat
 * panel opens or the selected account changes; the backend queues it and
 * returns immediately.
 */
export async function prewarmChat(accountId: string): Promise<void> {
  return invoke('prewarm_chat', { accountId });
}

// ── Memory subsystem ─────────────────────────────────────────────────────────

export async function listPendingTasks(
  accountId: string,
  opts: { status?: string; dueBefore?: number | null; limit?: number } = {},
): Promise<PendingTask[]> {
  return invoke('list_pending_tasks', {
    accountId,
    status: opts.status ?? null,
    dueBefore: opts.dueBefore ?? null,
    limit: opts.limit ?? null,
  });
}

export async function getTaskCounts(accountId: string): Promise<TaskCountsSummary> {
  return invoke('get_task_counts', { accountId });
}

export async function createPendingTask(req: CreatePendingTaskRequest): Promise<PendingTask> {
  return invoke('create_pending_task', { req });
}

export async function updatePendingTaskStatus(taskId: string, status: string): Promise<void> {
  return invoke('update_pending_task_status', { taskId, status });
}

export async function listOpenThreads(
  accountId: string,
  opts: { awaiting?: string; limit?: number } = {},
): Promise<ThreadState[]> {
  return invoke('list_open_threads', {
    accountId,
    awaiting: opts.awaiting ?? null,
    limit: opts.limit ?? null,
  });
}

export async function listMemoryFacts(
  accountId: string,
  opts: { status?: string; limit?: number } = {},
): Promise<MemoryFact[]> {
  return invoke('list_memory_facts', {
    accountId,
    status: opts.status ?? null,
    limit: opts.limit ?? null,
  });
}

export async function promoteMemoryFact(factId: string): Promise<void> {
  return invoke('promote_memory_fact', { factId });
}

export async function retireMemoryFact(factId: string): Promise<void> {
  return invoke('retire_memory_fact', { factId });
}

export async function updateMemoryFact(factId: string, fact: string): Promise<void> {
  return invoke('update_memory_fact', { factId, fact });
}

export async function deleteMemoryFact(factId: string): Promise<void> {
  return invoke('delete_memory_fact', { factId });
}

export async function getMemoryCounts(accountId: string): Promise<MemoryCountsSummary> {
  return invoke('get_memory_counts', { accountId });
}

// ── Memory configuration & backfill ──────────────────────────────────────────

export async function getMemoryConfig(): Promise<MemoryConfig> {
  return invoke('get_memory_config');
}

export async function setMemoryConfig(config: MemoryConfig): Promise<void> {
  return invoke('set_memory_config', { config });
}

export async function getTaskConfig(): Promise<TaskConfig> {
  return invoke('get_task_config');
}

export async function setTaskConfig(config: TaskConfig): Promise<void> {
  return invoke('set_task_config', { config });
}

export async function getMemoryBackfillStatus(accountId: string): Promise<BackfillStatus> {
  return invoke('get_memory_backfill_status', { accountId });
}

export async function startMemoryBackfill(accountId: string): Promise<void> {
  return invoke('start_memory_backfill', { accountId });
}

export async function cancelMemoryBackfill(): Promise<void> {
  return invoke('cancel_memory_backfill');
}

export async function resetMemoryExtraction(accountId: string): Promise<number> {
  return invoke('reset_memory_extraction', { accountId });
}

export async function getTaskBackfillStatus(accountId: string): Promise<BackfillStatus> {
  return invoke('get_task_backfill_status', { accountId });
}

export async function startTaskBackfill(accountId: string): Promise<void> {
  return invoke('start_task_backfill', { accountId });
}

export async function cancelTaskBackfill(): Promise<void> {
  return invoke('cancel_task_backfill');
}

export async function resetTaskExtraction(accountId: string): Promise<number> {
  return invoke('reset_task_extraction', { accountId });
}

export async function runMemoryConsolidation(accountId: string): Promise<void> {
  return invoke('run_memory_consolidation', { accountId });
}

// ── Calendar ─────────────────────────────────────────────────────────────────

/** Events overlapping `[rangeStart, rangeEnd)` (unix seconds) for one account.
 *  The calendar surface is per-account only — no unified variant exists. */
export async function getCalendarEvents(
  accountId: string,
  rangeStart: number,
  rangeEnd: number,
): Promise<CalendarEvent[]> {
  return invoke('get_calendar_events', { accountId, rangeStart, rangeEnd });
}

/** Every calendar the account can see (its own, shared with it, subscribed),
 *  primary first. Drives per-calendar colours and the show/hide filter. */
export async function getCalendars(accountId: string): Promise<Calendar[]> {
  return invoke('get_calendars', { accountId });
}

/** Show or hide one calendar in the calendar view. Hidden calendars keep
 *  syncing in the background, so toggling one back on is instant. */
export async function setCalendarVisible(accountId: string, calendarId: string, visible: boolean): Promise<void> {
  return invoke('set_calendar_visible', { accountId, calendarId, visible });
}

/** Create a calendar event on the provider (double-click "New event" flow).
 *  Times are unix seconds; the backend validates (non-empty title, end >
 *  start, attendee shape — max 100) and returns the stored event — for Gmail
 *  with `meetingLink` / `meetingPlatform` already populated from the
 *  auto-added Google Meet. For `recurrence !== 'none'` the returned event is
 *  the recurrence *master*; the next calendar sync expands it into
 *  per-occurrence instances. `timeZone` is the user's IANA zone
 *  (`Intl.DateTimeFormat().resolvedOptions().timeZone`). */
export async function createCalendarEvent(
  accountId: string,
  title: string,
  description: string,
  attendees: string[],
  startTime: number,
  endTime: number,
  recurrence: CalendarRecurrence,
  timeZone: string,
): Promise<CalendarEvent> {
  return invoke('create_calendar_event', {
    accountId,
    title,
    description,
    attendees,
    startTime,
    endTime,
    recurrence,
    timeZone,
  });
}

/** Delete a calendar event on the provider (event-detail dialog). When
 *  `notifyAttendees` is set, Outlook accepts an optional cancellation
 *  `message`; Google always sends its standard cancellation email. For
 *  recurring-series instances `scope` widens the delete to the following
 *  occurrences or the whole series; the default only removes the clicked
 *  occurrence. */
export async function deleteCalendarEvent(
  accountId: string,
  calendarId: string,
  providerEventId: string,
  notifyAttendees: boolean,
  message: string,
  scope: CalendarDeleteScope = 'instance',
): Promise<void> {
  return invoke('delete_calendar_event', {
    accountId,
    calendarId,
    providerEventId,
    notifyAttendees,
    message,
    scope,
  });
}

/** The calendar invite (.ics) carried by an email, or null when the email
 *  has none. */
export async function getCalendarInvite(emailId: string): Promise<CalendarInvite | null> {
  return invoke('get_calendar_invite', { emailId });
}

/** RSVP to a calendar invite on the provider (invite card in the email
 *  view). Auth-class errors are possible — classify with `isAuthError`. */
export async function rsvpCalendarInvite(
  accountId: string,
  icalUid: string,
  response: 'accepted' | 'declined' | 'tentative',
): Promise<void> {
  return invoke('rsvp_calendar_invite', { accountId, icalUid, response });
}

/** Run one calendar sync cycle for the account right now (view open / manual
 *  refresh). Hits the network — can take a few seconds. Returns the number of
 *  events stored. Errors are meaningful (e.g. auth needs re-consent). */
export async function syncCalendarNow(accountId: string): Promise<number> {
  return invoke('sync_calendar_now', { accountId });
}

// Dashboard
export async function getDashboardStats(): Promise<AccountDashboard[]> {
  return invoke('get_dashboard_stats');
}

export async function refreshServerTotal(accountId: string): Promise<RefreshServerTotalResponse> {
  return invoke('refresh_server_total', { accountId });
}

export async function getQueueState(): Promise<AllQueuesState> {
  return invoke('get_queue_state');
}

export async function getStorageStats(): Promise<StorageStats> {
  return invoke('get_storage_stats');
}

// ── Lenses ───────────────────────────────────────────────────────────────────

export async function listLenses(): Promise<LensSummary[]> {
  return invoke('list_lenses');
}

export async function getLens(lensId: string): Promise<Lens> {
  return invoke('get_lens', { lensId });
}

export async function createLens(input: CreateLensInput): Promise<Lens> {
  return invoke('create_lens', { input });
}

export async function updateLens(lensId: string, input: UpdateLensInput): Promise<Lens> {
  return invoke('update_lens', { lensId, input });
}

export async function deleteLens(lensId: string): Promise<void> {
  return invoke('delete_lens', { lensId });
}

export async function duplicateLens(lensId: string, newName: string): Promise<Lens> {
  return invoke('duplicate_lens', { lensId, newName });
}

export async function listLensTemplates(): Promise<LensTemplate[]> {
  return invoke('list_lens_templates');
}

export async function createLensFromTemplate(templateKey: string, name?: string, accountId?: string): Promise<Lens> {
  return invoke('create_lens_from_template', {
    templateKey,
    name: name ?? null,
    accountId: accountId ?? null,
  });
}

export async function getLensRows(
  lensId: string,
  opts: { sort?: LensSortSpec; limit?: number; offset?: number } = {},
): Promise<LensRowsPage> {
  // Backend SortSpec is `{ key, desc }` — translate from the UI shape.
  const sortPayload = opts.sort ? { key: opts.sort.columnKey, desc: opts.sort.direction === 'desc' } : null;
  return invoke('get_lens_rows', {
    lensId,
    sort: sortPayload,
    limit: opts.limit ?? null,
    offset: opts.offset ?? null,
  });
}

export async function updateLensRowOverride(
  lensId: string,
  emailId: string,
  overrides: Record<string, unknown>,
): Promise<void> {
  return invoke('update_lens_row_override', { lensId, emailId, overrides });
}

export async function excludeLensRow(lensId: string, emailId: string): Promise<void> {
  return invoke('exclude_lens_row', { lensId, emailId });
}

export async function includeLensRow(lensId: string, emailId: string): Promise<void> {
  return invoke('include_lens_row', { lensId, emailId });
}

export async function runLens(lensId: string, kind?: LensRunKind): Promise<LensRunHandle> {
  return invoke('run_lens', { lensId, kind: kind ?? null });
}

export async function cancelLensRun(lensId: string): Promise<boolean> {
  return invoke('cancel_lens_run', { lensId });
}

export async function getLensStatus(lensId: string): Promise<LensStatus> {
  return invoke('get_lens_status', { lensId });
}

export async function listLensRuns(lensId: string, limit?: number): Promise<LensRunHistoryEntry[]> {
  return invoke('list_lens_runs', { lensId, limit: limit ?? null });
}

export async function reextractLensRow(lensId: string, emailId: string): Promise<void> {
  return invoke('reextract_lens_row', { lensId, emailId });
}

export async function previewLensExtraction(
  scope: LensScope,
  schema: LensSchema,
  prompt: string,
  sampleSize?: number,
): Promise<LensPreviewRow[]> {
  return invoke('preview_lens_extraction', {
    scope,
    schema,
    prompt,
    sampleSize: sampleSize ?? null,
  });
}

/**
 * Confirm a message is junk and file it in the server's Junk folder where the
 * provider supports moves (IMAP today). Resolves to `false` when the account has
 * no server-side Junk folder — the local override is recorded either way.
 */
export async function reportJunkToProvider(accountId: string, emailId: string): Promise<boolean> {
  return invoke('report_junk_to_provider', { accountId, emailId });
}

export async function getJunkConfig(): Promise<JunkConfig> {
  return invoke('get_junk_config');
}

export async function setJunkConfig(config: JunkConfig): Promise<void> {
  return invoke('set_junk_config', { config });
}

export async function getJunkStats(accountId: string): Promise<JunkStats> {
  return invoke('get_junk_stats', { accountId });
}
