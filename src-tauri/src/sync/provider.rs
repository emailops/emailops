use std::collections::HashMap;

use async_trait::async_trait;

use crate::models::error::{AppError, Result};
use crate::models::{Email, ProviderDraft};
use crate::services::i18n::Language;
use crate::sync::draft_plan::{plan_draft_fetches, ListedDraft, ProviderDraftPull};

/// Whether a provider (identified by its `accounts.provider` string) supports
/// server-side drafts we can push to / pull from. Gmail and Outlook expose
/// draft APIs; IMAP does not in our implementation, so its drafts stay local.
pub fn provider_supports_drafts(provider: &str) -> bool {
    matches!(provider, "gmail" | "outlook")
}

/// Whether a provider supports server-side mailbox-state writes — pushing
/// read/unread and delete back to the account so the change is visible in the
/// provider's own clients. Gmail implements them via `messages.modify` /
/// `messages.trash`; IMAP (flags + Trash move) and Outlook (Graph `isRead` +
/// move) are not wired yet, so their mailbox state stays local to EmailOps.
pub fn provider_supports_mailbox_writes(provider: &str) -> bool {
    matches!(provider, "gmail")
}

/// An attachment to include in an outgoing email.
///
/// `content_id` + `is_inline = true` mark this as an inline image referenced
/// from the HTML body via a `cid:<content_id>` URI (RFC 2392). Regular file
/// attachments leave both at their default (None / false) so the SMTP layer
/// renders them as `Content-Disposition: attachment`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailAttachment {
    /// Original filename (e.g. "report.pdf")
    pub filename: String,
    /// MIME type (e.g. "application/pdf")
    pub mime_type: String,
    /// Base64-encoded file content (standard or URL-safe, with or without padding)
    pub data: String,
    /// Content-ID used for inline references from the HTML body (`<img src="cid:…">`).
    /// `None` for regular file attachments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
    /// When true, render with `Content-Disposition: inline` and (for providers
    /// that need it) nest inside a `multipart/related` part next to the HTML body.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_inline: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Body of an outgoing email: a plain-text part, an optional HTML alternative,
/// and the inline images referenced from the HTML via `cid:` URIs.
///
/// Plain-text is always required as a fallback for clients that don't render
/// HTML (or have it disabled) — every modern MUA still expects `multipart/alternative`.
#[derive(Debug, Clone)]
pub struct EmailBody {
    /// Plain-text fallback (always present).
    pub text: String,
    /// Optional HTML alternative. When set, providers send `multipart/alternative`.
    pub html: Option<String>,
    /// Inline images referenced from `html` via `cid:<content_id>`. Each must
    /// have `is_inline = true` and a non-empty `content_id`. Ignored when
    /// `html` is None.
    pub inline_images: Vec<EmailAttachment>,
    /// Language for the "Sent with EmailOps" footer appended at the MIME/payload
    /// layer. Resolved from the user's UI-language preference in the send
    /// service; defaults to English so direct constructions stay deterministic.
    pub language: Language,
    /// Whether the "Sent with EmailOps" footer is appended when this body is
    /// serialized. The footer belongs to the *send* action, so drafts pushed to
    /// the provider set this `false` — otherwise a push→pull→send round-trip
    /// would bake the footer in twice. Defaults to `true`.
    pub append_footer: bool,
}

impl EmailBody {
    /// Plain-text-only body — most existing call sites use this.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            html: None,
            inline_images: Vec::new(),
            language: Language::default(),
            append_footer: true,
        }
    }

    /// Text + HTML alternative, no inline images.
    pub fn with_html(text: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            html: Some(html.into()),
            inline_images: Vec::new(),
            language: Language::default(),
            append_footer: true,
        }
    }

    /// Set the footer language (builder-style). Used by the send service after
    /// resolving the user's UI-language preference.
    pub fn with_language(mut self, language: Language) -> Self {
        self.language = language;
        self
    }

    /// Suppress the "Sent with EmailOps" footer (builder-style). Used when
    /// pushing a draft to the provider — the footer is added at send time.
    pub fn without_footer(mut self) -> Self {
        self.append_footer = false;
        self
    }

    /// The plain-text footer to append when serializing this body — empty when
    /// `append_footer` is disabled.
    pub fn footer_plain(&self) -> String {
        if self.append_footer {
            email_footer_plain(self.language)
        } else {
            String::new()
        }
    }

    /// The HTML footer to append when serializing this body — empty when
    /// `append_footer` is disabled.
    pub fn footer_html(&self) -> String {
        if self.append_footer {
            email_footer_html(self.language)
        } else {
            String::new()
        }
    }

    /// True when this body has an HTML alternative the provider should serialize.
    pub fn has_html(&self) -> bool {
        self.html.is_some()
    }
}

/// Reference to a message in the provider's system (ID + thread ID).
#[derive(Debug, Clone)]
pub struct MessageRef {
    pub id: String,
    pub thread_id: String,
}

/// Metadata about a file attachment in an email.
#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    /// Provider-specific attachment ID for fetching bytes.
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    /// Base64-encoded inline data (for small attachments embedded in the message).
    pub inline_data: Option<String>,
}

/// Auxiliary mailbox views synced in addition to the primary inbox.
/// Drafts live in the separate `drafts` table so they are not included here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtraMailbox {
    Sent,
    Spam,
    Trash,
}

impl ExtraMailbox {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Spam => "spam",
            Self::Trash => "trash",
        }
    }

    /// The mailboxes that need an extra sync pass beyond the primary inbox.
    ///
    /// `Sent` is included even though the main inbox sync already pulls sent
    /// messages (Gmail via `in:sent`, Outlook via `/me/messages`, IMAP via
    /// merged Sent folder). The reason is capacity: the inbox pass is capped
    /// at `MAX_INCREMENTAL_EMAILS_PER_SYNC` per run, and on heavy mailboxes
    /// inbox traffic can crowd out sent emails until they fall out of the
    /// incremental window. A dedicated Sent pass with its own watermark
    /// guarantees the user's outgoing mail is captured independently.
    /// Duplicates are deduped cheaply via `emails_exist_batch`.
    pub fn all() -> &'static [ExtraMailbox] {
        &[Self::Sent, Self::Spam, Self::Trash]
    }
}

/// Email category parsed from provider labels/folders.
#[derive(Debug, Clone, PartialEq)]
pub enum EmailCategory {
    Primary,
    Social,
    Promotions,
    Updates,
    Forums,
}

impl EmailCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Social => "social",
            Self::Promotions => "promotions",
            Self::Updates => "updates",
            Self::Forums => "forums",
        }
    }

    pub fn is_promotions(&self) -> bool {
        matches!(self, Self::Promotions)
    }
}

/// Localized "Sent with" lead-in for the footer. The product name "EmailOps"
/// is a brand and is never translated.
fn footer_prefix(language: Language) -> &'static str {
    match language {
        Language::En => "Sent with",
        Language::Es => "Enviado con",
        Language::Fr => "Envoyé avec",
        Language::De => "Gesendet mit",
    }
}

