use async_trait::async_trait;

use crate::models::error::Result;
use crate::models::Email;

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
}

impl EmailBody {
    /// Plain-text-only body — most existing call sites use this.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            html: None,
            inline_images: Vec::new(),
        }
    }

    /// Text + HTML alternative, no inline images.
    pub fn with_html(text: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            html: Some(html.into()),
            inline_images: Vec::new(),
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

/// Plain-text footer appended to every outgoing email.
///
/// The URL is on its own line — wrapping it in parentheses (e.g. `(https://…)`)
/// makes most email-client auto-linkers swallow the trailing `)` into the URL,
/// producing a broken link like `https://getemailops.com)/`.
pub fn email_footer_plain() -> &'static str {
    "\n\n--\nEnviado con Emailops\nhttps://getemailops.com"
}

/// HTML footer appended to every outgoing email.
pub fn email_footer_html() -> &'static str {
    "<br><br><hr style=\"border:none;border-top:1px solid #eee;margin:16px 0\"><p style=\"color:#888;font-size:12px;margin:0\">Enviado con <a href=\"https://getemailops.com\" style=\"color:#888\">Emailops</a></p>"
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
    /// the HTML via `cid:` URIs.
    async fn send_reply(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        thread_id: &str,
        original_message_id: Option<&str>,
        subject: &str,
        body: &EmailBody,
    ) -> Result<()>;

    /// Send a new email (not a reply to any existing thread).
    ///
    /// `body` carries the plain-text part and, when the user composed in the
    /// rich editor, an HTML alternative. Inline images live inside `body`;
    /// `attachments` is for regular file attachments only.
    async fn send_new_email(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<()>;

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
        }
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
    ) -> Result<()> {
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
                attachments: Vec::new(),
            });
        Ok(())
    }

    async fn send_new_email(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<()> {
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
        Ok(())
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

        let rich = EmailBody::with_html("hello", "<p>hello</p>");
        assert!(rich.has_html());
        assert_eq!(rich.html.as_deref(), Some("<p>hello</p>"));
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
