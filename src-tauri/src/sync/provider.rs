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
/// read/unread, star, archive and delete back to the account so the change is
/// visible in the provider's own clients. Gmail implements them via
/// `messages.modify` (`UNREAD`, `STARRED`, `INBOX` labels) / `messages.trash`,
/// IMAP via `UID STORE` on `\Seen` / `\Flagged` and moves to the Archive and
/// Trash folders, Outlook via Graph `isRead` / `flag` and moves to `archive` /
/// `deleteditems`. An unknown provider keeps its mailbox state local to
/// EmailOps.
pub fn provider_supports_mailbox_writes(provider: &str) -> bool {
    matches!(provider, "gmail" | "imap" | "outlook")
}

/// An attachment to include in an outgoing email.
///
/// `content_id` + `is_inline = true` mark this as an inline image referenced
/// from the HTML body via a `cid:<content_id>` URI (RFC 2392). Regular file
/// attachments leave both at their default (None / false) so the SMTP layer
/// renders them as `Content-Disposition: attachment`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
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
    format!(
        "\n\n--\n{} EmailOps\nhttps://getemailops.com/?utm_source=email_footer",
        footer_prefix(language)
    )
}

/// HTML footer appended to every outgoing email, in the user's UI language.
pub fn email_footer_html(language: Language) -> String {
    format!(
        "<br><br><hr style=\"border:none;border-top:1px solid #eee;margin:16px 0\">\
         <p style=\"color:#888;font-size:12px;margin:0\">{} \
         <a href=\"https://getemailops.com/?utm_source=email_footer\" style=\"color:#888\">EmailOps</a></p>",
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

/// Where a message sits at the provider right now: the id it is addressable
/// by (unchanged on Gmail, re-keyed on IMAP/Graph) and its `emails.mailbox`
/// value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageLocation {
    pub id: String,
    pub mailbox: String,
}

/// What the provider reports, right now, for a message the app already stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteMessageState {
    /// Still addressable under the stored id. `is_starred` is `None` when the
    /// provider's answer did not say (the message keeps its stored star).
    Present { is_read: bool, is_starred: Option<bool> },
    /// Nothing answers to the stored id any more: the message was deleted, or
    /// moved — which re-keys it on IMAP and Graph. [`EmailProvider::locate_message`]
    /// tells the two apart.
    Missing,
}

/// One change to a message, as the provider's change log reports it (Gmail's
/// History API). Labels are the provider's own ids (`UNREAD`, `INBOX`, `TRASH`…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageChange {
    LabelsAdded {
        id: String,
        labels: Vec<String>,
    },
    LabelsRemoved {
        id: String,
        labels: Vec<String>,
    },
    /// Deleted for good — not moved to Trash, which is a label change.
    Deleted {
        id: String,
    },
}

/// One page of the provider's change log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    /// Changes in the order they happened.
    pub changes: Vec<MessageChange>,
    /// The cursor a later listing starts from once this page is applied: the
    /// page's last record, or the mailbox's current position on the last page.
    pub resume_cursor: String,
    /// Set while more pages follow for the same start cursor.
    pub next_page_token: Option<String>,
}

/// What [`EmailProvider::list_history`] answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryListing {
    Page(HistoryPage),
    /// The provider no longer keeps the log back to the start cursor: what
    /// changed since then cannot be replayed.
    CursorExpired,
}

/// The labels the provider holds, right now, for a message the app stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteLabels {
    Present(Vec<String>),
    /// Nothing answers to the id any more: deleted for good.
    Missing,
}

/// The UIDVALIDITY an IMAP server reports for one mailbox the sync stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderUidValidity {
    /// The `emails.mailbox` value of the mailbox's messages.
    pub mailbox: String,
    /// What every stored id of this mailbox starts with; the rest is the UID.
    pub id_prefix: String,
    pub uid_validity: u32,
}

/// One message of a mailbox as the provider has it now, with what identifies
/// it independently of its provider id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageIdentity {
    /// The id the sync would store this message under today.
    pub id: String,
    /// RFC 5322 Message-ID header.
    pub message_id: Option<String>,
    /// The timestamp the sync would store for it (IMAP INTERNALDATE).
    pub timestamp: Option<i64>,
}