/// Plain-text footer appended to every outgoing email, in the user's UI language.
///
/// The URL is on its own line — wrapping it in parentheses (e.g. `(https://…)`)
/// makes most email-client auto-linkers swallow the trailing `)` into the URL,
/// producing a broken link like `https://getemailops.com)/`.
pub fn email_footer_plain(language: Language) -> String {
    format!("\n\n--\n{} EmailOps\nhttps://getemailops.com", footer_prefix(language))
}

/// HTML footer appended to every outgoing email, in the user's UI language.
pub fn email_footer_html(language: Language) -> String {
    format!(
        "<br><br><hr style=\"border:none;border-top:1px solid #eee;margin:16px 0\">\
         <p style=\"color:#888;font-size:12px;margin:0\">{} \
         <a href=\"https://getemailops.com\" style=\"color:#888\">EmailOps</a></p>",
        footer_prefix(language)
    )
}

/// Metadata a provider can report about a just-sent message, used to insert
/// an optimistic local Sent copy without waiting for the next sync. All
/// fields are best-effort: Gmail fills all three, IMAP only the RFC
/// Message-ID, Outlook none (Graph's send endpoints return 202 with no body).
#[derive(Debug, Clone, Default)]
pub struct SentMessageMeta {
    /// Provider-canonical message id (Gmail `id`). When present, the
    /// optimistic row uses it as its primary key and needs no reconciliation
    /// — the sync layer's existing-id dedup keeps it as the permanent row.
    pub provider_message_id: Option<String>,
    /// Provider thread id for the sent copy (Gmail `threadId`).
    pub provider_thread_id: Option<String>,
    /// RFC 5322 `Message-ID` header of the outgoing MIME (lettre-generated).
    /// Lets the reconciler exact-match the provider's Sent copy when it is
    /// ingested later (IMAP).
    pub message_id_header: Option<String>,
}

/// Target of a message move: back to the inbox, or into a custom folder
/// addressed by its exact server path (wire format).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveTarget {
    Inbox,
    Folder(String),
}

impl MoveTarget {
    /// The `emails.mailbox` column value for messages living in this target.
    pub fn mailbox_value(&self) -> String {
        match self {
            Self::Inbox => "inbox".to_string(),
            Self::Folder(path) => format!("folder:{path}"),
        }
    }
}

/// Abstraction over email providers (Gmail, IMAP, Outlook, etc.).
///
/// Services depend on this trait, never on concrete providers.
#[async_trait]
pub trait EmailProvider: Send + Sync {
    /// Get the authenticated user's email and display name.
    async fn get_profile(&self) -> Result<(String, String)>;

    /// List message references, optionally filtered by date range.
    /// Returns (messages, next_page_token).
    /// `label_filter` is an optional Gmail query fragment (e.g. `"(category:primary OR in:sent)"`).
    /// IMAP implementations should ignore it.
    async fn list_messages(
        &self,
        max_results: u32,
        page_token: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        label_filter: Option<&str>,
    ) -> Result<(Vec<MessageRef>, Option<String>)>;

    /// Fetch a full message by ID.
    /// Returns the email, its category, and attachment metadata.
    async fn get_message(&self, message_id: &str) -> Result<(Email, EmailCategory, Vec<AttachmentInfo>)>;

    /// Send a reply to an existing message.
    ///
    /// `body` carries the plain-text part and, when the user composed in the
    /// rich editor, an HTML alternative plus any inline images referenced from
    /// the HTML via `cid:` URIs. `attachments` is for regular file attachments
    /// only.
    ///
    /// Returns best-effort [`SentMessageMeta`] about the sent copy so the
    /// caller can store an optimistic local Sent row.
    async fn send_reply(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        thread_id: &str,
        original_message_id: Option<&str>,
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<SentMessageMeta>;

    /// Send a new email (not a reply to any existing thread).
    ///
    /// `body` carries the plain-text part and, when the user composed in the
    /// rich editor, an HTML alternative. Inline images live inside `body`;
    /// `attachments` is for regular file attachments only.
    ///
    /// Returns best-effort [`SentMessageMeta`] about the sent copy so the
    /// caller can store an optimistic local Sent row.
    async fn send_new_email(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<SentMessageMeta>;

    /// Fetch raw attachment bytes by message ID and attachment ID.
    async fn fetch_attachment_bytes(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>>;

    /// List message IDs in a non-inbox mailbox (Sent/Spam/Trash). Newest-first,
    /// capped by `max_results`. Pass `after_timestamp = None` for an initial
    /// pull; on subsequent syncs pass the timestamp of the newest message we
    /// already have for that mailbox to fetch only new items.
    ///
    /// `before_timestamp` constrains the search to messages strictly older
    /// than the given epoch. Backfill loops use this to walk the mailbox
    /// history in date-descending windows: pass the oldest already-stored
    /// timestamp, ingest the returned batch, then pass the new minimum on the
    /// next iteration until an empty page comes back.
    ///
    /// Default impl returns empty, so a provider that hasn't been wired up yet
    /// simply skips these mailboxes instead of erroring.
    async fn list_mailbox_messages(
        &self,
        _mailbox: ExtraMailbox,
        _max_results: u32,
        _after_timestamp: Option<i64>,
        _before_timestamp: Option<i64>,
    ) -> Result<Vec<MessageRef>> {
        Ok(Vec::new())
    }

    /// Enumerate the folders the provider exposes (IMAP `LIST`), including
    /// name attributes so the caller can run the role/custom detection ladder.
    /// Default impl returns empty — providers without folder enumeration
    /// (Gmail, Outlook) simply skip custom-folder sync.
    async fn list_folders(&self) -> Result<Vec<crate::sync::folder_plan::ListedFolder>> {
        Ok(Vec::new())
    }

    /// List message IDs in a custom folder addressed by its exact server path.
    /// Same newest-first / watermark semantics as [`Self::list_mailbox_messages`].
    /// Default impl returns empty.
    async fn list_folder_messages(
        &self,
        _server_path: &str,
        _max_results: u32,
        _after_timestamp: Option<i64>,
        _before_timestamp: Option<i64>,
    ) -> Result<Vec<MessageRef>> {
        Ok(Vec::new())
    }

    /// Fetch multiple messages in bulk.
    ///
    /// Returns `Err` only on a transport or auth failure that affects the whole batch.
    /// Individual sub-request failures are `Err` entries in the inner `Vec`, so the
    /// caller can log and skip them without aborting the entire sync.
    ///
    /// Default implementation: sequential `get_message` calls.
    /// Gmail overrides this with the Batch HTTP API (up to 100 requests per HTTP call).
    async fn batch_get_messages(
        &self,
        message_ids: &[&str],
    ) -> Result<Vec<Result<(Email, EmailCategory, Vec<AttachmentInfo>)>>> {
        let mut results = Vec::with_capacity(message_ids.len());
        for id in message_ids {
            results.push(self.get_message(id).await);
        }
        Ok(results)
    }

    // ── Folder management ─────────────────────────────────────────────────
    //
    // IMAP-only in v1: only the IMAP adapter overrides these; callers gate
    // the UI behind the account's provider, so the "unsupported" defaults
    // below only fire if a provider is mis-wired.

    /// Create a folder at the given exact server path (wire format).
    async fn create_folder(&self, _server_path: &str) -> Result<()> {
        Err(AppError::InvalidInput(
            "folder management is not supported by this provider".to_string(),
        ))
    }

    /// Rename a folder from one exact server path to another.
    async fn rename_folder(&self, _old_server_path: &str, _new_server_path: &str) -> Result<()> {
        Err(AppError::InvalidInput(
            "folder management is not supported by this provider".to_string(),
        ))
    }

    /// Delete a folder (and, per IMAP semantics, the messages inside it).
    async fn delete_folder(&self, _server_path: &str) -> Result<()> {
        Err(AppError::InvalidInput(
            "folder management is not supported by this provider".to_string(),
        ))
    }

    /// Move a message into `target`. `message_id_header` is the message's RFC
    /// 5322 Message-ID when known — implementations use it to resolve the
    /// message's new provider id in the target folder. Returns that new
    /// [`MessageRef`] so the caller can re-ingest the moved message without a
    /// full folder resync, or `None` when the new id cannot be determined.
    async fn move_message(
        &self,
        _message_id: &str,
        _message_id_header: Option<&str>,
        _target: &MoveTarget,
    ) -> Result<Option<MessageRef>> {
        Err(AppError::InvalidInput(
            "moving messages is not supported by this provider".to_string(),
        ))
    }

    // ── Mailbox state ─────────────────────────────────────────────────────
    //
    // Read/unread and delete, pushed back to the account so the change shows
    // up in the provider's own clients. Callers gate on
    // `provider_supports_mailbox_writes` and keep the change local for
    // providers that don't implement these, so the defaults below only fire
    // if a provider is mis-wired.

    /// Set one message's read/unread state at the provider.
    async fn set_read_state(&self, _message_id: &str, _read: bool) -> Result<()> {
        Err(AppError::InvalidInput(
            "mailbox state writes are not supported by this provider".to_string(),
        ))
    }

    /// Move one message to the provider's Trash. Recoverable by the user from
    /// the provider's own UI — this is not a permanent delete.
    async fn trash_message(&self, _message_id: &str) -> Result<()> {
        Err(AppError::InvalidInput(
            "mailbox state writes are not supported by this provider".to_string(),
        ))
    }

    // ── Drafts ────────────────────────────────────────────────────────────
    //
    // Providers that support server-side drafts (Gmail, Outlook) override
    // these; callers gate the create/update/delete calls behind
    // `provider_supports_drafts(account.provider)`, so the "unsupported"
    // defaults below only fire if a provider is mis-wired. `list_drafts`
    // defaults to empty (the pull pass is best-effort) rather than erroring.

    /// Create a draft in the provider's Drafts folder. Returns the provider's
    /// draft id, which the caller stores locally to keep the two in sync.
    async fn create_draft(
        &self,
        _from_email: &str,
        _to_emails: &[String],
        _cc_emails: &[String],
        _subject: &str,
        _body: &EmailBody,
        _attachments: &[EmailAttachment],
    ) -> Result<String> {
        Err(AppError::InvalidInput(
            "drafts are not supported by this provider".to_string(),
        ))
    }

    /// Update an existing provider draft in place. Returns the (possibly new)
    /// provider draft id.
    async fn update_draft(
        &self,
        _provider_draft_id: &str,
        _from_email: &str,
        _to_emails: &[String],
        _cc_emails: &[String],
        _subject: &str,
        _body: &EmailBody,
        _attachments: &[EmailAttachment],
    ) -> Result<String> {
        Err(AppError::InvalidInput(
            "drafts are not supported by this provider".to_string(),
        ))
    }

    /// Delete a draft from the provider's Drafts folder.
    async fn delete_draft(&self, _provider_draft_id: &str) -> Result<()> {
        Err(AppError::InvalidInput(
            "drafts are not supported by this provider".to_string(),
        ))
    }

    /// List drafts currently in the provider's Drafts folder, for the pull pass.
    ///
    /// `known_change_tokens` maps provider draft id → the change token stored
    /// when that draft's content was last read. Providers whose listing is
    /// cheap but whose per-draft read is not (Gmail: 1 `drafts.list` + N
    /// `drafts.get`) must use it to skip unchanged drafts — otherwise every
    /// sync tick re-downloads the whole Drafts folder. Providers that return
    /// full content in the listing itself can ignore it.
    ///
    /// The returned `present_ids` must enumerate *all* drafts upstream, not
    /// just the changed ones: it is the keep-list for `prune_provider_drafts`,
    /// so a partial list silently deletes local drafts.
    async fn list_drafts(&self, _known_change_tokens: &HashMap<String, String>) -> Result<ProviderDraftPull> {
        Ok(ProviderDraftPull::default())
    }
}

// ── Fake provider for tests ──────────────────────────────────────────────────
//
// Lives in the production crate (not `#[cfg(test)]`) so integration tests and
// eval harnesses can use it without enabling cargo test mode. Tests that touch
// sync paths should depend on `FakeEmailProvider` instead of stubbing reqwest /
// the Gmail HTTP API.

/// In-memory `EmailProvider` for tests. Pre-load messages with [`add_message`],
/// inspect sent mail via [`sent`]. All other surface area returns sensible
/// defaults (empty mailboxes, missing attachment bytes, etc.).
pub struct FakeEmailProvider {
    profile_email: String,
    profile_name: String,
    /// Messages available to `list_messages` / `get_message`, keyed by id.
    messages: std::sync::RwLock<Vec<FakeStoredMessage>>,
    /// Outbound mail recorded by `send_reply` / `send_new_email`.
    sent: std::sync::RwLock<Vec<FakeSentMessage>>,
    /// Bytes returned by `fetch_attachment_bytes`, keyed by `(message_id, attachment_id)`.
    attachment_bytes: std::sync::RwLock<std::collections::HashMap<(String, String), Vec<u8>>>,
    /// Server-side drafts, keyed by provider draft id. Populated by
    /// `create_draft`/`update_draft`, read by `list_drafts`, and used by tests
    /// to assert push/pull behaviour.
    drafts: std::sync::RwLock<std::collections::HashMap<String, ProviderDraft>>,
    /// Monotonic counter for deterministic fake draft ids (no `Math.random`).
    draft_seq: std::sync::atomic::AtomicU64,
    /// Metadata returned by `send_reply` / `send_new_email`. Defaults to an
    /// empty meta (Outlook-shaped); tests set it to simulate Gmail (provider
    /// ids) or IMAP (Message-ID header) providers.
    send_meta: std::sync::RwLock<SentMessageMeta>,
    /// Folders reported by `list_folders`, simulating an IMAP `LIST` response.
    folders: std::sync::RwLock<Vec<crate::sync::folder_plan::ListedFolder>>,
    /// Folder-management operations performed, for test assertions.
    folder_ops: std::sync::RwLock<Vec<FakeFolderOp>>,
    /// When `Some`, `move_message` returns this instead of the default
    /// same-id ref — simulates providers that re-key moved messages
    /// (`Some(Some(ref))`) or cannot report the new id (`Some(None)`).
    move_result: std::sync::RwLock<Option<Option<MessageRef>>>,
    /// Mailbox-state writes performed, for test assertions.
    mailbox_ops: std::sync::RwLock<Vec<FakeMailboxOp>>,
    /// When `Some`, every mailbox-state write fails with this message instead
    /// of being recorded — simulates an offline or refusing provider.
    mailbox_write_failure: std::sync::RwLock<Option<String>>,
}

/// A mailbox-state call recorded by [`FakeEmailProvider`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeMailboxOp {
    SetReadState { message_id: String, read: bool },
    Trash { message_id: String },
}

/// A folder-management call recorded by [`FakeEmailProvider`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeFolderOp {
    Create(String),
    Rename(String, String),
    Delete(String),
    Move { message_id: String, mailbox_value: String },
}

#[derive(Debug, Clone)]
struct FakeStoredMessage {
    email: Email,
    category: EmailCategory,
    attachments: Vec<AttachmentInfo>,
}

/// Record of a message produced by `send_reply` / `send_new_email` on a
/// `FakeEmailProvider`. `original_message_id` is `Some` for replies, `None`
/// for new mail.
#[derive(Debug, Clone)]
pub struct FakeSentMessage {
    pub from_email: String,
    pub to_emails: Vec<String>,
    pub cc_emails: Vec<String>,
    pub thread_id: Option<String>,
    pub original_message_id: Option<String>,
    pub subject: String,
    pub body: EmailBody,
    pub attachments: Vec<EmailAttachment>,
}

impl FakeEmailProvider {
    pub fn new(profile_email: impl Into<String>, profile_name: impl Into<String>) -> Self {
        Self {
            profile_email: profile_email.into(),
            profile_name: profile_name.into(),
            messages: std::sync::RwLock::new(Vec::new()),
            sent: std::sync::RwLock::new(Vec::new()),
            attachment_bytes: std::sync::RwLock::new(std::collections::HashMap::new()),
            drafts: std::sync::RwLock::new(std::collections::HashMap::new()),
            draft_seq: std::sync::atomic::AtomicU64::new(0),
            send_meta: std::sync::RwLock::new(SentMessageMeta::default()),
            folders: std::sync::RwLock::new(Vec::new()),
            folder_ops: std::sync::RwLock::new(Vec::new()),
            move_result: std::sync::RwLock::new(None),
            mailbox_ops: std::sync::RwLock::new(Vec::new()),
            mailbox_write_failure: std::sync::RwLock::new(None),
        }
    }