/// Everything a reply needs to know about the message it answers.
///
/// Grouped rather than passed loose because the fields are easy to confuse and
/// each provider threads on a different one — picking the wrong field fails
/// silently (a split thread) or addresses the wrong resource:
///
/// - Gmail: `thread_id` (its `threadId`) plus the RFC headers.
/// - IMAP: the RFC headers alone; the server does no threading of its own.
/// - Outlook: `provider_message_id`, because Graph's `/reply` addresses the
///   parent by item id and writes the RFC headers itself.
#[derive(Debug, Clone, Copy)]
pub struct ReplyTarget<'a> {
    /// Provider-side id of the parent — the `emails.id` we stored: a Gmail or
    /// Graph message id, or the IMAP UID key.
    ///
    /// Distinct from `message_id`: for Outlook this is the opaque Graph item
    /// id (`AQMkAD…`) while `message_id` is `internetMessageId` (`<abc@host>`).
    /// `/me/messages/{id}/reply` only accepts the former, and
    /// `internetMessageId` is frequently absent besides.
    pub provider_message_id: &'a str,
    /// Provider thread id. Gmail sends it back as `threadId`; IMAP has none.
    pub thread_id: &'a str,
    /// The parent's RFC 5322 `Message-ID`, for `In-Reply-To`.
    pub message_id: Option<&'a str>,
    /// The parent's RFC 5322 `References`. Goes out with the parent's
    /// Message-ID appended (§3.6.4) so the reply continues the thread instead
    /// of rooting a new one. `None` when the parent carried no chain, or
    /// predates us storing it.
    pub references: Option<&'a str>,
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
    ///
    /// `from_name` is the display name for the From header (`None` sends the
    /// bare address). Outlook ignores it: Graph takes the sender name from the
    /// mailbox itself.
    ///
    /// `target` describes the message being answered — see [`ReplyTarget`],
    /// which exists because the three providers each need a *different* one of
    /// its fields to thread a reply correctly.
    async fn send_reply(
        &self,
        from_email: &str,
        from_name: Option<&str>,
        to_emails: &[String],
        cc_emails: &[String],
        target: &ReplyTarget<'_>,
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
        from_name: Option<&str>,
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

    /// Ids of every message in the mailbox that carries attachments, when the
    /// provider can answer with a cheap server-side search (Gmail
    /// `has:attachment`, Graph `hasAttachments eq true`, an IMAP `HEADER
    /// Content-Type` search per folder). `None` when it cannot.
    /// Drives the one-time attachment-metadata backfill, which then fetches
    /// only those messages instead of the whole mailbox.
    async fn list_message_ids_with_attachments(&self) -> Result<Option<Vec<String>>> {
        Ok(None)
    }

    /// The signature the provider's own web client inserts when sending from
    /// `email` (Gmail's "Send mail as" signature), as HTML. `None` when the
    /// provider keeps none in its API (Graph does not expose Outlook's, IMAP
    /// has none) or none is set.
    async fn get_signature(&self, _email: &str) -> Result<Option<String>> {
        Ok(None)
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

    /// Star or unstar one message at the provider (Gmail `STARRED`, Graph
    /// `flag.flagStatus`, IMAP `\Flagged`).
    async fn set_starred(&self, _message_id: &str, _starred: bool) -> Result<()> {
        Err(AppError::InvalidInput(
            "mailbox state writes are not supported by this provider".to_string(),
        ))
    }

    /// Take one message out of the inbox while keeping it in the account:
    /// Gmail removes the `INBOX` label (id unchanged), Graph moves it to the
    /// well-known `archive` folder and IMAP to the folder flagged
    /// `\Archive` (or named Archive/Archives) — both re-key it, so the
    /// answer is the id and `emails.mailbox` value the message has now.
    ///
    /// `message_id_header` is the RFC 5322 Message-ID when known; IMAP uses it
    /// to find the moved message's new UID. `AppError::NotFound` means the
    /// provider no longer has the message under this id. An IMAP account
    /// without an archive folder is refused with `AppError::InvalidInput`
    /// rather than archived locally only.
    ///
    /// The inverse is [`Self::move_message`] with [`MoveTarget::Inbox`].
    async fn archive_message(&self, _message_id: &str, _message_id_header: Option<&str>) -> Result<MessageLocation> {
        Err(AppError::InvalidInput(
            "mailbox state writes are not supported by this provider".to_string(),
        ))
    }

    /// File one message in the provider's Spam/Junk folder, as the provider's
    /// own "Report spam" does: Gmail adds `SPAM` and drops `INBOX` (id
    /// unchanged), Graph moves it to the well-known `junkemail` folder and
    /// IMAP to the `\Junk` folder (or one named like it) — both re-key it, so
    /// the answer is the id it has now and the `spam` mailbox.
    ///
    /// `AppError::NotFound` means the provider no longer has the message under
    /// this id; an IMAP account without a Junk folder is
    /// `AppError::NoSpamFolder`. The inverse is [`Self::move_message`] with
    /// [`MoveTarget::Inbox`].
    async fn move_to_spam(&self, _message_id: &str, _message_id_header: Option<&str>) -> Result<MessageLocation> {
        Err(AppError::InvalidInput(
            "mailbox state writes are not supported by this provider".to_string(),
        ))
    }

    /// Move one message to the provider's Trash. Recoverable by the user from
    /// the provider's own UI — this is not a permanent delete.
    ///
    /// `message_id_header` is the message's RFC 5322 Message-ID when known.
    /// IMAP addresses a message by a UID that a server-side mailbox rebuild
    /// can hand to a different message, so it checks the header before moving
    /// anything; Gmail and Graph ids are never reused and ignore it.
    ///
    /// `AppError::NotFound` means the provider no longer has the message under
    /// this id, so there is nothing left to trash.
    async fn trash_message(&self, _message_id: &str, _message_id_header: Option<&str>) -> Result<()> {
        Err(AppError::InvalidInput(
            "mailbox state writes are not supported by this provider".to_string(),
        ))
    }

    /// Where a message the app already stores lives at the provider right now.
    /// Lets sync catch up with moves the user made in the provider's own
    /// clients, which the insert-only fetch passes never see.
    ///
    /// `message_id_header` is the message's RFC 5322 Message-ID when known:
    /// Gmail keeps a message's id across a move, but IMAP (new UID) and Graph
    /// (new item id) re-key it, so there the header is the only way to find it
    /// again — and the returned [`MessageLocation::id`] is then the new id the
    /// local row must be re-keyed to. `Ok(None)` means the provider no longer
    /// has the message at all (deleted forever).
    async fn locate_message(
        &self,
        _message_id: &str,
        _message_id_header: Option<&str>,
    ) -> Result<Option<MessageLocation>> {
        Err(AppError::InvalidInput(
            "locating a message is not supported by this provider".to_string(),
        ))
    }

    /// The provider's current state for messages the app already stores, so a
    /// sync can pick up what the user did in another client (read/unread,
    /// delete, move) — the fetch passes skip every id they already know.
    ///
    /// The map holds an entry only for ids the provider actually checked. An id
    /// it could not check (its folder would not open, its sub-request was
    /// throttled) is left out, and the caller must leave that row alone: only
    /// an explicit [`RemoteMessageState::Missing`] means the message is gone.
    ///
    /// `Ok(None)` means the provider has no such refresh.
    async fn fetch_message_states(
        &self,
        _message_ids: &[String],
    ) -> Result<Option<HashMap<String, RemoteMessageState>>> {
        Ok(None)
    }

    /// The current position of the provider's change log, to start following
    /// it from. `Ok(None)` means the provider has no change log (everything
    /// but Gmail), and the stored-mail refresh polls message states instead.
    async fn history_cursor(&self) -> Result<Option<String>> {
        Ok(None)
    }

    /// One page of what changed after `cursor`. `page_token` continues a
    /// listing started with the same `cursor`.
    async fn list_history(&self, _cursor: &str, _page_token: Option<&str>) -> Result<HistoryListing> {
        Err(AppError::InvalidInput("this provider has no change log".to_string()))
    }

    /// The current labels of stored messages, for the bounded reconciliation
    /// that replaces the change log when its cursor expired. Like
    /// [`Self::fetch_message_states`], the map holds only the ids the provider
    /// actually checked.
    async fn fetch_message_labels(&self, _message_ids: &[String]) -> Result<HashMap<String, RemoteLabels>> {
        Ok(HashMap::new())
    }

    /// The current UIDVALIDITY of every mailbox the sync stores mail from.
    /// IMAP only: its message ids embed a UID, which a server-side mailbox
    /// rebuild re-assigns. Gmail and Graph ids are never reused, so the default
    /// reports nothing. A mailbox the server could not answer for is left out.
    async fn folder_uid_validities(&self) -> Result<Vec<FolderUidValidity>> {
        Ok(Vec::new())
    }

    /// Every message currently in `mailbox` (an `emails.mailbox` value), so
    /// rows stored before a UIDVALIDITY change can be matched to the ids their
    /// messages have now. Only called for a mailbox
    /// [`Self::folder_uid_validities`] reported.
    async fn list_mailbox_identities(&self, _mailbox: &str) -> Result<Vec<MessageIdentity>> {
        Err(AppError::InvalidInput(
            "listing mailbox identities is not supported by this provider".to_string(),
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
    /// `reply` is the email a reply draft answers: the provider copy is then a
    /// reply itself (threading headers, the provider's thread), so it can be
    /// matched back to its thread if the local row is lost.
    #[allow(clippy::too_many_arguments)]
    async fn create_draft(
        &self,
        _from_email: &str,
        _to_emails: &[String],
        _cc_emails: &[String],
        _subject: &str,
        _body: &EmailBody,
        _attachments: &[EmailAttachment],
        _reply: Option<&ReplyTarget<'_>>,
    ) -> Result<String> {
        Err(AppError::InvalidInput(
            "drafts are not supported by this provider".to_string(),
        ))
    }

    /// Update an existing provider draft in place. Returns the (possibly new)
    /// provider draft id. `reply` as in [`EmailProvider::create_draft`].
    #[allow(clippy::too_many_arguments)]
    async fn update_draft(
        &self,
        _provider_draft_id: &str,
        _from_email: &str,
        _to_emails: &[String],
        _cc_emails: &[String],
        _subject: &str,
        _body: &EmailBody,
        _attachments: &[EmailAttachment],
        _reply: Option<&ReplyTarget<'_>>,
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
    /// Provider calls in the order they were made, so a test can assert on the
    /// shape of a sync — e.g. that downloading starts before listing ends.
    calls: std::sync::Arc<std::sync::RwLock<Vec<String>>>,
    /// When `Some`, every mailbox-state write fails instead of being recorded
    /// — simulates an offline or refusing provider, or a message it lost.
    mailbox_write_failure: std::sync::RwLock<Option<FakeWriteFailure>>,
    /// When `Some`, `create_draft` / `update_draft` fail with this message —
    /// simulates a provider that is reachable but refusing draft writes.
    draft_write_failure: std::sync::RwLock<Option<String>>,
    /// Message ids whose `get_message` fails — simulates a message the
    /// provider cannot return (rate limit, deleted server-side).
    failing_messages: std::sync::RwLock<std::collections::HashSet<String>>,
    /// When set, what `list_message_ids_with_attachments` answers instead of
    /// the stored messages that have attachments (`None` = no search).
    attachment_listing: std::sync::RwLock<Option<Option<Vec<String>>>>,
    /// Whether `fetch_message_states` answers from the stored messages. Off by
    /// default so a sync test that seeds local rows the fake never heard of
    /// does not see them reported as deleted upstream.
    reports_message_states: std::sync::atomic::AtomicBool,
    /// Ids `fetch_message_states` leaves out of its answer — a folder that
    /// would not open, a throttled sub-request.
    unverifiable_messages: std::sync::RwLock<std::collections::HashSet<String>>,
    /// What `folder_uid_validities` reports — empty unless a test models an
    /// IMAP server.
    uid_validities: std::sync::RwLock<Vec<FolderUidValidity>>,
    /// When `Some`, `list_mailbox_identities` fails with this message.
    identity_listing_failure: std::sync::RwLock<Option<String>>,
    /// The change log `history_cursor` / `list_history` answer from — `None`
    /// unless a test models Gmail with [`Self::enable_history`].
    history: std::sync::RwLock<Option<FakeHistory>>,
    /// When `Some`, `fetch_message_labels` fails with this message.
    label_fetch_failure: std::sync::RwLock<Option<String>>,
    /// When `Some`, what `archive_message` answers instead of the archived
    /// message's own id under `archive` — models a provider that re-keys
    /// (Graph, IMAP) or files the message in a folder (IMAP).
    archive_location: std::sync::RwLock<Option<MessageLocation>>,
    /// When `Some`, `send_reply` / `send_new_email` fail with this message and
    /// record nothing — a provider refusing the send (5xx, offline).
    send_failure: std::sync::RwLock<Option<String>>,
    /// What `get_signature` answers for the profile address.
    signature: std::sync::RwLock<Option<String>>,
}

/// The change log of a [`FakeEmailProvider`] modelling Gmail.
#[derive(Debug, Clone)]
struct FakeHistory {
    /// `(record id, change)`, ids increasing.
    records: Vec<(u64, MessageChange)>,
    /// The mailbox's current position.
    current: u64,
    /// Cursors below this are no longer kept.
    oldest_kept: u64,
    /// Records per page.
    page_size: usize,
    /// When `Some`, `list_history` fails with this message.
    failure: Option<String>,
}

/// The Gmail labels a fake message stands for.
fn fake_labels(email: &Email) -> Vec<String> {
    let mut labels: Vec<String> = match email.mailbox.as_str() {
        "trash" => Some("TRASH"),
        "spam" => Some("SPAM"),
        "sent" => Some("SENT"),
        // Archived Gmail mail is mail without the INBOX label.
        "archive" => None,
        _ => Some("INBOX"),
    }
    .into_iter()
    .map(str::to_string)
    .collect();
    if !email.is_read {
        labels.push("UNREAD".to_string());
    }
    if email.is_starred {
        labels.push("STARRED".to_string());
    }
    labels
}

/// How [`FakeEmailProvider`] fails a mailbox-state write.
#[derive(Debug, Clone)]
enum FakeWriteFailure {
    /// Transient: offline, 5xx, a refusing server.
    Unavailable(String),
    /// The provider no longer has the message under that id.
    MessageGone,
}

/// A mailbox-state call recorded by [`FakeEmailProvider`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeMailboxOp {
    SetReadState {
        message_id: String,
        read: bool,
    },
    Trash {
        message_id: String,
        /// The Message-ID header the caller vouched for the message with.
        message_id_header: Option<String>,
    },
    SetStarred {
        message_id: String,
        starred: bool,
    },
    Archive {
        message_id: String,
    },
    Spam {
        message_id: String,
    },
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
    /// Provider-side id of the message replied to (`ReplyTarget::provider_message_id`).
    /// `None` for a fresh send. Outlook threads on exactly this.
    pub provider_message_id: Option<String>,
    pub from_email: String,
    pub from_name: Option<String>,
    pub to_emails: Vec<String>,
    pub cc_emails: Vec<String>,
    pub thread_id: Option<String>,
    pub original_message_id: Option<String>,
    /// The parent's `References` chain as handed to the provider, so tests can
    /// assert a reply continues its thread instead of rooting a new one.
    pub original_references: Option<String>,
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
            calls: std::sync::Arc::new(std::sync::RwLock::new(Vec::new())),
            mailbox_write_failure: std::sync::RwLock::new(None),
            draft_write_failure: std::sync::RwLock::new(None),
            failing_messages: std::sync::RwLock::new(std::collections::HashSet::new()),
            attachment_listing: std::sync::RwLock::new(None),
            reports_message_states: std::sync::atomic::AtomicBool::new(false),
            unverifiable_messages: std::sync::RwLock::new(std::collections::HashSet::new()),
            uid_validities: std::sync::RwLock::new(Vec::new()),
            identity_listing_failure: std::sync::RwLock::new(None),
            history: std::sync::RwLock::new(None),
            label_fetch_failure: std::sync::RwLock::new(None),
            archive_location: std::sync::RwLock::new(None),
            send_failure: std::sync::RwLock::new(None),
            signature: std::sync::RwLock::new(None),
        }
    }

    /// Make `archive_message` answer `location` from now on.
    pub fn set_archive_location(&self, location: MessageLocation) {
        *self.archive_location.write().unwrap_or_else(PoisonError::into_inner) = Some(location);
    }

    /// Report `uid_validity` for `mailbox` from now on, replacing any earlier
    /// value — calling it again with another number is a server-side rebuild.
    pub fn set_folder_uid_validity(&self, mailbox: &str, id_prefix: &str, uid_validity: u32) {
        let mut validities = self.uid_validities.write().unwrap_or_else(PoisonError::into_inner);
        validities.retain(|v| v.mailbox != mailbox);
        validities.push(FolderUidValidity {
            mailbox: mailbox.to_string(),
            id_prefix: id_prefix.to_string(),
            uid_validity,
        });
    }

    /// Make `list_mailbox_identities` fail with `message`.
    pub fn fail_identity_listing(&self, message: impl Into<String>) {
        *self
            .identity_listing_failure
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(message.into());
    }

    /// Give the fake a change log positioned at `current`, as Gmail has.
    pub fn enable_history(&self, current: u64) {
        *self.history.write().unwrap_or_else(PoisonError::into_inner) = Some(FakeHistory {
            records: Vec::new(),
            current,
            oldest_kept: 0,
            page_size: 100,
            failure: None,
        });
    }

    fn with_history(&self, change: impl FnOnce(&mut FakeHistory)) {
        if let Some(history) = self.history.write().unwrap_or_else(PoisonError::into_inner).as_mut() {
            change(history);
        }
    }

    /// Append a change to the log the way another client's action would.
    pub fn record_history(&self, change: MessageChange) {
        self.with_history(|h| {
            h.current += 1;
            h.records.push((h.current, change));
        });
    }

    /// Stop keeping the log for every cursor handed out so far.
    pub fn expire_history(&self) {
        self.with_history(|h| {
            h.current += 1;
            h.oldest_kept = h.current;
        });
    }

    pub fn set_history_page_size(&self, page_size: usize) {
        self.with_history(|h| h.page_size = page_size);
    }

    /// Make `list_history` fail (`Some`) or answer again (`None`).
    pub fn fail_history_listing(&self, failure: Option<&str>) {
        self.with_history(|h| h.failure = failure.map(str::to_string));
    }

    /// Make every send fail with `message` (`Some`), or succeed again (`None`).
    /// Set the provider-side signature `get_signature` reports.
    pub fn set_signature(&self, html: Option<&str>) {
        *self.signature.write().unwrap_or_else(PoisonError::into_inner) = html.map(str::to_string);
    }

    pub fn fail_sends(&self, message: Option<&str>) {
        *self.send_failure.write().unwrap_or_else(PoisonError::into_inner) = message.map(str::to_string);
    }

    fn send_refusal(&self) -> Result<()> {
        match self.send_failure.read().unwrap_or_else(PoisonError::into_inner).clone() {
            Some(message) => Err(crate::models::error::AppError::SyncError(message)),
            None => Ok(()),
        }
    }

    /// Make `fetch_message_labels` fail.
    pub fn fail_label_fetch(&self, message: impl Into<String>) {
        *self.label_fetch_failure.write().unwrap_or_else(PoisonError::into_inner) = Some(message.into());
    }

    /// Make `fetch_message_states` answer from the fake mailbox: a stored
    /// message is `Present` with its read flag, anything else is `Missing`.
    pub fn report_message_states(&self) {
        self.reports_message_states
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Leave `message_id` out of what `fetch_message_states` answers.
    pub fn make_state_unverifiable(&self, message_id: impl Into<String>) {
        self.unverifiable_messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(message_id.into());
    }

    /// Change a message the way another client would: flip its read flag.
    pub fn set_remote_read(&self, message_id: &str, read: bool) {
        if let Some(stored) = self
            .messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
            .find(|m| m.email.id == message_id)
        {
            stored.email.is_read = read;
        }
    }

    /// Delete a message the way another client would: it is simply gone.
    pub fn remove_message(&self, message_id: &str) {
        self.messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|m| m.email.id != message_id);
    }

    /// Move a message the way another client would on IMAP or Graph: it lands
    /// in `mailbox` under a new id, and the old id stops resolving.
    pub fn relocate_message(&self, message_id: &str, new_id: &str, mailbox: &str) {
        if let Some(stored) = self
            .messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
            .find(|m| m.email.id == message_id)
        {
            stored.email.id = new_id.to_string();
            stored.email.mailbox = mailbox.to_string();
        }
    }

    /// Snapshot of the mailbox-state writes performed so far.
    fn record_call(&self, name: &str) {
        self.calls
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_string());
    }

    /// Provider calls in order.
    pub fn calls(&self) -> Vec<String> {
        self.calls.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// A handle to the call log that outlives the provider being moved into a
    /// sync (which takes `Box<dyn EmailProvider>` by value).
    pub fn call_log(&self) -> std::sync::Arc<std::sync::RwLock<Vec<String>>> {
        std::sync::Arc::clone(&self.calls)
    }

    pub fn mailbox_ops(&self) -> Vec<FakeMailboxOp> {
        self.mailbox_ops.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Fix what `list_message_ids_with_attachments` answers: `None` behaves
    /// like a provider with no attachment search, `Some(ids)` lists exactly
    /// those ids (which may disagree with what `get_message` returns).
    pub fn set_attachment_listing(&self, listing: Option<Vec<String>>) {
        *self.attachment_listing.write().unwrap_or_else(PoisonError::into_inner) = Some(listing);
    }

    /// Make `get_message` fail for `message_id` from now on.
    pub fn fail_message(&self, message_id: impl Into<String>) {
        self.failing_messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(message_id.into());
    }

    /// Make every subsequent draft create/update fail with `message`, or let
    /// them through again with `None`.
    pub fn fail_draft_writes(&self, message: Option<&str>) {
        *self.draft_write_failure.write().unwrap_or_else(PoisonError::into_inner) = message.map(String::from);
    }

    fn draft_write_result(&self) -> Result<()> {
        match self
            .draft_write_failure
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_deref()
        {
            Some(message) => Err(AppError::SyncError(message.to_string())),
            None => Ok(()),
        }
    }

    /// Make every subsequent mailbox-state write fail with `message`.
    pub fn fail_mailbox_writes(&self, message: impl Into<String>) {
        *self
            .mailbox_write_failure
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(FakeWriteFailure::Unavailable(message.into()));
    }

    /// Make every subsequent mailbox-state write answer "no such message",
    /// as a provider does once the message was deleted or re-keyed upstream.
    pub fn fail_mailbox_writes_as_not_found(&self) {
        *self
            .mailbox_write_failure
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(FakeWriteFailure::MessageGone);
    }

    /// Let mailbox-state writes succeed again.
    pub fn restore_mailbox_writes(&self) {
        *self
            .mailbox_write_failure
            .write()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// `Err` when a failure has been configured, `Ok` otherwise.
    fn mailbox_write_gate(&self) -> Result<()> {
        match self
            .mailbox_write_failure
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            Some(FakeWriteFailure::Unavailable(message)) => Err(AppError::SyncError(message)),
            Some(FakeWriteFailure::MessageGone) => Err(AppError::NotFound("Fake message is gone".to_string())),
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

    async fn get_signature(&self, email: &str) -> Result<Option<String>> {
        if !email.eq_ignore_ascii_case(&self.profile_email) {
            return Ok(None);
        }
        Ok(self.signature.read().unwrap_or_else(PoisonError::into_inner).clone())
    }

    /// Paginated, like every real provider: `page_token` is the offset into the
    /// filtered set, and a token comes back whenever messages remain.
    ///
    /// The fake used to return the first `max_results` and no token, so nothing
    /// that depends on paging could be tested — including the sync loop's
    /// listing/downloading interleave, which is why a mailbox that took
    /// hundreds of pages to enumerate showed nothing until it had finished.
    async fn list_messages(
        &self,
        max_results: u32,
        page_token: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        _label_filter: Option<&str>,
    ) -> Result<(Vec<MessageRef>, Option<String>)> {
        self.record_call("list_messages");
        let offset: usize = page_token.and_then(|t| t.parse().ok()).unwrap_or(0);
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
        let total = filtered.len();
        let page: Vec<&FakeStoredMessage> = filtered.into_iter().skip(offset).take(max_results as usize).collect();
        let next_offset = offset + page.len();
        let refs: Vec<MessageRef> = page
            .into_iter()
            .map(|m| MessageRef {
                id: m.email.id.clone(),
                thread_id: m.email.thread_id.clone(),
            })
            .collect();
        let next_page = (next_offset < total && !refs.is_empty()).then(|| next_offset.to_string());
        Ok((refs, next_page))
    }

    async fn list_message_ids_with_attachments(&self) -> Result<Option<Vec<String>>> {
        self.record_call("list_message_ids_with_attachments");
        if let Some(listing) = self
            .attachment_listing
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Ok(listing);
        }
        let guard = self.messages.read().unwrap_or_else(PoisonError::into_inner);
        Ok(Some(
            guard
                .iter()
                .filter(|m| !m.attachments.is_empty())
                .map(|m| m.email.id.clone())
                .collect(),
        ))
    }

    /// Overrides the sequential default purely to record the call, so a test
    /// can assert on the order of listing versus downloading.
    async fn batch_get_messages(
        &self,
        message_ids: &[&str],
    ) -> Result<Vec<Result<(Email, EmailCategory, Vec<AttachmentInfo>)>>> {
        self.record_call("batch_get_messages");
        let mut results = Vec::with_capacity(message_ids.len());
        for id in message_ids {
            results.push(self.get_message(id).await);
        }
        Ok(results)
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
        if self
            .failing_messages
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(message_id)
        {
            return Err(crate::models::error::AppError::SyncError(format!(
                "Fake fetch failure: {message_id}"
            )));
        }
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
        from_name: Option<&str>,
        to_emails: &[String],
        cc_emails: &[String],
        target: &ReplyTarget<'_>,
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<SentMessageMeta> {
        self.send_refusal()?;
        self.sent
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeSentMessage {
                from_email: from_email.to_string(),
                from_name: from_name.map(str::to_string),
                to_emails: to_emails.to_vec(),
                cc_emails: cc_emails.to_vec(),
                provider_message_id: Some(target.provider_message_id.to_string()),
                thread_id: Some(target.thread_id.to_string()),
                original_message_id: target.message_id.map(str::to_string),
                original_references: target.references.map(str::to_string),
                subject: subject.to_string(),
                body: body.clone(),
                attachments: attachments.to_vec(),
            });
        Ok(self.send_meta.read().unwrap_or_else(PoisonError::into_inner).clone())
    }

    async fn send_new_email(
        &self,
        from_email: &str,
        from_name: Option<&str>,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<SentMessageMeta> {
        self.send_refusal()?;
        self.sent
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeSentMessage {
                from_email: from_email.to_string(),
                from_name: from_name.map(str::to_string),
                to_emails: to_emails.to_vec(),
                cc_emails: cc_emails.to_vec(),
                provider_message_id: None,
                thread_id: None,
                original_message_id: None,
                original_references: None,
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
        self.record_call("set_read_state");
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

    async fn set_starred(&self, message_id: &str, starred: bool) -> Result<()> {
        self.record_call("set_starred");
        self.mailbox_write_gate()?;
        if let Some(stored) = self
            .messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
            .find(|m| m.email.id == message_id)
        {
            stored.email.is_starred = starred;
        }
        self.mailbox_ops
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeMailboxOp::SetStarred {
                message_id: message_id.to_string(),
                starred,
            });
        Ok(())
    }

    async fn archive_message(&self, message_id: &str, _message_id_header: Option<&str>) -> Result<MessageLocation> {
        self.record_call("archive_message");
        self.mailbox_write_gate()?;
        let location = self
            .archive_location
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| MessageLocation {
                id: message_id.to_string(),
                mailbox: "archive".to_string(),
            });
        if let Some(stored) = self
            .messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
            .find(|m| m.email.id == message_id)
        {
            stored.email.id = location.id.clone();
            stored.email.mailbox = location.mailbox.clone();
        }
        self.mailbox_ops
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeMailboxOp::Archive {
                message_id: message_id.to_string(),
            });
        Ok(location)
    }

    async fn move_to_spam(&self, message_id: &str, _message_id_header: Option<&str>) -> Result<MessageLocation> {
        self.record_call("move_to_spam");
        self.mailbox_write_gate()?;
        let location = MessageLocation {
            id: message_id.to_string(),
            mailbox: "spam".to_string(),
        };
        if let Some(stored) = self
            .messages
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
            .find(|m| m.email.id == message_id)
        {
            stored.email.mailbox = location.mailbox.clone();
        }
        self.mailbox_ops
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeMailboxOp::Spam {
                message_id: message_id.to_string(),
            });
        Ok(location)
    }

    async fn trash_message(&self, message_id: &str, message_id_header: Option<&str>) -> Result<()> {
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
                message_id_header: message_id_header.map(str::to_string),
            });
        Ok(())
    }

    /// Models both provider shapes: the message is found under its own id
    /// (Gmail), or — when it was moved and re-keyed — under the same
    /// Message-ID header at a new id (IMAP/Graph).
    async fn locate_message(
        &self,
        message_id: &str,
        message_id_header: Option<&str>,
    ) -> Result<Option<MessageLocation>> {
        self.record_call("locate_message");
        let guard = self.messages.read().unwrap_or_else(PoisonError::into_inner);
        let found = guard.iter().find(|m| m.email.id == message_id).or_else(|| {
            message_id_header.and_then(|header| guard.iter().find(|m| m.email.message_id.as_deref() == Some(header)))
        });
        Ok(found.map(|m| MessageLocation {
            id: m.email.id.clone(),
            mailbox: m.email.mailbox.clone(),
        }))
    }

    async fn folder_uid_validities(&self) -> Result<Vec<FolderUidValidity>> {
        Ok(self
            .uid_validities
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn list_mailbox_identities(&self, mailbox: &str) -> Result<Vec<MessageIdentity>> {
        self.record_call("list_mailbox_identities");
        if let Some(message) = self
            .identity_listing_failure
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Err(AppError::SyncError(message));
        }
        let guard = self.messages.read().unwrap_or_else(PoisonError::into_inner);
        Ok(guard
            .iter()
            .filter(|m| m.email.mailbox == mailbox)
            .map(|m| MessageIdentity {
                id: m.email.id.clone(),
                message_id: m.email.message_id.clone(),
                timestamp: Some(m.email.timestamp),
            })
            .collect())
    }

    async fn history_cursor(&self) -> Result<Option<String>> {
        let guard = self.history.read().unwrap_or_else(PoisonError::into_inner);
        let Some(history) = guard.as_ref() else {
            return Ok(None);
        };
        self.record_call("history_cursor");
        Ok(Some(history.current.to_string()))
    }

    async fn list_history(&self, cursor: &str, page_token: Option<&str>) -> Result<HistoryListing> {
        let guard = self.history.read().unwrap_or_else(PoisonError::into_inner);
        let Some(history) = guard.as_ref() else {
            return Err(AppError::InvalidInput("this provider has no change log".to_string()));
        };
        self.record_call("list_history");
        if let Some(message) = &history.failure {
            return Err(AppError::SyncError(message.clone()));
        }
        let parse = |value: &str| {
            value
                .parse::<u64>()
                .map_err(|_| AppError::InvalidInput(format!("not a history position: {value}")))
        };
        let start = parse(cursor)?;
        if start < history.oldest_kept {
            return Ok(HistoryListing::CursorExpired);
        }
        let offset = page_token.map(parse).transpose()?.unwrap_or(0) as usize;
        let after: Vec<&(u64, MessageChange)> = history.records.iter().filter(|(id, _)| *id > start).collect();
        let page: Vec<&(u64, MessageChange)> = after.iter().skip(offset).take(history.page_size).copied().collect();
        let end = offset + page.len();
        let more = end < after.len();
        Ok(HistoryListing::Page(HistoryPage {
            changes: page.iter().map(|(_, change)| change.clone()).collect(),
            resume_cursor: match page.last() {
                Some((id, _)) if more => id.to_string(),
                _ => history.current.to_string(),
            },
            next_page_token: more.then(|| end.to_string()),
        }))
    }

    async fn fetch_message_labels(&self, message_ids: &[String]) -> Result<HashMap<String, RemoteLabels>> {
        self.record_call("fetch_message_labels");
        if let Some(message) = self
            .label_fetch_failure
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Err(AppError::SyncError(message));
        }
        let unverifiable = self
            .unverifiable_messages
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let guard = self.messages.read().unwrap_or_else(PoisonError::into_inner);
        Ok(message_ids
            .iter()
            .filter(|id| !unverifiable.contains(*id))
            .map(|id| {
                let labels = match guard.iter().find(|m| &m.email.id == id) {
                    Some(m) => RemoteLabels::Present(fake_labels(&m.email)),
                    None => RemoteLabels::Missing,
                };
                (id.clone(), labels)
            })
            .collect())
    }

    async fn fetch_message_states(
        &self,
        message_ids: &[String],
    ) -> Result<Option<HashMap<String, RemoteMessageState>>> {
        if !self.reports_message_states.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(None);
        }
        self.record_call("fetch_message_states");
        let unverifiable = self
            .unverifiable_messages
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let guard = self.messages.read().unwrap_or_else(PoisonError::into_inner);
        Ok(Some(
            message_ids
                .iter()
                .filter(|id| !unverifiable.contains(*id))
                .map(|id| {
                    let state = match guard.iter().find(|m| &m.email.id == id) {
                        Some(m) => RemoteMessageState::Present {
                            is_read: m.email.is_read,
                            is_starred: Some(m.email.is_starred),
                        },
                        None => RemoteMessageState::Missing,
                    };
                    (id.clone(), state)
                })
                .collect(),
        ))
    }

    async fn create_draft(
        &self,
        _from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        _attachments: &[EmailAttachment],
        reply: Option<&ReplyTarget<'_>>,
    ) -> Result<String> {
        self.draft_write_result()?;
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
            in_reply_to: reply.and_then(|r| r.message_id).map(str::to_string),
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
        reply: Option<&ReplyTarget<'_>>,
    ) -> Result<String> {
        self.draft_write_result()?;
        // A draft sent or deleted from another device is gone: real providers
        // answer 404, which their clients surface as `NotFound`.
        if !self
            .drafts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(provider_draft_id)
        {
            return Err(AppError::NotFound(format!("Fake draft not found: {provider_draft_id}")));
        }
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
            in_reply_to: reply.and_then(|r| r.message_id).map(str::to_string),
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
            references: None,
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
            is_starred: false,
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
            None,
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
        p.send_new_email(
            "me@example.com",
            None,
            &["x@y.com".to_string()],
            &[],
            "subj",
            &body,
            &[],
        )
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
                None,
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
                None,
                &["x@y.com".to_string()],
                &[],
                &ReplyTarget {
                    provider_message_id: "prov-1",
                    thread_id: "thread-1",
                    message_id: Some("<orig@remote>"),
                    references: None,
                },
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
            _from_name: Option<&str>,
            _to_emails: &[String],
            _cc_emails: &[String],
            _target: &ReplyTarget<'_>,
            _subject: &str,
            _body: &EmailBody,
            _attachments: &[EmailAttachment],
        ) -> Result<SentMessageMeta> {
            Ok(SentMessageMeta::default())
        }
        async fn send_new_email(
            &self,
            _from_email: &str,
            _from_name: Option<&str>,
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
        for result in [
            p.set_read_state("m", true).await,
            p.set_starred("m", true).await,
            p.trash_message("m", None).await,
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
    async fn a_provider_without_a_signature_api_has_no_signature_to_import() {
        assert_eq!(BareProvider.get_signature("me@example.com").await.unwrap(), None);
    }

    #[test]
    fn every_shipped_provider_supports_server_side_mailbox_writes() {
        for provider in ["gmail", "imap", "outlook"] {
            assert!(provider_supports_mailbox_writes(provider), "{provider}");
        }
        assert!(
            !provider_supports_mailbox_writes("exchange-ews"),
            "a provider nobody wired must stay local-only"
        );
    }

    #[tokio::test]
    async fn fake_provider_records_read_state_and_trash_calls() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.set_read_state("m-1", true).await.unwrap();
        p.set_read_state("m-2", false).await.unwrap();
        p.trash_message("m-1", Some("<m-1@example.com>")).await.unwrap();

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
                    message_id: "m-1".to_string(),
                    message_id_header: Some("<m-1@example.com>".to_string()),
                },
            ]
        );
    }

    #[tokio::test]
    async fn fake_provider_can_simulate_a_failing_mailbox_write() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.fail_mailbox_writes("mailbox is over quota");

        let err = p.trash_message("m-1", None).await.unwrap_err();
        assert!(err.to_string().contains("over quota"), "unexpected error: {err}");
        assert!(p.mailbox_ops().is_empty(), "a failed write must not be recorded");
    }

    #[tokio::test]
    async fn providers_without_uids_report_no_uid_validity() {
        assert!(BareProvider.folder_uid_validities().await.unwrap().is_empty());
        assert!(BareProvider.list_mailbox_identities("inbox").await.is_err());
        let p = FakeEmailProvider::new("me@example.com", "Me");
        assert!(p.folder_uid_validities().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fake_provider_models_a_uid_validity_change_and_lists_a_mailbox() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.set_folder_uid_validity("inbox", "acc::", 1);
        p.set_folder_uid_validity("inbox", "acc::", 2);
        let mut in_sent = sample_email("acc::SENT::4", 900);
        in_sent.mailbox = "sent".to_string();
        p.add_message(in_sent, EmailCategory::Primary, vec![]);
        p.add_message(sample_email("acc::7", 1_000), EmailCategory::Primary, vec![]);

        assert_eq!(
            p.folder_uid_validities().await.unwrap(),
            vec![FolderUidValidity {
                mailbox: "inbox".to_string(),
                id_prefix: "acc::".to_string(),
                uid_validity: 2,
            }]
        );
        assert_eq!(
            p.list_mailbox_identities("inbox").await.unwrap(),
            vec![MessageIdentity {
                id: "acc::7".to_string(),
                message_id: None,
                timestamp: Some(1_000),
            }]
        );

        p.fail_identity_listing("connection reset");
        assert!(p.list_mailbox_identities("inbox").await.is_err());
    }

    #[tokio::test]
    async fn providers_without_a_state_refresh_answer_none() {
        assert_eq!(
            BareProvider.fetch_message_states(&["m".to_string()]).await.unwrap(),
            None
        );
        // The fake is opt-in, so sync tests that seed local-only rows are safe.
        let p = FakeEmailProvider::new("me@example.com", "Me");
        assert_eq!(p.fetch_message_states(&["m".to_string()]).await.unwrap(), None);
    }

    #[tokio::test]
    async fn fake_provider_reports_flags_vanished_and_unverifiable_messages() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.report_message_states();
        for id in ["read", "unread", "moved", "deleted", "unknown"] {
            let mut email = sample_email(id, 1_000);
            email.message_id = Some(format!("<{id}@example.com>"));
            p.add_message(email, EmailCategory::Primary, vec![]);
        }
        p.set_remote_read("read", true);
        p.relocate_message("moved", "moved-2", "trash");
        p.remove_message("deleted");
        p.make_state_unverifiable("unknown");

        let ids: Vec<String> = ["read", "unread", "moved", "deleted", "unknown"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let states = p.fetch_message_states(&ids).await.unwrap().unwrap();

        assert_eq!(
            states.get("read"),
            Some(&RemoteMessageState::Present {
                is_read: true,
                is_starred: Some(false)
            })
        );
        assert_eq!(
            states.get("unread"),
            Some(&RemoteMessageState::Present {
                is_read: false,
                is_starred: Some(false)
            })
        );
        assert_eq!(states.get("moved"), Some(&RemoteMessageState::Missing));
        assert_eq!(states.get("deleted"), Some(&RemoteMessageState::Missing));
        assert_eq!(states.get("unknown"), None, "unverifiable ids are left out");
        assert_eq!(
            p.locate_message("moved", Some("<moved@example.com>")).await.unwrap(),
            Some(MessageLocation {
                id: "moved-2".to_string(),
                mailbox: "trash".to_string()
            })
        );
    }

    #[tokio::test]
    async fn fake_provider_can_report_a_message_it_no_longer_has() {
        let p = FakeEmailProvider::new("me@example.com", "Me");
        p.fail_mailbox_writes_as_not_found();
        assert!(matches!(
            p.set_read_state("m-1", true).await,
            Err(AppError::NotFound(_))
        ));

        p.restore_mailbox_writes();
        p.set_read_state("m-1", true).await.unwrap();
        assert_eq!(p.mailbox_ops().len(), 1);
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
            assert!(footer.contains("https://getemailops.com/?utm_source=email_footer"));
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
            assert!(footer.contains("href=\"https://getemailops.com/?utm_source=email_footer\""));
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