    /// Snapshot of the mailbox-state writes performed so far.
    pub fn mailbox_ops(&self) -> Vec<FakeMailboxOp> {
        self.mailbox_ops.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Make every subsequent mailbox-state write fail with `message`.
    pub fn fail_mailbox_writes(&self, message: impl Into<String>) {
        *self
            .mailbox_write_failure
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(message.into());
    }

    /// `Err` when a failure has been configured, `Ok` otherwise.
    fn mailbox_write_gate(&self) -> Result<()> {
        match self
            .mailbox_write_failure
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            Some(message) => Err(AppError::SyncError(message)),
            None => Ok(()),
        }
    }

    /// Override what subsequent `move_message` calls return (see the field
    /// doc). The default (no override) returns the moved message's own id.
    pub fn set_move_result(&self, result: Option<MessageRef>) {
        *self.move_result.write().unwrap_or_else(PoisonError::into_inner) = Some(result);
    }

    /// Snapshot of the folder-management operations performed so far.
    pub fn folder_ops(&self) -> Vec<FakeFolderOp> {
        self.folder_ops.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Configure the folder set reported by `list_folders`, simulating the
    /// server's IMAP `LIST` response.
    pub fn set_folders(&self, folders: Vec<crate::sync::folder_plan::ListedFolder>) {
        *self.folders.write().unwrap_or_else(PoisonError::into_inner) = folders;
    }

    /// Configure the [`SentMessageMeta`] returned by subsequent send calls,
    /// simulating a Gmail-shaped (provider ids) or IMAP-shaped (Message-ID
    /// header) provider.
    pub fn set_send_meta(&self, meta: SentMessageMeta) {
        *self.send_meta.write().unwrap_or_else(PoisonError::into_inner) = meta;
    }

    /// Seed a draft as if it already exists in the provider's Drafts folder.
    /// Used by pull tests that need the provider to have drafts the app hasn't
    /// created itself.
    pub fn add_provider_draft(&self, draft: ProviderDraft) {
        self.drafts
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(draft.provider_draft_id.clone(), draft);
    }

    /// Snapshot of the provider-side drafts, for test assertions.
    pub fn provider_drafts(&self) -> Vec<ProviderDraft> {
        self.drafts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Add a message to the fake mailbox. Ordering reflects insertion order;
    /// `list_messages` re-sorts newest-first by timestamp.
    pub fn add_message(&self, email: Email, category: EmailCategory, attachments: Vec<AttachmentInfo>) {
        self.messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeStoredMessage {
                email,
                category,
                attachments,
            });
    }

    /// Snapshot of every message sent through this provider. Returned by value
    /// so tests can inspect without holding the lock.
    pub fn sent(&self) -> Vec<FakeSentMessage> {
        self.sent.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Configure the bytes returned by `fetch_attachment_bytes` for a given
    /// message + attachment id.
    pub fn set_attachment_bytes(
        &self,
        message_id: impl Into<String>,
        attachment_id: impl Into<String>,
        bytes: Vec<u8>,
    ) {
        self.attachment_bytes
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((message_id.into(), attachment_id.into()), bytes);
    }
}

use std::sync::PoisonError;

#[async_trait]
impl EmailProvider for FakeEmailProvider {
    async fn get_profile(&self) -> Result<(String, String)> {
        Ok((self.profile_email.clone(), self.profile_name.clone()))
    }

    async fn list_messages(
        &self,
        max_results: u32,
        _page_token: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        _label_filter: Option<&str>,
    ) -> Result<(Vec<MessageRef>, Option<String>)> {
        let guard = self.messages.read().unwrap_or_else(PoisonError::into_inner);
        let mut filtered: Vec<&FakeStoredMessage> = guard
            .iter()
            // The main inbox pass intentionally only sees inbox-shaped messages
            // by default. Extra-mailbox content (sent / spam / trash) is routed
            // via `list_mailbox_messages` so tests can exercise the dedicated
            // pass without the inbox pass swallowing the messages first.
            .filter(|m| m.email.mailbox == "inbox")
            .filter(|m| after_timestamp.is_none_or(|t| m.email.timestamp >= t))
            .filter(|m| before_timestamp.is_none_or(|t| m.email.timestamp < t))
            .collect();
        filtered.sort_by_key(|m| std::cmp::Reverse(m.email.timestamp));
        filtered.truncate(max_results as usize);
        let refs: Vec<MessageRef> = filtered
            .into_iter()
            .map(|m| MessageRef {
                id: m.email.id.clone(),
                thread_id: m.email.thread_id.clone(),
            })
            .collect();
        Ok((refs, None))
    }

    async fn list_mailbox_messages(
        &self,
        mailbox: ExtraMailbox,
        max_results: u32,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
    ) -> Result<Vec<MessageRef>> {
        let mailbox_name = mailbox.as_str();
        let guard = self.messages.read().unwrap_or_else(PoisonError::into_inner);
        let mut filtered: Vec<&FakeStoredMessage> = guard
            .iter()
            .filter(|m| m.email.mailbox == mailbox_name)
            .filter(|m| after_timestamp.is_none_or(|t| m.email.timestamp > t))
            .filter(|m| before_timestamp.is_none_or(|t| m.email.timestamp < t))
            .collect();
        filtered.sort_by_key(|m| std::cmp::Reverse(m.email.timestamp));
        filtered.truncate(max_results as usize);
        Ok(filtered
            .into_iter()
            .map(|m| MessageRef {
                id: m.email.id.clone(),
                thread_id: m.email.thread_id.clone(),
            })
            .collect())
    }

    async fn list_folders(&self) -> Result<Vec<crate::sync::folder_plan::ListedFolder>> {
        Ok(self.folders.read().unwrap_or_else(PoisonError::into_inner).clone())
    }

    async fn list_folder_messages(
        &self,
        server_path: &str,
        max_results: u32,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
    ) -> Result<Vec<MessageRef>> {
        let mailbox_value = format!("folder:{server_path}");
        let guard = self.messages.read().unwrap_or_else(PoisonError::into_inner);
        let mut filtered: Vec<&FakeStoredMessage> = guard
            .iter()
            .filter(|m| m.email.mailbox == mailbox_value)
            .filter(|m| after_timestamp.is_none_or(|t| m.email.timestamp > t))
            .filter(|m| before_timestamp.is_none_or(|t| m.email.timestamp < t))
            .collect();
        filtered.sort_by_key(|m| std::cmp::Reverse(m.email.timestamp));
        filtered.truncate(max_results as usize);
        Ok(filtered
            .into_iter()
            .map(|m| MessageRef {
                id: m.email.id.clone(),
                thread_id: m.email.thread_id.clone(),
            })
            .collect())
    }

    async fn get_message(&self, message_id: &str) -> Result<(Email, EmailCategory, Vec<AttachmentInfo>)> {
        let guard = self.messages.read().unwrap_or_else(PoisonError::into_inner);
        guard
            .iter()
            .find(|m| m.email.id == message_id)
            .map(|m| (m.email.clone(), m.category.clone(), m.attachments.clone()))
            .ok_or_else(|| crate::models::error::AppError::NotFound(format!("Fake message not found: {message_id}")))
    }

    async fn send_reply(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        thread_id: &str,
        original_message_id: Option<&str>,
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<SentMessageMeta> {
        self.sent
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeSentMessage {
                from_email: from_email.to_string(),
                to_emails: to_emails.to_vec(),
                cc_emails: cc_emails.to_vec(),
                thread_id: Some(thread_id.to_string()),
                original_message_id: original_message_id.map(str::to_string),
                subject: subject.to_string(),
                body: body.clone(),
                attachments: attachments.to_vec(),
            });
        Ok(self.send_meta.read().unwrap_or_else(PoisonError::into_inner).clone())
    }

    async fn send_new_email(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<SentMessageMeta> {
        self.sent
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeSentMessage {
                from_email: from_email.to_string(),
                to_emails: to_emails.to_vec(),
                cc_emails: cc_emails.to_vec(),
                thread_id: None,
                original_message_id: None,
                subject: subject.to_string(),
                body: body.clone(),
                attachments: attachments.to_vec(),
            });
        Ok(self.send_meta.read().unwrap_or_else(PoisonError::into_inner).clone())
    }

    async fn fetch_attachment_bytes(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        let key = (message_id.to_string(), attachment_id.to_string());
        self.attachment_bytes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                crate::models::error::AppError::NotFound(format!(
                    "Fake attachment bytes not configured: {message_id}/{attachment_id}"
                ))
            })
    }

    async fn create_folder(&self, server_path: &str) -> Result<()> {
        self.folders
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(crate::sync::folder_plan::ListedFolder {
                raw_name: server_path.to_string(),
                delimiter: Some(".".to_string()),
                attributes: vec!["\\HasNoChildren".to_string()],
            });
        self.folder_ops
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeFolderOp::Create(server_path.to_string()));
        Ok(())
    }

    async fn rename_folder(&self, old_server_path: &str, new_server_path: &str) -> Result<()> {
        let mut folders = self.folders.write().unwrap_or_else(PoisonError::into_inner);
        let entry = folders
            .iter_mut()
            .find(|f| f.raw_name == old_server_path)
            .ok_or_else(|| AppError::NotFound(format!("Fake folder not found: {old_server_path}")))?;
        entry.raw_name = new_server_path.to_string();
        drop(folders);
        self.folder_ops
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeFolderOp::Rename(
                old_server_path.to_string(),
                new_server_path.to_string(),
            ));
        Ok(())
    }

    async fn delete_folder(&self, server_path: &str) -> Result<()> {
        let mut folders = self.folders.write().unwrap_or_else(PoisonError::into_inner);
        let before = folders.len();
        folders.retain(|f| f.raw_name != server_path);
        if folders.len() == before {
            return Err(AppError::NotFound(format!("Fake folder not found: {server_path}")));
        }
        drop(folders);
        // Per IMAP semantics deleting a folder deletes its messages too.
        let mailbox_value = format!("folder:{server_path}");
        self.messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|m| m.email.mailbox != mailbox_value);
        self.folder_ops
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeFolderOp::Delete(server_path.to_string()));
        Ok(())
    }

    async fn move_message(
        &self,
        message_id: &str,
        _message_id_header: Option<&str>,
        target: &MoveTarget,
    ) -> Result<Option<MessageRef>> {
        let mut messages = self.messages.write().unwrap_or_else(PoisonError::into_inner);
        let stored = messages
            .iter_mut()
            .find(|m| m.email.id == message_id)
            .ok_or_else(|| AppError::NotFound(format!("Fake message not found: {message_id}")))?;
        stored.email.mailbox = target.mailbox_value();
        let moved_ref = MessageRef {
            id: stored.email.id.clone(),
            thread_id: stored.email.thread_id.clone(),
        };
        drop(messages);
        self.folder_ops
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeFolderOp::Move {
                message_id: message_id.to_string(),
                mailbox_value: target.mailbox_value(),
            });
        if let Some(overridden) = self.move_result.read().unwrap_or_else(PoisonError::into_inner).clone() {
            return Ok(overridden);
        }
        Ok(Some(moved_ref))
    }

    async fn set_read_state(&self, message_id: &str, read: bool) -> Result<()> {
        self.mailbox_write_gate()?;
        if let Some(stored) = self
            .messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
            .find(|m| m.email.id == message_id)
        {
            stored.email.is_read = read;
        }
        self.mailbox_ops
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeMailboxOp::SetReadState {
                message_id: message_id.to_string(),
                read,
            });
        Ok(())
    }

    async fn trash_message(&self, message_id: &str) -> Result<()> {
        self.mailbox_write_gate()?;
        self.messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|m| m.email.id != message_id);
        self.mailbox_ops
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeMailboxOp::Trash {
                message_id: message_id.to_string(),
            });
        Ok(())
    }

    async fn create_draft(
        &self,
        _from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        _attachments: &[EmailAttachment],
    ) -> Result<String> {
        let seq = self.draft_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let id = format!("fake-draft-{seq}");
        let draft = ProviderDraft {
            provider_draft_id: id.clone(),
            to_addresses: to_emails.to_vec(),
            cc_addresses: cc_emails.to_vec(),
            subject: subject.to_string(),
            body: body.text.clone(),
            body_html: body.html.clone(),
            updated_at: None,
            provider_message_id: Some(format!("fake-msg-{seq}")),
        };
        self.drafts
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone(), draft);
        Ok(id)
    }

    async fn update_draft(
        &self,
        provider_draft_id: &str,
        _from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        _attachments: &[EmailAttachment],
    ) -> Result<String> {
        // Saving a draft mints a fresh change token, mirroring Gmail replacing
        // the underlying message id on every `drafts.update`.
        let seq = self.draft_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let draft = ProviderDraft {
            provider_draft_id: provider_draft_id.to_string(),
            to_addresses: to_emails.to_vec(),
            cc_addresses: cc_emails.to_vec(),
            subject: subject.to_string(),
            body: body.text.clone(),
            body_html: body.html.clone(),
            updated_at: None,
            provider_message_id: Some(format!("fake-msg-{seq}")),
        };
        self.drafts
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(provider_draft_id.to_string(), draft);
        Ok(provider_draft_id.to_string())
    }

    async fn delete_draft(&self, provider_draft_id: &str) -> Result<()> {
        self.drafts
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(provider_draft_id);
        Ok(())
    }

    async fn list_drafts(&self, known_change_tokens: &HashMap<String, String>) -> Result<ProviderDraftPull> {
        // Mirror the real Gmail contract so service-level tests exercise the
        // skip path rather than a fake that always returns everything.
        let mut drafts = self.provider_drafts();
        drafts.sort_by(|a, b| a.provider_draft_id.cmp(&b.provider_draft_id));
        let listed: Vec<ListedDraft> = drafts
            .iter()
            .map(|d| ListedDraft {
                provider_draft_id: d.provider_draft_id.clone(),
                change_token: d.provider_message_id.clone(),
            })
            .collect();
        let plan = plan_draft_fetches(&listed, known_change_tokens);
        let changed = drafts
            .into_iter()
            .filter(|d| plan.to_fetch.contains(&d.provider_draft_id))
            .collect();
        Ok(ProviderDraftPull {
            changed,
            present_ids: plan.present_ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Email;

    fn sample_email(id: &str, ts: i64) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acc".to_string(),
            thread_id: format!("t-{id}"),
            message_id: None,
            subject: "hi".to_string(),
            sender: "Test".to_string(),
            sender_email: "test@example.com".to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: vec![],
            body: "".to_string(),
            snippet: "".to_string(),
            timestamp: ts,
            is_read: false,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: "inbox".to_string(),
            is_sent: false,
            headers: None,
        }
    }

    #[tokio::test]
    async fn fake_provider_list_and_get() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.add_message(sample_email("a", 1000), EmailCategory::Primary, vec![]);
        p.add_message(sample_email("b", 2000), EmailCategory::Primary, vec![]);
        let (refs, _) = p.list_messages(10, None, None, None, None).await.unwrap();
        // Newest first.
        assert_eq!(refs[0].id, "b");
        assert_eq!(refs[1].id, "a");
        let (msg, _, _) = p.get_message("a").await.unwrap();
        assert_eq!(msg.id, "a");
    }

    #[tokio::test]
    async fn fake_provider_records_sent() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.send_new_email(
            "me@example.com",
            &["x@y.com".to_string()],
            &[],
            "subj",
            &EmailBody::plain("body"),
            &[],
        )
        .await
        .unwrap();
        let sent = p.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].subject, "subj");
        assert_eq!(sent[0].body.text, "body");
        assert!(sent[0].body.html.is_none());
        assert!(sent[0].thread_id.is_none());
    }

    #[tokio::test]
    async fn fake_provider_records_html_and_inline_images() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        let mut body = EmailBody::with_html("plain fallback", "<p>hi</p><img src=\"cid:img1\">");
        body.inline_images.push(EmailAttachment {
            filename: "img.png".into(),
            mime_type: "image/png".into(),
            data: "AAAA".into(),
            content_id: Some("img1".into()),
            is_inline: true,
        });
        p.send_new_email("me@example.com", &["x@y.com".to_string()], &[], "subj", &body, &[])
            .await
            .unwrap();
        let sent = p.sent();
        assert_eq!(sent[0].body.html.as_deref(), Some("<p>hi</p><img src=\"cid:img1\">"));
        assert_eq!(sent[0].body.inline_images.len(), 1);
        assert_eq!(sent[0].body.inline_images[0].content_id.as_deref(), Some("img1"));
        assert!(sent[0].body.inline_images[0].is_inline);
    }

    #[tokio::test]
    async fn fake_provider_send_returns_default_meta_unless_configured() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        let meta = p
            .send_new_email(
                "me@example.com",
                &["x@y.com".to_string()],
                &[],
                "subj",
                &EmailBody::plain("body"),
                &[],
            )
            .await
            .unwrap();
        assert!(meta.provider_message_id.is_none());
        assert!(meta.provider_thread_id.is_none());
        assert!(meta.message_id_header.is_none());

        // A Gmail-shaped fake reports provider ids for the sent copy.
        p.set_send_meta(SentMessageMeta {
            provider_message_id: Some("gm-1".into()),
            provider_thread_id: Some("gt-1".into()),
            message_id_header: Some("<mid-1@local>".into()),
        });
        let meta = p
            .send_reply(
                "me@example.com",
                &["x@y.com".to_string()],
                &[],
                "thread-1",
                Some("<orig@remote>"),
                "Re: subj",
                &EmailBody::plain("body"),
                &[],
            )
            .await
            .unwrap();
        assert_eq!(meta.provider_message_id.as_deref(), Some("gm-1"));
        assert_eq!(meta.provider_thread_id.as_deref(), Some("gt-1"));
        assert_eq!(meta.message_id_header.as_deref(), Some("<mid-1@local>"));
    }

    #[tokio::test]
    async fn fake_provider_lists_configured_folders() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        assert!(p.list_folders().await.unwrap().is_empty(), "default is empty");

        p.set_folders(vec![crate::sync::folder_plan::ListedFolder {
            raw_name: "INBOX.Patienten".to_string(),
            delimiter: Some(".".to_string()),
            attributes: vec!["\\HasNoChildren".to_string()],
        }]);
        let folders = p.list_folders().await.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].raw_name, "INBOX.Patienten");
    }

    #[tokio::test]
    async fn fake_provider_folder_messages_filter_by_folder_and_timestamps() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        let mut in_folder_old = sample_email("f-old", 1000);
        in_folder_old.mailbox = "folder:INBOX.Patienten".to_string();
        let mut in_folder_new = sample_email("f-new", 3000);
        in_folder_new.mailbox = "folder:INBOX.Patienten".to_string();
        let mut other_folder = sample_email("other", 2000);
        other_folder.mailbox = "folder:INBOX.Zulieferer".to_string();
        p.add_message(in_folder_old, EmailCategory::Primary, vec![]);
        p.add_message(in_folder_new, EmailCategory::Primary, vec![]);
        p.add_message(other_folder, EmailCategory::Primary, vec![]);
        p.add_message(sample_email("inbox-msg", 2500), EmailCategory::Primary, vec![]);

        // No bounds: both folder messages, newest first, inbox and other
        // folders excluded.
        let refs = p.list_folder_messages("INBOX.Patienten", 10, None, None).await.unwrap();
        let ids: Vec<&str> = refs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["f-new", "f-old"]);

        // after_timestamp is exclusive (watermark semantics, mirrors
        // list_mailbox_messages); before_timestamp is exclusive too.
        let refs = p
            .list_folder_messages("INBOX.Patienten", 10, Some(1000), None)
            .await
            .unwrap();
        let ids: Vec<&str> = refs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["f-new"]);

        let refs = p
            .list_folder_messages("INBOX.Patienten", 10, None, Some(3000))
            .await
            .unwrap();
        let ids: Vec<&str> = refs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["f-old"]);
    }

    #[tokio::test]
    async fn fake_provider_folder_crud_mutates_listing_and_records_ops() {
        let p = FakeEmailProvider::new("me@example.com", "Me");

        p.create_folder("INBOX.Neu").await.unwrap();
        let names: Vec<String> = p
            .list_folders()
            .await
            .unwrap()
            .iter()
            .map(|f| f.raw_name.clone())
            .collect();
        assert_eq!(names, vec!["INBOX.Neu"]);

        p.rename_folder("INBOX.Neu", "INBOX.Projekte").await.unwrap();
        let names: Vec<String> = p
            .list_folders()
            .await
            .unwrap()
            .iter()
            .map(|f| f.raw_name.clone())
            .collect();
        assert_eq!(names, vec!["INBOX.Projekte"]);

        p.delete_folder("INBOX.Projekte").await.unwrap();
        assert!(p.list_folders().await.unwrap().is_empty());

        assert_eq!(
            p.folder_ops(),
            vec![
                FakeFolderOp::Create("INBOX.Neu".to_string()),
                FakeFolderOp::Rename("INBOX.Neu".to_string(), "INBOX.Projekte".to_string()),
                FakeFolderOp::Delete("INBOX.Projekte".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn fake_provider_delete_folder_drops_its_messages() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.set_folders(vec![crate::sync::folder_plan::ListedFolder {
            raw_name: "INBOX.Alt".to_string(),
            delimiter: Some(".".to_string()),
            attributes: vec![],
        }]);
        let mut in_folder = sample_email("f1", 1000);
        in_folder.mailbox = "folder:INBOX.Alt".to_string();
        p.add_message(in_folder, EmailCategory::Primary, vec![]);
        p.add_message(sample_email("i1", 2000), EmailCategory::Primary, vec![]);

        p.delete_folder("INBOX.Alt").await.unwrap();

        assert!(p.get_message("f1").await.is_err(), "folder message gone");
        assert!(p.get_message("i1").await.is_ok(), "inbox message untouched");
    }

    #[tokio::test]
    async fn fake_provider_folder_ops_error_on_missing_targets() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        assert!(p.rename_folder("Nope", "New").await.is_err());
        assert!(p.delete_folder("Nope").await.is_err());
        assert!(p.move_message("nope", None, &MoveTarget::Inbox).await.is_err());
    }

    #[tokio::test]
    async fn fake_provider_move_message_updates_mailbox_and_returns_ref() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.add_message(sample_email("m1", 1000), EmailCategory::Primary, vec![]);

        let target = MoveTarget::Folder("INBOX.Archiv".to_string());
        let moved = p.move_message("m1", Some("<mid@x>"), &target).await.unwrap();
        assert_eq!(moved.map(|r| r.id), Some("m1".to_string()));
        let (email, _, _) = p.get_message("m1").await.unwrap();
        assert_eq!(email.mailbox, "folder:INBOX.Archiv");

        // ...and back to the inbox.
        p.move_message("m1", None, &MoveTarget::Inbox).await.unwrap();
        let (email, _, _) = p.get_message("m1").await.unwrap();
        assert_eq!(email.mailbox, "inbox");
    }

    #[test]
    fn move_target_mailbox_values() {
        assert_eq!(MoveTarget::Inbox.mailbox_value(), "inbox");
        assert_eq!(
            MoveTarget::Folder("INBOX.Patienten".to_string()).mailbox_value(),
            "folder:INBOX.Patienten"
        );
    }

    /// Minimal provider that overrides nothing optional — locks the contract
    /// that folder management defaults to a typed "unsupported" error rather
    /// than silently succeeding.
    struct BareProvider;

    #[async_trait]
    impl EmailProvider for BareProvider {
        async fn get_profile(&self) -> Result<(String, String)> {
            Ok((String::new(), String::new()))
        }
        async fn list_messages(
            &self,
            _max_results: u32,
            _page_token: Option<&str>,
            _after_timestamp: Option<i64>,
            _before_timestamp: Option<i64>,
            _label_filter: Option<&str>,
        ) -> Result<(Vec<MessageRef>, Option<String>)> {
            Ok((Vec::new(), None))
        }
        async fn get_message(&self, id: &str) -> Result<(Email, EmailCategory, Vec<AttachmentInfo>)> {
            Err(crate::models::error::AppError::NotFound(id.to_string()))
        }
        async fn send_reply(
            &self,
            _from_email: &str,
            _to_emails: &[String],
            _cc_emails: &[String],
            _thread_id: &str,
            _original_message_id: Option<&str>,
            _subject: &str,
            _body: &EmailBody,
            _attachments: &[EmailAttachment],
        ) -> Result<SentMessageMeta> {
            Ok(SentMessageMeta::default())
        }
        async fn send_new_email(
            &self,
            _from_email: &str,
            _to_emails: &[String],
            _cc_emails: &[String],
            _subject: &str,
            _body: &EmailBody,
            _attachments: &[EmailAttachment],
        ) -> Result<SentMessageMeta> {
            Ok(SentMessageMeta::default())
        }
        async fn fetch_attachment_bytes(&self, _message_id: &str, _attachment_id: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn folder_management_defaults_to_unsupported_error() {
        let p = BareProvider;
        for result in [
            p.create_folder("X").await,
            p.rename_folder("X", "Y").await,
            p.delete_folder("X").await,
            p.move_message("m", None, &MoveTarget::Inbox).await.map(|_| ()),
        ] {
            match result {
                Err(crate::models::error::AppError::InvalidInput(msg)) => {
                    assert!(msg.contains("not supported"), "unexpected message: {msg}");
                }
                other => panic!("expected InvalidInput, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn mailbox_state_writes_default_to_unsupported_error() {
        // A provider that doesn't override these must fail loudly rather than
        // silently pretend the push happened — callers gate on
        // `provider_supports_mailbox_writes` and keep the change local instead.
        let p = BareProvider;
        for result in [p.set_read_state("m", true).await, p.trash_message("m").await] {
            match result {
                Err(crate::models::error::AppError::InvalidInput(msg)) => {
                    assert!(msg.contains("not supported"), "unexpected message: {msg}");
                }
                other => panic!("expected InvalidInput, got {other:?}"),
            }
        }
    }

    #[test]
    fn only_gmail_supports_server_side_mailbox_writes() {
        assert!(provider_supports_mailbox_writes("gmail"));
        assert!(
            !provider_supports_mailbox_writes("imap"),
            "IMAP flag/move write-back is not implemented yet — must stay local-only"
        );
        assert!(!provider_supports_mailbox_writes("outlook"));
    }

    #[tokio::test]
    async fn fake_provider_records_read_state_and_trash_calls() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.set_read_state("m-1", true).await.unwrap();
        p.set_read_state("m-2", false).await.unwrap();
        p.trash_message("m-1").await.unwrap();

        assert_eq!(
            p.mailbox_ops(),
            vec![
                FakeMailboxOp::SetReadState {
                    message_id: "m-1".to_string(),
                    read: true
                },
                FakeMailboxOp::SetReadState {
                    message_id: "m-2".to_string(),
                    read: false
                },
                FakeMailboxOp::Trash {
                    message_id: "m-1".to_string()
                },
            ]
        );
    }

    #[tokio::test]
    async fn fake_provider_can_simulate_a_failing_mailbox_write() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.fail_mailbox_writes("mailbox is over quota");

        let err = p.trash_message("m-1").await.unwrap_err();
        assert!(err.to_string().contains("over quota"), "unexpected error: {err}");
        assert!(p.mailbox_ops().is_empty(), "a failed write must not be recorded");
    }

    #[test]
    fn email_attachment_serde_round_trip_inline() {
        let att = EmailAttachment {
            filename: "logo.png".into(),
            mime_type: "image/png".into(),
            data: "QUFB".into(),
            content_id: Some("logo".into()),
            is_inline: true,
        };
        let json = serde_json::to_string(&att).unwrap();
        // camelCase + inline fields present
        assert!(json.contains("\"mimeType\":\"image/png\""));
        assert!(json.contains("\"contentId\":\"logo\""));
        assert!(json.contains("\"isInline\":true"));
        let back: EmailAttachment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content_id.as_deref(), Some("logo"));
        assert!(back.is_inline);
    }

    #[test]
    fn email_attachment_serde_round_trip_regular() {
        // Regular (non-inline) attachment — inline fields should be omitted on
        // the wire so frontend payloads stay minimal and existing JSON without
        // these fields still deserializes.
        let att = EmailAttachment {
            filename: "report.pdf".into(),
            mime_type: "application/pdf".into(),
            data: "QUFB".into(),
            content_id: None,
            is_inline: false,
        };
        let json = serde_json::to_string(&att).unwrap();
        assert!(!json.contains("contentId"));
        assert!(!json.contains("isInline"));

        // Deserializing legacy payload without the new fields still works.
        let legacy = r#"{"filename":"a.bin","mimeType":"application/octet-stream","data":"AA=="}"#;
        let back: EmailAttachment = serde_json::from_str(legacy).unwrap();
        assert_eq!(back.filename, "a.bin");
        assert!(back.content_id.is_none());
        assert!(!back.is_inline);
    }

    #[test]
    fn email_body_constructors() {
        let plain = EmailBody::plain("hello");
        assert_eq!(plain.text, "hello");
        assert!(plain.html.is_none());
        assert!(plain.inline_images.is_empty());
        assert!(!plain.has_html());
        // Footer language defaults to English so direct constructions are deterministic.
        assert_eq!(plain.language, Language::En);

        let rich = EmailBody::with_html("hello", "<p>hello</p>");
        assert!(rich.has_html());
        assert_eq!(rich.html.as_deref(), Some("<p>hello</p>"));
    }

    #[test]
    fn email_body_with_language_overrides_default() {
        let body = EmailBody::plain("hi").with_language(Language::De);
        assert_eq!(body.language, Language::De);
    }

    #[test]
    fn footer_plain_is_localized_and_uses_emailops_brand() {
        // Brand name is "EmailOps" (capital O), never "Emailops", in every locale.
        for lang in Language::ALL {
            let footer = email_footer_plain(lang);
            assert!(
                footer.contains("EmailOps"),
                "{lang:?} footer must brand EmailOps: {footer}"
            );
            assert!(
                !footer.contains("Emailops"),
                "{lang:?} footer must not lowercase the O: {footer}"
            );
            assert!(footer.contains("https://getemailops.com"));
        }
        assert!(email_footer_plain(Language::En).contains("Sent with EmailOps"));
        assert!(email_footer_plain(Language::Es).contains("Enviado con EmailOps"));
        assert!(email_footer_plain(Language::Fr).contains("Envoyé avec EmailOps"));
        assert!(email_footer_plain(Language::De).contains("Gesendet mit EmailOps"));
    }

    #[test]
    fn footer_html_is_localized_and_links_emailops() {
        for lang in Language::ALL {
            let footer = email_footer_html(lang);
            assert!(
                footer.contains(">EmailOps</a>"),
                "{lang:?} html footer must link EmailOps: {footer}"
            );
            assert!(!footer.contains(">Emailops</a>"));
            assert!(footer.contains("href=\"https://getemailops.com\""));
        }
        assert!(email_footer_html(Language::En).contains("Sent with <a"));
        assert!(email_footer_html(Language::Es).contains("Enviado con <a"));
    }

    #[tokio::test]
    async fn fake_provider_after_timestamp_filter() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.add_message(sample_email("a", 1000), EmailCategory::Primary, vec![]);
        p.add_message(sample_email("b", 2000), EmailCategory::Primary, vec![]);
        let (refs, _) = p.list_messages(10, None, Some(1500), None, None).await.unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].id, "b");
    }
}
