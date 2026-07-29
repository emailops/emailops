use async_trait::async_trait;
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::time::sleep;

use crate::models::error::{AppError, Result};
use crate::models::{AppLogEvent, Email};
use crate::sync::provider::{self, EmailBody, EmailProvider, MessageRef};

pub use crate::sync::provider::EmailAttachment;

/// Pick the local `mailbox` value for a message based on its Gmail labels.
///
/// The main inbox sync pass intentionally fetches `in:sent` so the user's own
/// replies show up alongside the thread. Without this mapping those rows land
/// with `mailbox = "inbox"` and appear as standalone items in the inbox view.
///
/// Self-sent emails (the user emailing themselves) carry both `INBOX` and
/// `SENT` — they stay in the inbox.
fn mailbox_from_labels(labels: &[String]) -> &'static str {
    let has_sent = labels.iter().any(|l| l == "SENT");
    let has_inbox = labels.iter().any(|l| l == "INBOX");
    if has_sent && !has_inbox {
        "sent"
    } else {
        "inbox"
    }
}

const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1";
const GMAIL_BATCH_URL: &str = "https://www.googleapis.com/batch/gmail/v1";
const GMAIL_MAX_RETRIES: u32 = 5;
const GMAIL_INITIAL_BACKOFF_MS: u64 = 1_000;
const GMAIL_MAX_BACKOFF_MS: u64 = 30_000;

#[derive(Debug, Deserialize)]
struct GmailProfile {
    #[serde(rename = "emailAddress")]
    email_address: String,
    #[serde(rename = "messagesTotal")]
    messages_total: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GmailMessageList {
    messages: Option<Vec<GmailMessageRef>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GmailMessageRef {
    pub id: String,
    #[serde(rename = "threadId")]
    pub thread_id: String,
}

#[derive(Debug, Deserialize)]
struct GmailMessage {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: String,
    #[serde(rename = "labelIds")]
    label_ids: Option<Vec<String>>,
    snippet: String,
    payload: GmailPayload,
    #[serde(rename = "internalDate")]
    internal_date: String,
}

/// Draft resource where we only need the draft `id`. Used for `drafts.create`,
/// `drafts.update`, and `drafts.list` entries — all of which return the nested
/// `message` in MINIMAL format (no `payload`). Deserializing that partial
/// message into the full [`GmailMessage`] fails, so those paths must decode the
/// id plus the nested message's id. Read with `format=full` — see
/// [`GmailDraft`] — when the message body is actually needed.
#[derive(Debug, Deserialize)]
struct GmailDraftId {
    id: String,
    /// MINIMAL-format message stub. Its `id` is the draft's change token:
    /// Gmail replaces the underlying message on every draft save, so an
    /// unchanged id proves the content is untouched and the `drafts.get` can
    /// be skipped. Absent on the create/update responses, which don't need it.
    #[serde(default)]
    message: Option<GmailDraftMessageRef>,
}

/// Just enough of a MINIMAL-format draft message to read its id.
#[derive(Debug, Deserialize)]
struct GmailDraftMessageRef {
    id: String,
}

/// Full draft resource, decoded only from a `format=full` fetch where the
/// nested `message` carries a complete payload.
#[derive(Debug, Deserialize)]
struct GmailDraft {
    #[allow(dead_code)]
    id: String,
    message: Option<GmailMessage>,
}

#[derive(Debug, Deserialize)]
struct GmailDraftList {
    drafts: Option<Vec<GmailDraftId>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailPayload {
    headers: Vec<GmailHeader>,
    body: Option<GmailBody>,
    parts: Option<Vec<GmailPart>>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailHeader {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct GmailBody {
    data: Option<String>,
    #[allow(dead_code)]
    size: i64,
    #[serde(rename = "attachmentId")]
    attachment_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailPart {
    #[serde(rename = "mimeType")]
    mime_type: String,
    filename: Option<String>,
    headers: Option<Vec<GmailHeader>>,
    body: Option<GmailBody>,
    parts: Option<Vec<GmailPart>>,
}

#[derive(Debug, Deserialize)]
struct GmailApiErrorEnvelope {
    error: Option<GmailApiError>,
}

#[derive(Debug, Deserialize)]
struct GmailApiError {
    message: Option<String>,
    errors: Option<Vec<GmailApiErrorDetail>>,
}

#[derive(Debug, Deserialize)]
struct GmailApiErrorDetail {
    reason: Option<String>,
}

pub struct GmailClient {
    client: Client,
    /// Interior-mutable so it can be updated on a transparent 401-refresh without `&mut self`.
    access_token: std::sync::Mutex<String>,
    /// Stored so we can transparently refresh on 401 mid-sync.
    refresh_token: Option<String>,
    app: Option<AppHandle>,
    account_id: Option<String>,
    /// Base URL for the Gmail API. Defaults to [`GMAIL_API_BASE`]; override
    /// via [`GmailClient::with_base_url`] in tests so the client can be
    /// pointed at a `MockProviderServer` (see `sync::mock`).
    base_url: String,
}

/// Metadata about a file attachment in an email (not inline body parts).
#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    /// Gmail attachment ID for large attachments, empty for small inline ones.
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    /// Base64 data for small inline attachments (when attachment_id is empty).
    pub inline_data: Option<String>,
}

/// Email category parsed from Gmail labels
#[derive(Debug, Clone, PartialEq)]
pub enum EmailCategory {
    Primary,
    Social,
    Promotions,
    Updates,
    Forums,
}

impl EmailCategory {
    pub fn from_labels(labels: &[String]) -> Self {
        for label in labels {
            match label.as_str() {
                "CATEGORY_SOCIAL" => return Self::Social,
                "CATEGORY_PROMOTIONS" => return Self::Promotions,
                "CATEGORY_UPDATES" => return Self::Updates,
                "CATEGORY_FORUMS" => return Self::Forums,
                "CATEGORY_PERSONAL" => return Self::Primary,
                _ => {}
            }
        }
        Self::Primary
    }

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

impl GmailClient {
    pub fn new(
        access_token: String,
        refresh_token: Option<String>,
        app: Option<AppHandle>,
        account_id: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            access_token: std::sync::Mutex::new(access_token),
            refresh_token,
            app,
            account_id,
            base_url: GMAIL_API_BASE.to_string(),
        }
    }

    /// Override the Gmail API base URL. Production code never calls this —
    /// the `MockProviderServer` test harness uses it to redirect HTTP traffic
    /// at a `wiremock` instance loaded from a recorded cassette.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Refresh the access token using the stored refresh token and update it in place.
    /// Called transparently on 401 — no user-visible log is emitted.
    async fn refresh_access_token(&self) -> Result<()> {
        let Some(refresh_token) = &self.refresh_token else {
            return Err(AppError::AuthError(
                "Gmail session expired and no refresh token is stored. Please re-authenticate.".to_string(),
            ));
        };
        let Some(account_id) = &self.account_id else {
            return Err(AppError::AuthError(
                "Gmail token refresh failed: account ID unknown.".to_string(),
            ));
        };
        let config = crate::sync::oauth::OAuthConfig::for_provider("gmail");
        let new_tokens = crate::sync::oauth::refresh_oauth_token(&config, refresh_token).await?;
        crate::services::accounts::store_tokens(account_id, &new_tokens)?;
        // Recover from a poisoned mutex — the protected value is a single
        // String, so a previous panic can't have left it in an invalid state.
        *self
            .access_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_tokens.access_token;
        Ok(())
    }

    pub async fn get_profile(&self) -> Result<(String, String)> {
        let url = format!("{}/users/me/profile", self.base_url);
        let response = self.send_get_with_retry(&url, "get profile").await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to get profile: {}", error_text)));
        }

        let profile: GmailProfile = response.json().await?;
        Ok((profile.email_address.clone(), profile.email_address))
    }

    /// Returns Gmail's `messagesTotal` from the profile endpoint — the number
    /// of messages in the entire mailbox (all labels, including Spam/Trash).
    /// Used by the dashboard to show "synced X / Y on server".
    pub async fn get_messages_total(&self) -> Result<Option<i64>> {
        let url = format!("{}/users/me/profile", self.base_url);
        let response = self.send_get_with_retry(&url, "get profile (messages total)").await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to get profile: {}", error_text)));
        }

        let profile: GmailProfile = response.json().await?;
        Ok(profile.messages_total)
    }

    pub async fn list_messages(
        &self,
        max_results: u32,
        page_token: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        label_filter: Option<&str>,
    ) -> Result<(Vec<GmailMessageRef>, Option<String>)> {
        self.list_messages_scoped(
            max_results,
            page_token,
            after_timestamp,
            before_timestamp,
            label_filter,
            /* include_spam_trash */ false,
            /* include_all_mail */ false,
        )
        .await
    }

    /// Lower-level variant that lets the caller decide whether to keep Gmail's
    /// default `-in:spam -in:trash` exclusion (Inbox sync) or drop it (Spam /
    /// Trash mailbox sync). `include_all_mail=true` adds the `in:anywhere`
    /// operator, which Gmail requires to include spam & trash in results.
    async fn list_messages_scoped(
        &self,
        max_results: u32,
        page_token: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        label_filter: Option<&str>,
        include_spam_trash: bool,
        include_all_mail: bool,
    ) -> Result<(Vec<GmailMessageRef>, Option<String>)> {
        let mut url = format!("{}/users/me/messages?maxResults={}", self.base_url, max_results);
        if include_all_mail {
            // Required for `in:trash` / `in:spam` to return results.
            url.push_str("&includeSpamTrash=true");
        }

        if let Some(token) = page_token {
            url.push_str(&format!("&pageToken={}", token));
        }

        let mut query_parts = Vec::new();
        // Gmail search treats `after:0` (and other non-positive epoch values) as
        // invalid and silently returns zero messages — it does NOT mean
        // "everything since 1970". The caller passes Some(0) as a sentinel for
        // "unbounded / All mail" on the first sync of a new account, so skip
        // the operator entirely in that case and let the rest of the query
        // (label filter, spam/trash exclusion) define the result set.
        if let Some(ts) = after_timestamp.filter(|t| *t > 0) {
            query_parts.push(format!("after:{}", ts));
        }
        if let Some(ts) = before_timestamp.filter(|t| *t > 0) {
            query_parts.push(format!("before:{}", ts));
        }
        if let Some(filter) = label_filter {
            query_parts.push(filter.to_string());
        }
        if !include_spam_trash {
            query_parts.push("-in:spam -in:trash".to_string());
        }
        if !query_parts.is_empty() {
            url.push_str(&format!("&q={}", query_parts.join("%20")));
        }

        let response = self.send_get_with_retry(&url, "list messages").await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to list messages: {}", error_text)));
        }

        let list: GmailMessageList = response.json().await?;
        Ok((list.messages.unwrap_or_default(), list.next_page_token))
    }

    pub async fn get_message(&self, message_id: &str) -> Result<(Email, EmailCategory, Vec<AttachmentInfo>)> {
        let url = format!("{}/users/me/messages/{}?format=full", self.base_url, message_id);

        let response = self.send_get_with_retry(&url, "get message").await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to get message: {}", error_text)));
        }

        let msg: GmailMessage = response.json().await?;
        self.parse_message(msg).await
    }

    pub async fn send_reply(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        thread_id: &str,
        original_message_id: Option<&str>,
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<crate::sync::provider::SentMessageMeta> {
        let normalized_subject = reply_subject(subject);
        let message = crate::sync::mime_builder::build_lettre_message(&crate::sync::mime_builder::SendMimeParams {
            from_email,
            to_emails,
            cc_emails,
            subject: &normalized_subject,
            in_reply_to: original_message_id.filter(|v| !v.trim().is_empty()),
            body,
            attachments,
        })?;
        let message_id_header = crate::sync::mime_builder::extract_message_id(&message);
        let raw = base64_url_encode(&message.formatted());
        let payload = serde_json::json!({
            "threadId": thread_id,
            "raw": raw,
        });
        let url = format!("{}/users/me/messages/send", self.base_url);

        let response = self.send_post_json_with_retry(&url, &payload, "send reply").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to send reply: {}", error_text)));
        }

        Ok(sent_meta_from_response(response.json().await.ok(), message_id_header))
    }

    pub async fn send_new_email(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<crate::sync::provider::SentMessageMeta> {
        let message = crate::sync::mime_builder::build_lettre_message(&crate::sync::mime_builder::SendMimeParams {
            from_email,
            to_emails,
            cc_emails,
            subject,
            in_reply_to: None,
            body,
            attachments,
        })?;
        let message_id_header = crate::sync::mime_builder::extract_message_id(&message);
        let raw = base64_url_encode(&message.formatted());
        let payload = serde_json::json!({ "raw": raw });
        let url = format!("{}/users/me/messages/send", self.base_url);

        let response = self.send_post_json_with_retry(&url, &payload, "send new email").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to send email: {}", error_text)));
        }

        Ok(sent_meta_from_response(response.json().await.ok(), message_id_header))
    }

    /// Build the base64url-encoded MIME + `{message:{raw}}` payload shared by
    /// draft create and update.
    fn draft_payload(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<serde_json::Value> {
        let mime = crate::sync::mime_builder::build_send_mime(&crate::sync::mime_builder::SendMimeParams {
            from_email,
            to_emails,
            cc_emails,
            subject,
            in_reply_to: None,
            body,
            attachments,
        })?;
        let raw = base64_url_encode(mime.as_bytes());
        Ok(serde_json::json!({ "message": { "raw": raw } }))
    }

    async fn create_draft(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<String> {
        let payload = self.draft_payload(from_email, to_emails, cc_emails, subject, body, attachments)?;
        let url = format!("{}/users/me/drafts", self.base_url);
        let response = self.send_post_json_with_retry(&url, &payload, "create draft").await?;
        let draft: GmailDraftId = response.json().await?;
        Ok(draft.id)
    }

    async fn update_draft(
        &self,
        provider_draft_id: &str,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<String> {
        let payload = self.draft_payload(from_email, to_emails, cc_emails, subject, body, attachments)?;
        let url = format!("{}/users/me/drafts/{}", self.base_url, provider_draft_id);
        let response = self
            .send_request_with_retry("update draft", |client, token| {
                client.put(&url).bearer_auth(token).json(&payload)
            })
            .await?;
        let draft: GmailDraftId = response.json().await?;
        Ok(draft.id)
    }

    async fn delete_draft(&self, provider_draft_id: &str) -> Result<()> {
        let url = format!("{}/users/me/drafts/{}", self.base_url, provider_draft_id);
        self.send_request_with_retry("delete draft", |client, token| client.delete(&url).bearer_auth(token))
            .await?;
        Ok(())
    }

    async fn list_drafts(
        &self,
        known_change_tokens: &std::collections::HashMap<String, String>,
    ) -> Result<crate::sync::draft_plan::ProviderDraftPull> {
        // Enumerate the whole Drafts folder. This must be exhaustive: the ids
        // become the keep-list for `prune_provider_drafts`, so stopping at page
        // one would delete every draft past the first 100 on each sync.
        let mut listed = Vec::new();
        let mut page_token: Option<String> = None;
        for page in 0..crate::sync::draft_plan::MAX_DRAFT_PAGES {
            let mut url = format!("{}/users/me/drafts?maxResults=100", self.base_url);
            if let Some(token) = &page_token {
                url.push_str(&format!("&pageToken={}", urlencoding::encode(token)));
            }
            let response = self.send_get_with_retry(&url, "list drafts").await?;
            let list: GmailDraftList = response.json().await?;
            listed.extend(list.drafts.unwrap_or_default().into_iter().map(|entry| {
                crate::sync::draft_plan::ListedDraft {
                    provider_draft_id: entry.id,
                    change_token: entry.message.map(|m| m.id),
                }
            }));
            match list.next_page_token {
                Some(token) if !token.is_empty() => page_token = Some(token),
                _ => break,
            }
            // Bail rather than truncate: a partial list would make the prune
            // pass delete every draft it never got to see.
            if page + 1 == crate::sync::draft_plan::MAX_DRAFT_PAGES {
                return Err(AppError::SyncError(format!(
                    "Gmail drafts listing returned too many pages (over {}); refusing a partial list.",
                    crate::sync::draft_plan::MAX_DRAFT_PAGES
                )));
            }
        }

        // Only drafts whose message id moved need the (expensive) full read.
        let plan = crate::sync::draft_plan::plan_draft_fetches(&listed, known_change_tokens);
        let mut changed = Vec::with_capacity(plan.to_fetch.len());
        for draft_id in &plan.to_fetch {
            let full_url = format!("{}/users/me/drafts/{}?format=full", self.base_url, draft_id);
            let full_resp = self.send_get_with_retry(&full_url, "get draft").await?;
            let full: GmailDraft = full_resp.json().await?;
            let Some(msg) = full.message else { continue };
            // Take the token from the content we actually read, not from the
            // listing: if the draft was saved in between, storing the listed id
            // would pin us to content we never fetched.
            let message_id = Some(msg.id.clone());
            let (email, _cat, _atts) = self.parse_message(msg).await?;
            // The parsed body is HTML; split it so the composer renders the rich
            // source instead of escaping it as literal text.
            let (body, body_html) = crate::util::html::split_draft_body(&email.body);
            changed.push(crate::models::ProviderDraft {
                provider_draft_id: draft_id.clone(),
                to_addresses: email.recipients,
                cc_addresses: email.cc,
                subject: email.subject,
                body,
                body_html,
                // Gmail's `internalDate`, already parsed onto the message —
                // for a draft that's when it was last saved.
                updated_at: Some(email.timestamp),
                provider_message_id: message_id,
            });
        }
        Ok(crate::sync::draft_plan::ProviderDraftPull {
            changed,
            present_ids: plan.present_ids,
        })
    }

    async fn parse_message(&self, msg: GmailMessage) -> Result<(Email, EmailCategory, Vec<AttachmentInfo>)> {
        let headers = &msg.payload.headers;

        // Preserve message order: `capture` depends on it for both the topmost
        // Authentication-Results and the bottom-most Received.
        let header_pairs: Vec<(String, String)> = headers.iter().map(|h| (h.name.clone(), h.value.clone())).collect();

        let subject = headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("Subject"))
            .map(|h| h.value.clone())
            .unwrap_or_default();

        let message_id = headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("Message-ID"))
            .map(|h| h.value.clone());

        let from = headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("From"))
            .map(|h| h.value.clone())
            .unwrap_or_default();

        let to = headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("To"))
            .map(|h| h.value.clone())
            .unwrap_or_default();

        // Parse sender name and email
        let (sender_name, sender_email) = parse_email_address(&from);

        // Parse recipients (To and Cc)
        let recipients: Vec<String> = to
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let cc_header = headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("Cc"))
            .map(|h| h.value.clone())
            .unwrap_or_default();
        let cc: Vec<String> = cc_header
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Get body content
        let body = self.extract_body(&msg.id, &msg.payload).await;

        // Collect file attachment metadata
        let attachment_infos = Self::collect_attachment_infos(&msg.payload);

        // Get labels and determine category
        let labels = msg.label_ids.clone().unwrap_or_default();
        let category = EmailCategory::from_labels(&labels);

        // Check if read
        let is_read = !labels.contains(&"UNREAD".to_string());

        // The main inbox sync pass fetches `in:sent` so threads show the user's
        // replies, but those rows were getting `mailbox = "inbox"` and leaking
        // into the standalone inbox view. Use Gmail's authoritative labels.
        let mailbox = mailbox_from_labels(&labels);

        // Parse timestamp
        let timestamp = msg
            .internal_date
            .parse::<i64>()
            .unwrap_or_else(|_| chrono::Utc::now().timestamp_millis());

        let email = Email {
            id: msg.id,
            account_id: String::new(), // Will be set by caller
            thread_id: msg.thread_id,
            message_id,
            subject,
            sender: sender_name,
            sender_email,
            recipients,
            cc,
            body,
            snippet: msg.snippet,
            timestamp: timestamp / 1000, // Convert from millis to seconds
            is_read,
            triage_status: None,
            category: category.as_str().to_string(),
            // Derived from Gmail labels above. The extra-mailbox sync passes
            // (e.g. sync_extra_mailboxes for Sent / Trash) still override this
            // explicitly when they intentionally route a row into a folder.
            mailbox: mailbox.to_string(),
            // Independent of `mailbox`: a message the user sent to themselves
            // carries INBOX *and* SENT, and `mailbox` records 'inbox' to keep
            // the thread in the inbox view. Only this flag can tell the Sent
            // view about it — the sender is no help when the message went out
            // through a send-as alias.
            is_sent: labels.iter().any(|l| l == "SENT"),
            // `?format=full` already returns the complete header list; before
            // this we read five of them and discarded the rest. No extra
            // network cost.
            headers: Some(crate::sync::header_capture::capture(&header_pairs)),
        };

        Ok((email, category, attachment_infos))
    }

    /// Collect metadata about file attachments (not inline body parts) in this message.
    fn collect_attachment_infos(payload: &GmailPayload) -> Vec<AttachmentInfo> {
        let mut infos = Vec::new();
        if let Some(ref parts) = payload.parts {
            Self::collect_attachment_infos_recursive(parts, &mut infos);
        }
        infos
    }

    fn collect_attachment_infos_recursive(parts: &[GmailPart], infos: &mut Vec<AttachmentInfo>) {
        for part in parts {
            let is_multipart = part.mime_type.starts_with("multipart/");
            let is_body_type = matches!(part.mime_type.as_str(), "text/html" | "text/plain");

            if !is_multipart {
                // Detect attachment via two signals:
                // 1. Gmail's `filename` field (primary — most reliable)
                let api_filename = part.filename.as_ref().filter(|f| !f.is_empty());

                // 2. Content-Disposition header with filename (fallback)
                let header_filename = if api_filename.is_none() {
                    part.headers.as_ref().and_then(|headers| {
                        headers.iter().find_map(|h| {
                            if h.name.eq_ignore_ascii_case("Content-Disposition") {
                                extract_filename_from_disposition(&h.value)
                            } else if h.name.eq_ignore_ascii_case("Content-Type") {
                                extract_name_from_content_type(&h.value)
                            } else {
                                None
                            }
                        })
                    })
                } else {
                    None
                };

                let filename = api_filename.cloned().or(header_filename);

                // For parts without any filename: only treat as attachment if
                // they have an attachment_id and aren't body content types
                let is_attachment = match &filename {
                    Some(_) => true,
                    None => !is_body_type && part.body.as_ref().is_some_and(|b| b.attachment_id.is_some()),
                };

                if is_attachment {
                    if let Some(ref body) = part.body {
                        let has_content = body.attachment_id.is_some() || body.data.is_some();
                        if has_content {
                            // Inline images frequently arrive with no filename.
                            // Without disambiguation every one of them falls
                            // back to `attachment.png` and the unique index
                            // `(email_id, filename)` collapses the 2nd, 3rd…
                            // into the 1st row. When a Content-Id is present
                            // we suffix the fallback filename with it so each
                            // inline image stores its own row.
                            let content_id = part.headers.as_ref().and_then(|headers| {
                                headers
                                    .iter()
                                    .find(|h| h.name.eq_ignore_ascii_case("Content-Id"))
                                    .map(|h| h.value.trim_matches(|c| c == '<' || c == '>').to_string())
                                    .filter(|s| !s.is_empty())
                            });

                            let resolved_filename = filename.unwrap_or_else(|| {
                                let ext = mime_to_extension(&part.mime_type);
                                match &content_id {
                                    Some(cid) => format!("attachment-{}.{}", sanitize_filename_fragment(cid), ext),
                                    None => format!("attachment.{}", ext),
                                }
                            });

                            infos.push(AttachmentInfo {
                                attachment_id: body.attachment_id.clone().unwrap_or_default(),
                                filename: resolved_filename,
                                mime_type: part.mime_type.clone(),
                                size: body.size,
                                inline_data: if body.attachment_id.is_none() {
                                    body.data.clone()
                                } else {
                                    None
                                },
                            });
                        }
                    }
                }
            }

            // Recurse into nested parts
            if let Some(ref nested) = part.parts {
                Self::collect_attachment_infos_recursive(nested, infos);
            }
        }
    }

    /// Fetch attachment binary data from Gmail API.
    pub async fn fetch_attachment_bytes(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/users/me/messages/{}/attachments/{}",
            self.base_url, message_id, attachment_id
        );
        let response = self.send_get_with_retry(&url, "get attachment bytes").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to get attachment: {}", error_text)));
        }

        #[derive(Deserialize)]
        struct AttachmentResponse {
            data: String,
        }

        let att: AttachmentResponse = response.json().await?;
        decode_base64_url_bytes(&att.data)
    }

    async fn extract_body(&self, message_id: &str, payload: &GmailPayload) -> String {
        // Try to get HTML body first, fall back to plain text (inline data)
        let mut html = if let Some(body) = self.find_body_part(payload, "text/html") {
            body
        } else if let Some(body) = self.find_body_part(payload, "text/plain") {
            plain_text_to_html(&body)
        } else if let Some(ref body) = payload.body {
            if let Some(ref data) = body.data {
                base64_url_decode(data).unwrap_or_default()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // If no inline data, try fetching body via attachment ID
        if html.is_empty() {
            if let Some(att_id) = Self::find_body_attachment_id(payload, "text/html") {
                if let Ok(data) = self.fetch_attachment(message_id, &att_id).await {
                    html = data;
                }
            } else if let Some(att_id) = Self::find_body_attachment_id(payload, "text/plain") {
                if let Ok(data) = self.fetch_attachment(message_id, &att_id).await {
                    html = plain_text_to_html(&data);
                }
            }
        }

        // Replace cid: references with inline data URIs. Gmail returns inline
        // images one of two ways: small parts arrive with `body.data` populated
        // (URL-safe base64), large parts arrive with only `body.attachmentId`
        // and require a separate fetch via `/attachments/{id}`. Earlier the
        // attachment-id branch was silently dropped — rendered emails showed
        // broken `image.png` placeholders for every cid: reference.
        if html.contains("cid:") {
            let refs = collect_inline_image_refs(payload);
            for r in refs {
                let std_b64 = match r.inline_data {
                    Some(data) => {
                        // URL-safe → standard alphabet; data URIs use standard.
                        data.replace('-', "+").replace('_', "/")
                    }
                    None => match r.attachment_id {
                        Some(att_id) => match self.fetch_attachment_bytes(message_id, &att_id).await {
                            Ok(bytes) => {
                                use base64::{engine::general_purpose::STANDARD, Engine};
                                STANDARD.encode(&bytes)
                            }
                            Err(_) => continue,
                        },
                        None => continue,
                    },
                };
                let data_uri = format!("data:{};base64,{}", r.mime_type, std_b64);
                html = html.replace(&format!("cid:{}", r.content_id), &data_uri);
            }
        }

        html
    }
}

/// A reference to an inline image referenced by a `cid:` URL in the HTML body.
///
/// Gmail returns inline images two ways: small parts ship the base64 payload
/// inline (`inline_data`), large parts ship only an `attachment_id` that the
/// caller must fetch via `/users/me/messages/{id}/attachments/{id}`.
#[derive(Debug, Clone, PartialEq)]
struct InlineImageRef {
    content_id: String,
    mime_type: String,
    inline_data: Option<String>,
    attachment_id: Option<String>,
}

/// Walk a Gmail payload and collect every `image/*` part that has a
/// `Content-Id` header. Pure — no I/O — so it can be unit-tested without a
/// network or a `GmailClient`.
fn collect_inline_image_refs(payload: &GmailPayload) -> Vec<InlineImageRef> {
    let mut out = Vec::new();
    if let Some(ref parts) = payload.parts {
        collect_inline_image_refs_recursive(parts, &mut out);
    }
    out
}

fn collect_inline_image_refs_recursive(parts: &[GmailPart], out: &mut Vec<InlineImageRef>) {
    for part in parts {
        if part.mime_type.starts_with("image/") {
            let content_id = part.headers.as_ref().and_then(|headers| {
                headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case("Content-Id"))
                    .map(|h| h.value.trim_matches(|c| c == '<' || c == '>').to_string())
            });

            if let (Some(cid), Some(ref body)) = (content_id, &part.body) {
                if body.data.is_some() || body.attachment_id.is_some() {
                    out.push(InlineImageRef {
                        content_id: cid,
                        mime_type: part.mime_type.clone(),
                        inline_data: body.data.clone(),
                        attachment_id: body.attachment_id.clone(),
                    });
                }
            }
        }

        if let Some(ref nested) = part.parts {
            collect_inline_image_refs_recursive(nested, out);
        }
    }
}

impl GmailClient {
    fn find_body_part(&self, payload: &GmailPayload, mime_type: &str) -> Option<String> {
        let mut log = Vec::new();

        // Check direct body
        if payload.mime_type.as_deref() == Some(mime_type) {
            if let Some(ref body) = payload.body {
                if let Some(ref data) = body.data {
                    if let Ok(decoded) = base64_url_decode(data) {
                        return Some(decoded);
                    }
                }
            }
        }

        // Recurse into parts at arbitrary depth
        if let Some(ref parts) = payload.parts {
            for part in parts {
                if let Some(decoded) = Self::find_body_part_recursive(part, mime_type, &mut log) {
                    return Some(decoded);
                }
            }
        }

        None
    }

    fn find_body_part_recursive(part: &GmailPart, mime_type: &str, log: &mut Vec<String>) -> Option<String> {
        if part.mime_type == mime_type {
            match &part.body {
                Some(body) => match (&body.data, &body.attachment_id) {
                    (Some(data), _) => match base64_url_decode(data) {
                        Ok(decoded) => return Some(decoded),
                        Err(e) => log.push(format!(
                            "{} decode error: {}, data_len={}, first_bytes={:?}",
                            mime_type,
                            e,
                            data.len(),
                            &data[..data.len().min(40)]
                        )),
                    },
                    (None, Some(att_id)) => {
                        log.push(format!("{} matched but data=None, attachmentId={}", mime_type, att_id));
                    }
                    (None, None) => {
                        log.push(format!(
                            "{} matched but no data, no attachmentId (size={})",
                            mime_type, body.size
                        ));
                    }
                },
                None => {
                    log.push(format!("{} matched but body is None", mime_type));
                }
            }
        }

        if let Some(ref nested) = part.parts {
            for nested_part in nested {
                if let Some(decoded) = Self::find_body_part_recursive(nested_part, mime_type, log) {
                    return Some(decoded);
                }
            }
        }

        None
    }

    fn find_body_attachment_id(payload: &GmailPayload, mime_type: &str) -> Option<String> {
        if let Some(ref parts) = payload.parts {
            for part in parts {
                if let Some(id) = Self::find_attachment_id_recursive(part, mime_type) {
                    return Some(id);
                }
            }
        }
        None
    }

    fn find_attachment_id_recursive(part: &GmailPart, mime_type: &str) -> Option<String> {
        if part.mime_type == mime_type {
            if let Some(ref body) = part.body {
                if let Some(ref att_id) = body.attachment_id {
                    return Some(att_id.clone());
                }
            }
        }
        if let Some(ref nested) = part.parts {
            for nested_part in nested {
                if let Some(id) = Self::find_attachment_id_recursive(nested_part, mime_type) {
                    return Some(id);
                }
            }
        }
        None
    }

    async fn fetch_attachment(&self, message_id: &str, attachment_id: &str) -> Result<String> {
        let url = format!(
            "{}/users/me/messages/{}/attachments/{}",
            self.base_url, message_id, attachment_id
        );
        let response = self.send_get_with_retry(&url, "get attachment").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to get attachment: {}", error_text)));
        }

        #[derive(Deserialize)]
        struct AttachmentResponse {
            data: String,
        }

        let att: AttachmentResponse = response.json().await?;
        base64_url_decode(&att.data)
    }

    async fn send_get_with_retry(&self, url: &str, operation: &str) -> Result<Response> {
        self.send_request_with_retry(operation, |client, token| client.get(url).bearer_auth(token))
            .await
    }

    async fn send_post_json_with_retry(
        &self,
        url: &str,
        payload: &serde_json::Value,
        operation: &str,
    ) -> Result<Response> {
        self.send_request_with_retry(operation, |client, token| {
            client.post(url).bearer_auth(token).json(payload)
        })
        .await
    }

    async fn send_request_with_retry<F>(&self, operation: &str, request_builder: F) -> Result<Response>
    where
        F: Fn(&Client, &str) -> reqwest::RequestBuilder,
    {
        // A previous 429 told us when the quota window reopens: skip the call
        // entirely instead of burning more quota on requests that cannot succeed.
        if let Some(account_id) = &self.account_id {
            if let Some(until) = rate_limit_gate_until(account_id, crate::services::clock::now_secs()) {
                return Err(AppError::SyncError(format!(
                    "Gmail rate limit in effect — skipping {}; requests for this account are paused until {}",
                    operation,
                    format_gate_until(until)
                )));
            }
        }

        let mut delay_ms = GMAIL_INITIAL_BACKOFF_MS;
        // 401 gets one silent refresh attempt; this flag ensures we don't loop on auth failures.
        let mut auth_retried = false;

        for attempt in 0..=GMAIL_MAX_RETRIES {
            let token = self
                .access_token
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let response = request_builder(&self.client, &token).send().await;

            match response {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();

                    // Transparently refresh an expired token and retry — no error log emitted.
                    if status == StatusCode::UNAUTHORIZED && !auth_retried {
                        auth_retried = true;
                        match self.refresh_access_token().await {
                            Ok(()) => continue,      // retry immediately with new token
                            Err(e) => return Err(e), // refresh failed → propagate auth error
                        }
                    }

                    let headers = response.headers().clone();
                    let body = response.text().await.unwrap_or_default();
                    let should_retry = is_retryable_gmail_error(status, &body);

                    if should_retry && attempt < GMAIL_MAX_RETRIES {
                        match plan_rate_limit_wait(&headers, &body, delay_ms, crate::services::clock::now_secs()) {
                            RetryPlan::Wait(wait_ms) => {
                                self.emit_retry_log(operation, attempt + 1, wait_ms, status);
                                sleep(Duration::from_millis(wait_ms)).await;
                                delay_ms = (delay_ms * 2).min(GMAIL_MAX_BACKOFF_MS);
                                continue;
                            }
                            RetryPlan::GateUntil(until_secs) => {
                                if let Some(account_id) = &self.account_id {
                                    set_rate_limit_gate(account_id, until_secs);
                                }
                                self.emit_rate_limit_gate_log(operation, status, until_secs);
                            }
                        }
                    }

                    return Err(AppError::SyncError(format!(
                        "Failed to {}: {}",
                        operation,
                        format_gmail_error(status, &body)
                    )));
                }
                Err(error) => {
                    if is_retryable_transport_error(&error) && attempt < GMAIL_MAX_RETRIES {
                        self.emit_transport_retry_log(operation, attempt + 1, delay_ms, &error);
                        sleep(Duration::from_millis(delay_ms.min(GMAIL_MAX_BACKOFF_MS))).await;
                        delay_ms = (delay_ms * 2).min(GMAIL_MAX_BACKOFF_MS);
                        continue;
                    }
                    return Err(error.into());
                }
            }
        }

        Err(AppError::SyncError(format!(
            "Failed to {} after retries exhausted",
            operation
        )))
    }

    fn emit_retry_log(&self, operation: &str, attempt: u32, wait_ms: u64, status: StatusCode) {
        let Some(app) = &self.app else { return };
        let account = self.account_id.as_deref().unwrap_or("unknown account");
        let seconds = wait_ms.div_ceil(1000);
        let _ = app.emit(
            "app-log",
            AppLogEvent {
                level: "warn".to_string(),
                source: "sync".to_string(),
                message: format!(
                    "Gmail rate limited {} for {} (HTTP {}). Retrying in {}s (attempt {}/{})...",
                    operation,
                    account,
                    status.as_u16(),
                    seconds,
                    attempt + 1,
                    GMAIL_MAX_RETRIES + 1
                ),
            },
        );
    }

    fn emit_rate_limit_gate_log(&self, operation: &str, status: StatusCode, until_secs: i64) {
        let Some(app) = &self.app else { return };
        let account = self.account_id.as_deref().unwrap_or("unknown account");
        let _ = app.emit(
            "app-log",
            AppLogEvent {
                level: "warn".to_string(),
                source: "sync".to_string(),
                message: format!(
                    "Gmail rate limited {} for {} (HTTP {}). Provider asked to retry after {} — pausing Gmail requests for this account until then.",
                    operation,
                    account,
                    status.as_u16(),
                    format_gate_until(until_secs)
                ),
            },
        );
    }

    fn emit_transport_retry_log(&self, operation: &str, attempt: u32, wait_ms: u64, error: &reqwest::Error) {
        let Some(app) = &self.app else { return };
        let account = self.account_id.as_deref().unwrap_or("unknown account");
        let seconds = wait_ms.div_ceil(1000);
        let _ = app.emit(
            "app-log",
            AppLogEvent {
                level: "warn".to_string(),
                source: "sync".to_string(),
                message: format!(
                    "Transient Gmail error during {} for {}: {}. Retrying in {}s (attempt {}/{})...",
                    operation,
                    account,
                    error,
                    seconds,
                    attempt + 1,
                    GMAIL_MAX_RETRIES + 1
                ),
            },
        );
    }

    /// Build and send a batch HTTP request for the given message IDs, then parse all
    /// sub-responses. Returns `Ok(json)` for 2xx sub-responses or `Err(http_status)` for
    /// non-2xx ones. A transport/auth failure propagates as an outer `Err`.
    async fn send_batch_and_parse(
        &self,
        message_ids: &[&str],
        boundary: &str,
        operation: &str,
    ) -> Result<Vec<std::result::Result<String, u16>>> {
        // Build multipart/mixed request body.
        let mut body = String::new();
        for (i, &id) in message_ids.iter().enumerate() {
            body.push_str(&format!("--{}\r\n", boundary));
            body.push_str("Content-Type: application/http\r\n");
            body.push_str(&format!("Content-ID: <item{}>\r\n", i));
            body.push_str("\r\n");
            body.push_str(&format!(
                "GET /gmail/v1/users/me/messages/{}?format=full HTTP/1.1\r\n\r\n",
                id
            ));
        }
        body.push_str(&format!("--{}--\r\n", boundary));

        let content_type = format!("multipart/mixed; boundary=\"{}\"", boundary);
        let body_bytes = body.into_bytes();

        let response = self
            .send_request_with_retry(operation, |client, token| {
                client
                    .post(GMAIL_BATCH_URL)
                    .bearer_auth(token)
                    .header("Content-Type", content_type.clone())
                    .body(body_bytes.clone())
            })
            .await?;

        // Extract the response boundary from Content-Type header.
        let resp_content_type = response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let resp_boundary = extract_batch_boundary(&resp_content_type).ok_or_else(|| {
            AppError::SyncError(format!(
                "Batch response missing boundary in Content-Type: {}",
                resp_content_type
            ))
        })?;

        let text = response
            .text()
            .await
            .map_err(|e| AppError::SyncError(format!("Failed to read batch response body: {}", e)))?;

        Ok(parse_batch_parts(&text, &resp_boundary))
    }

    /// Deserialize a JSON string (one batch sub-response body) into a provider result.
    async fn parse_batch_part_json(
        &self,
        json: &str,
    ) -> Result<(Email, provider::EmailCategory, Vec<provider::AttachmentInfo>)> {
        let msg: GmailMessage = serde_json::from_str(json)
            .map_err(|e| AppError::SyncError(format!("Failed to parse batch message JSON: {}", e)))?;
        let (email, category, attachments) = self.parse_message(msg).await?;
        let cat = match category {
            EmailCategory::Primary => provider::EmailCategory::Primary,
            EmailCategory::Social => provider::EmailCategory::Social,
            EmailCategory::Promotions => provider::EmailCategory::Promotions,
            EmailCategory::Updates => provider::EmailCategory::Updates,
            EmailCategory::Forums => provider::EmailCategory::Forums,
        };
        let atts = attachments
            .into_iter()
            .map(|a| provider::AttachmentInfo {
                attachment_id: a.attachment_id,
                filename: a.filename,
                mime_type: a.mime_type,
                size: a.size,
                inline_data: a.inline_data,
            })
            .collect();
        Ok((email, cat, atts))
    }
}

#[async_trait]
impl EmailProvider for GmailClient {
    async fn get_profile(&self) -> Result<(String, String)> {
        self.get_profile().await
    }

    async fn list_messages(
        &self,
        max_results: u32,
        page_token: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        label_filter: Option<&str>,
    ) -> Result<(Vec<MessageRef>, Option<String>)> {
        let (refs, token) = self
            .list_messages(max_results, page_token, after_timestamp, before_timestamp, label_filter)
            .await?;
        let message_refs = refs
            .into_iter()
            .map(|r| MessageRef {
                id: r.id,
                thread_id: r.thread_id,
            })
            .collect();
        Ok((message_refs, token))
    }

    async fn list_mailbox_messages(
        &self,
        mailbox: provider::ExtraMailbox,
        max_results: u32,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
    ) -> Result<Vec<MessageRef>> {
        // Gmail labels are applied via query operators. Sent lives alongside
        // regular mail; Spam/Trash require includeSpamTrash=true.
        let (label_filter, include_spam_trash, include_all_mail) = match mailbox {
            provider::ExtraMailbox::Sent => ("in:sent", false, false),
            provider::ExtraMailbox::Spam => ("in:spam", true, true),
            provider::ExtraMailbox::Trash => ("in:trash", true, true),
        };

        let mut collected: Vec<MessageRef> = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            if collected.len() as u32 >= max_results {
                break;
            }
            let remaining = max_results - collected.len() as u32;
            let page = remaining.min(100);
            let (refs, next) = self
                .list_messages_scoped(
                    page,
                    page_token.as_deref(),
                    after_timestamp,
                    before_timestamp,
                    Some(label_filter),
                    include_spam_trash,
                    include_all_mail,
                )
                .await?;
            for r in refs {
                collected.push(MessageRef {
                    id: r.id,
                    thread_id: r.thread_id,
                });
                if collected.len() as u32 >= max_results {
                    break;
                }
            }
            match next {
                Some(t) => page_token = Some(t),
                None => break,
            }
        }
        Ok(collected)
    }

    async fn get_message(
        &self,
        message_id: &str,
    ) -> Result<(Email, provider::EmailCategory, Vec<provider::AttachmentInfo>)> {
        let (email, category, attachments) = self.get_message(message_id).await?;
        let cat = match category {
            EmailCategory::Primary => provider::EmailCategory::Primary,
            EmailCategory::Social => provider::EmailCategory::Social,
            EmailCategory::Promotions => provider::EmailCategory::Promotions,
            EmailCategory::Updates => provider::EmailCategory::Updates,
            EmailCategory::Forums => provider::EmailCategory::Forums,
        };
        let atts = attachments
            .into_iter()
            .map(|a| provider::AttachmentInfo {
                attachment_id: a.attachment_id,
                filename: a.filename,
                mime_type: a.mime_type,
                size: a.size,
                inline_data: a.inline_data,
            })
            .collect();
        Ok((email, cat, atts))
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
    ) -> Result<provider::SentMessageMeta> {
        self.send_reply(
            from_email,
            to_emails,
            cc_emails,
            thread_id,
            original_message_id,
            subject,
            body,
            attachments,
        )
        .await
    }

    async fn send_new_email(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<provider::SentMessageMeta> {
        self.send_new_email(from_email, to_emails, cc_emails, subject, body, attachments)
            .await
    }

    async fn fetch_attachment_bytes(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        self.fetch_attachment_bytes(message_id, attachment_id).await
    }

    async fn create_draft(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<String> {
        self.create_draft(from_email, to_emails, cc_emails, subject, body, attachments)
            .await
    }

    async fn update_draft(
        &self,
        provider_draft_id: &str,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<String> {
        self.update_draft(
            provider_draft_id,
            from_email,
            to_emails,
            cc_emails,
            subject,
            body,
            attachments,
        )
        .await
    }

    async fn delete_draft(&self, provider_draft_id: &str) -> Result<()> {
        self.delete_draft(provider_draft_id).await
    }

    async fn list_drafts(
        &self,
        known_change_tokens: &std::collections::HashMap<String, String>,
    ) -> Result<crate::sync::draft_plan::ProviderDraftPull> {
        self.list_drafts(known_change_tokens).await
    }

    async fn batch_get_messages(
        &self,
        message_ids: &[&str],
    ) -> Result<Vec<Result<(Email, provider::EmailCategory, Vec<provider::AttachmentInfo>)>>> {
        type ProviderResult = Result<(Email, provider::EmailCategory, Vec<provider::AttachmentInfo>)>;

        const BOUNDARY: &str = "batch_emailops_v1";

        // Send the first batch and parse all sub-responses.
        let initial_parts = self
            .send_batch_and_parse(message_ids, BOUNDARY, "batch get messages")
            .await?;

        // Slot results by original index. 429s are tracked for retry.
        let mut final_results: Vec<Option<ProviderResult>> = (0..message_ids.len()).map(|_| None).collect();
        let mut rate_limited: Vec<usize> = Vec::new();

        for (idx, part) in initial_parts.into_iter().enumerate() {
            match part {
                Ok(json) => final_results[idx] = Some(self.parse_batch_part_json(&json).await),
                Err(429) => rate_limited.push(idx),
                Err(status) => {
                    final_results[idx] = Some(Err(AppError::SyncError(format!(
                        "Batch sub-request failed with HTTP {}",
                        status
                    ))))
                }
            }
        }

        // Retry any 429-rate-limited sub-requests with exponential backoff.
        let mut delay_ms = GMAIL_INITIAL_BACKOFF_MS;
        for attempt in 0..GMAIL_MAX_RETRIES {
            if rate_limited.is_empty() {
                break;
            }

            self.emit_retry_log(
                &format!("{} rate-limited batch sub-request(s)", rate_limited.len()),
                attempt + 1,
                delay_ms,
                StatusCode::TOO_MANY_REQUESTS,
            );
            sleep(Duration::from_millis(delay_ms.min(GMAIL_MAX_BACKOFF_MS))).await;
            delay_ms = (delay_ms * 2).min(GMAIL_MAX_BACKOFF_MS);

            let retry_ids: Vec<&str> = rate_limited.iter().map(|&i| message_ids[i]).collect();
            let retry_parts = match self
                .send_batch_and_parse(&retry_ids, BOUNDARY, "retry rate-limited batch sub-requests")
                .await
            {
                Ok(parts) => parts,
                Err(e) => {
                    // Transport / auth failure — propagate to all still-pending slots.
                    let msg = e.to_string();
                    for &idx in &rate_limited {
                        final_results[idx] = Some(Err(AppError::SyncError(msg.clone())));
                    }
                    rate_limited.clear();
                    break;
                }
            };

            let mut still_limited: Vec<usize> = Vec::new();
            for (local_i, part) in retry_parts.into_iter().enumerate() {
                let original_idx = rate_limited[local_i];
                match part {
                    Ok(json) => final_results[original_idx] = Some(self.parse_batch_part_json(&json).await),
                    Err(429) => still_limited.push(original_idx),
                    Err(status) => {
                        final_results[original_idx] = Some(Err(AppError::SyncError(format!(
                            "Batch sub-request failed with HTTP {}",
                            status
                        ))))
                    }
                }
            }
            rate_limited = still_limited;
        }

        // Any remaining 429s after all retry attempts → permanent error for this sync.
        for idx in rate_limited {
            final_results[idx] = Some(Err(AppError::SyncError(
                "Batch sub-request rate limited — retries exhausted".to_string(),
            )));
        }

        // Collect, filling any structural gaps (shouldn't happen with a well-formed response).
        Ok(final_results
            .into_iter()
            .map(|opt| opt.unwrap_or_else(|| Err(AppError::SyncError("Missing batch response part".to_string()))))
            .collect())
    }
}

// ── Batch API helpers ─────────────────────────────────────────────────────────

/// Extract the `boundary=` value from a `multipart/mixed` Content-Type header.
fn extract_batch_boundary(content_type: &str) -> Option<String> {
    for segment in content_type.split(';') {
        let seg = segment.trim();
        if let Some(val) = seg.strip_prefix("boundary=") {
            let boundary = val.trim().trim_matches('"');
            if !boundary.is_empty() {
                return Some(boundary.to_string());
            }
        }
    }
    None
}

/// Split a multipart/mixed `body` on `boundary` and extract JSON from each part.
///
/// Returns `Ok(json_string)` for 2xx sub-responses, `Err(http_status_code)` otherwise.
fn parse_batch_parts(body: &str, boundary: &str) -> Vec<std::result::Result<String, u16>> {
    let delimiter = format!("--{}", boundary);
    let mut results = Vec::new();

    for part in body.split(delimiter.as_str()).skip(1) {
        // The closing "----" terminator shows up as "--\r\n" or "--" after trimming
        let trimmed = part.trim_start_matches(['\r', '\n']);
        if trimmed.starts_with("--") || trimmed.is_empty() {
            continue;
        }

        match extract_json_from_batch_part(part) {
            Some((status, json)) if (200..300).contains(&status) => results.push(Ok(json)),
            Some((status, _)) => results.push(Err(status)),
            None => {
                // Malformed part — treat as an unknown server error
                results.push(Err(0));
            }
        }
    }

    results
}

/// Parse one multipart part that wraps an inner HTTP response.
///
/// Structure:
/// ```text
/// Content-Type: application/http\r\n
/// Content-ID: <response-item1>\r\n
/// \r\n
/// HTTP/1.1 200 OK\r\n
/// Content-Type: application/json; charset=UTF-8\r\n
/// \r\n
/// { ... json body ... }
/// ```
///
/// Returns `(http_status, json_body)` or `None` if the part cannot be parsed.
fn extract_json_from_batch_part(part: &str) -> Option<(u16, String)> {
    // Skip outer MIME headers — find the first blank line
    let inner_start = find_after_double_newline(part)?;
    let inner_http = &part[inner_start..];

    // Parse HTTP status line: "HTTP/1.1 200 OK\r\n" or "HTTP/1.1 200 OK\n"
    let newline_pos = inner_http.find('\n')?;
    let status_line = inner_http[..newline_pos].trim();
    let status: u16 = status_line.split_whitespace().nth(1)?.parse().ok()?;
    let after_status_line = &inner_http[newline_pos + 1..];

    // Skip inner HTTP response headers — find the next blank line → JSON body
    let body_start = find_after_double_newline(after_status_line)?;
    let json = after_status_line[body_start..].trim().to_string();

    if json.is_empty() {
        None
    } else {
        Some((status, json))
    }
}

/// Return the byte offset just past the first `\r\n\r\n` or `\n\n` in `s`.
fn find_after_double_newline(s: &str) -> Option<usize> {
    if let Some(pos) = s.find("\r\n\r\n") {
        Some(pos + 4)
    } else {
        s.find("\n\n").map(|pos| pos + 2)
    }
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

fn is_retryable_gmail_error(status: StatusCode, body: &str) -> bool {
    if matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    ) {
        return true;
    }

    if status == StatusCode::FORBIDDEN {
        return gmail_error_reasons(body).iter().any(|reason| {
            matches!(
                reason.as_str(),
                "rateLimitExceeded" | "userRateLimitExceeded" | "quotaExceeded"
            )
        });
    }

    false
}

/// How to respond to a retryable Gmail error.
#[derive(Debug, PartialEq)]
enum RetryPlan {
    /// Sleep this many milliseconds, then retry within the attempt loop.
    Wait(u64),
    /// The provider's retry window ends past what in-loop backoff can
    /// bridge — stop retrying and pause the account until this unix-seconds
    /// instant. Every in-window retry is itself a quota-charged API call, so
    /// continuing the ladder only digs the rate-limit hole deeper.
    GateUntil(i64),
}

/// Pick the wait strategy for a retryable response. The provider's own hint
/// (`Retry-After` header, or the "Retry after <RFC3339>" timestamp Gmail
/// embeds in 429 error messages) wins over the exponential fallback; hints
/// longer than [`GMAIL_MAX_BACKOFF_MS`] gate the account instead of sleeping
/// a sync task for minutes.
fn plan_rate_limit_wait(
    headers: &reqwest::header::HeaderMap,
    body: &str,
    fallback_delay_ms: u64,
    now_secs: i64,
) -> RetryPlan {
    let provider_wait_ms = retry_after_ms(headers, now_secs).or_else(|| {
        retry_until_millis_from_body(body)
            .map(|until_ms| u64::try_from((until_ms - now_secs * 1000).max(0)).unwrap_or(0))
    });

    match provider_wait_ms {
        Some(wait_ms) if wait_ms > GMAIL_MAX_BACKOFF_MS => {
            let until_secs = now_secs.saturating_add(i64::try_from(wait_ms.div_ceil(1000)).unwrap_or(i64::MAX));
            RetryPlan::GateUntil(until_secs)
        }
        Some(wait_ms) => RetryPlan::Wait(wait_ms),
        None => RetryPlan::Wait(fallback_delay_ms.min(GMAIL_MAX_BACKOFF_MS)),
    }
}

fn retry_after_ms(headers: &reqwest::header::HeaderMap, now_secs: i64) -> Option<u64> {
    let raw = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(seconds.saturating_mul(1000));
    }
    // RFC 7231 also allows an HTTP-date form ("Fri, 24 Jul 2026 14:18:08 GMT").
    let date = chrono::DateTime::parse_from_rfc2822(raw).ok()?;
    u64::try_from((date.timestamp() - now_secs).max(0))
        .ok()
        .map(|s| s.saturating_mul(1000))
}

/// Gmail 429 bodies often carry the retry hint only inside the human-readable
/// message, e.g. "User-rate limit exceeded.  Retry after
/// 2026-07-24T14:18:08.861Z" — with no `Retry-After` header at all.
fn retry_until_millis_from_body(body: &str) -> Option<i64> {
    // Hard-coded literal that cannot fail by construction — syntax checked by tests.
    #[allow(clippy::unwrap_used)]
    static RETRY_AFTER_TS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)retry after\s+(\d{4}-\d{2}-\d{2}T[0-9:.]+(?:Z|[+-][0-9:]+))").unwrap()
    });
    let ts = RETRY_AFTER_TS.captures(body)?.get(1)?.as_str();
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Process-wide "paused until" map, keyed by account id. `GmailClient`s are
/// constructed per operation (see `services::accounts` / `services::emails`),
/// so an instance field would not survive to the next command or sync run —
/// the gate must outlive the client.
static RATE_LIMIT_GATES: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

fn set_rate_limit_gate(account_id: &str, until_secs: i64) {
    RATE_LIMIT_GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(account_id.to_string(), until_secs);
}

/// `Some(until)` while the account is paused; expired entries are removed.
fn rate_limit_gate_until(account_id: &str, now_secs: i64) -> Option<i64> {
    let mut gates = RATE_LIMIT_GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match gates.get(account_id) {
        Some(&until) if now_secs < until => Some(until),
        Some(_) => {
            gates.remove(account_id);
            None
        }
        None => None,
    }
}

fn format_gate_until(until_secs: i64) -> String {
    chrono::DateTime::from_timestamp(until_secs, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| until_secs.to_string())
}

fn gmail_error_reasons(body: &str) -> Vec<String> {
    serde_json::from_str::<GmailApiErrorEnvelope>(body)
        .ok()
        .and_then(|envelope| envelope.error)
        .and_then(|error| error.errors)
        .map(|errors| errors.into_iter().filter_map(|detail| detail.reason).collect())
        .unwrap_or_default()
}

fn format_gmail_error(status: StatusCode, body: &str) -> String {
    let parsed_message = serde_json::from_str::<GmailApiErrorEnvelope>(body)
        .ok()
        .and_then(|envelope| envelope.error)
        .and_then(|error| error.message);

    parsed_message
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| format!("HTTP {} {}", status.as_u16(), body))
}

fn parse_email_address(from: &str) -> (String, String) {
    // Parse "Name <email@example.com>" or "email@example.com"
    if let Some(start) = from.find('<') {
        if let Some(end) = from.find('>') {
            let name = from[..start].trim().trim_matches('"').to_string();
            let email = from[start + 1..end].to_string();
            return (if name.is_empty() { email.clone() } else { name }, email);
        }
    }
    (from.to_string(), from.to_string())
}

pub fn decode_base64_url_bytes(data: &str) -> Result<Vec<u8>> {
    use base64::{
        alphabet,
        engine::{self, GeneralPurpose, GeneralPurposeConfig},
        Engine,
    };

    const LENIENT: GeneralPurpose = GeneralPurpose::new(
        &alphabet::URL_SAFE,
        GeneralPurposeConfig::new().with_decode_padding_mode(engine::DecodePaddingMode::Indifferent),
    );

    let cleaned: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    LENIENT
        .decode(&cleaned)
        .map_err(|e| AppError::SyncError(format!("Base64 decode error: {}", e)))
}

fn base64_url_decode(data: &str) -> Result<String> {
    use base64::{
        alphabet,
        engine::{self, GeneralPurpose, GeneralPurposeConfig},
        Engine,
    };

    const LENIENT: GeneralPurpose = GeneralPurpose::new(
        &alphabet::URL_SAFE,
        GeneralPurposeConfig::new().with_decode_padding_mode(engine::DecodePaddingMode::Indifferent),
    );

    let cleaned: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let bytes = LENIENT
        .decode(&cleaned)
        .map_err(|e| AppError::SyncError(format!("Base64 decode error: {}", e)))?;

    String::from_utf8(bytes).map_err(|e| AppError::SyncError(format!("UTF-8 decode error: {}", e)))
}

fn base64_url_encode(data: &[u8]) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    URL_SAFE_NO_PAD.encode(data)
}

/// Body of a successful `POST /users/me/messages/send` — Gmail returns the
/// canonical `id` / `threadId` of the freshly created Sent message.
#[derive(serde::Deserialize)]
struct GmailSendResponse {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: String,
}

/// Assemble the [`SentMessageMeta`] for a successful send. The response body
/// is best-effort — a malformed/absent body degrades to a meta with only the
/// Message-ID header, never an error (the mail IS sent at this point).
fn sent_meta_from_response(
    response: Option<GmailSendResponse>,
    message_id_header: Option<String>,
) -> crate::sync::provider::SentMessageMeta {
    crate::sync::provider::SentMessageMeta {
        provider_message_id: response.as_ref().map(|r| r.id.clone()),
        provider_thread_id: response.as_ref().map(|r| r.thread_id.clone()),
        message_id_header,
    }
}

use crate::sync::mime_builder::reply_subject;

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Convert plain text email body to HTML with proper formatting:
/// - Outlook-style [image_url]<link_url> → clickable image
/// - [image_url] alone → inline image
/// - Bare URLs → clickable links
/// - Newlines → <br>
/// - Preserve paragraph breaks
fn plain_text_to_html(text: &str) -> String {
    let escaped = html_escape(text);
    let mut result = String::with_capacity(escaped.len() * 2);

    result.push_str("<div style=\"white-space:pre-wrap;\">");

    for line in escaped.lines() {
        let processed = process_plain_text_line(line);
        result.push_str(&processed);
        result.push_str("<br>");
    }

    result.push_str("</div>");
    result
}

fn process_plain_text_line(line: &str) -> String {
    let mut result = String::new();
    let mut remaining = line;

    while !remaining.is_empty() {
        // Look for Outlook pattern: [url]&lt;link&gt;
        if let Some(bracket_start) = remaining.find('[') {
            // Add text before the bracket
            result.push_str(&remaining[..bracket_start]);

            let after_bracket = &remaining[bracket_start + 1..];
            if let Some(bracket_end) = after_bracket.find(']') {
                let inner = &after_bracket[..bracket_end];
                let after_close = &after_bracket[bracket_end + 1..];

                // Check if inner looks like an image URL
                let is_image = inner.contains("http")
                    && (inner.ends_with(".png")
                        || inner.ends_with(".jpg")
                        || inner.ends_with(".jpeg")
                        || inner.ends_with(".gif")
                        || inner.ends_with(".svg")
                        || inner.ends_with(".webp")
                        || inner.contains("/img/")
                        || inner.contains("/image/")
                        || inner.contains("/banners/"));

                // Check for &lt;link&gt; immediately after
                if after_close.starts_with("&lt;") {
                    if let Some(link_end) = after_close.find("&gt;") {
                        let link = &after_close[4..link_end]; // skip "&lt;"
                        let rest = &after_close[link_end + 4..]; // skip "&gt;"

                        if is_image && is_safe_remote_url(inner) && is_safe_remote_url(link) {
                            // [image]<link> → clickable image
                            result.push_str(&format!(
                                "<a href=\"{}\"><img src=\"{}\" style=\"max-width:100%;height:auto;\" /></a>",
                                link, inner
                            ));
                        } else if is_safe_remote_url(link) {
                            // [url]<link> → linked text (use url as display)
                            result.push_str(&format!("<a href=\"{}\">{}</a>", link, inner));
                        } else {
                            result.push_str(inner);
                        }
                        remaining = rest;
                        continue;
                    }
                }

                // Just [image_url] without a link
                if is_image && is_safe_remote_url(inner) {
                    result.push_str(&format!(
                        "<img src=\"{}\" style=\"max-width:100%;height:auto;\" />",
                        inner
                    ));
                    remaining = after_close;
                    continue;
                }

                // Not a special pattern — keep as-is
                result.push('[');
                remaining = after_bracket;
                continue;
            }

            // No closing bracket found
            result.push('[');
            remaining = after_bracket;
            continue;
        }

        // Look for bare URLs: https://... or http://...
        if let Some(url_start) = remaining.find("https://").or_else(|| remaining.find("http://")) {
            result.push_str(&remaining[..url_start]);
            let url_rest = &remaining[url_start..];
            // URL ends at whitespace, &lt;, or end of string
            let url_end = url_rest
                .find(|c: char| c.is_whitespace() || c == '&')
                .unwrap_or(url_rest.len());
            let url = &url_rest[..url_end];
            if is_safe_remote_url(url) {
                result.push_str(&format!("<a href=\"{}\">{}</a>", url, url));
            } else {
                result.push_str(url);
            }
            remaining = &url_rest[url_end..];
            continue;
        }

        // No more special patterns — append the rest
        result.push_str(remaining);
        break;
    }

    result
}

fn extract_name_from_content_type(value: &str) -> Option<String> {
    // Parse Content-Type header for name parameter
    // Handles: application/pdf; name="invoice.pdf"
    if let Some(start) = value.find("name=\"") {
        let rest = &value[start + 6..];
        if let Some(end) = rest.find('"') {
            let name = rest[..end].to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    if let Some(start) = value.find("name=") {
        let rest = &value[start + 5..];
        let end = rest.find(';').unwrap_or(rest.len());
        let name = rest[..end].trim().trim_matches('"').to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn extract_filename_from_disposition(value: &str) -> Option<String> {
    // Parse Content-Disposition header for filename parameter
    // Handles: attachment; filename="invoice.pdf" and filename*=UTF-8''invoice.pdf
    let lower = value.to_lowercase();
    if !lower.contains("attachment") && !lower.contains("filename") {
        return None;
    }

    // Try filename="..." first
    if let Some(start) = value.find("filename=\"") {
        let rest = &value[start + 10..];
        if let Some(end) = rest.find('"') {
            let name = rest[..end].to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }

    // Try filename=... (no quotes)
    if let Some(start) = value.find("filename=") {
        let rest = &value[start + 9..];
        let end = rest.find(';').unwrap_or(rest.len());
        let name = rest[..end].trim().trim_matches('"').to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }

    None
}

/// Reduce a Content-Id (or other arbitrary string) to characters safe to embed
/// inside a filename. We only need uniqueness across inline images of one
/// email, so a strict allowlist is fine — anything outside ASCII alphanumerics,
/// `-` or `_` becomes `_`. Capped at 64 chars to keep filenames sane.
fn sanitize_filename_fragment(input: &str) -> String {
    let mut out: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.len() > 64 {
        out.truncate(64);
    }
    if out.is_empty() {
        out.push('x');
    }
    out
}

fn mime_to_extension(mime_type: &str) -> &str {
    match mime_type {
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/gzip" => "gz",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.ms-excel" => "xls",
        "application/msword" => "doc",
        "application/octet-stream" => "bin",
        "text/csv" => "csv",
        "text/html" => "html",
        m if m.starts_with("image/png") => "png",
        m if m.starts_with("image/jpeg") => "jpg",
        m if m.starts_with("image/gif") => "gif",
        m if m.starts_with("image/webp") => "webp",
        m if m.starts_with("image/svg") => "svg",
        m if m.starts_with("image/") => "img",
        _ => "bin",
    }
}

fn is_safe_remote_url(value: &str) -> bool {
    url::Url::parse(value)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- rate-limit retry planning tests ---

    /// Arbitrary fixed "now" so planner tests are deterministic without the
    /// global clock seam.
    const RL_NOW: i64 = 1_753_366_000;

    fn headers_with_retry_after(value: &str) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, value.parse().unwrap());
        headers
    }

    fn rate_limit_body(until_secs: i64) -> String {
        let ts = chrono::DateTime::from_timestamp(until_secs, 861_000_000)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        format!(
            r#"{{"error":{{"code":429,"message":"User-rate limit exceeded.  Retry after {ts}","errors":[{{"reason":"rateLimitExceeded","domain":"usageLimits"}}]}}}}"#
        )
    }

    #[test]
    fn retry_after_header_parses_delta_seconds() {
        assert_eq!(retry_after_ms(&headers_with_retry_after("12"), RL_NOW), Some(12_000));
    }

    #[test]
    fn retry_after_header_parses_http_date() {
        let value = chrono::DateTime::from_timestamp(RL_NOW + 90, 0).unwrap().to_rfc2822();
        assert_eq!(retry_after_ms(&headers_with_retry_after(&value), RL_NOW), Some(90_000));
    }

    #[test]
    fn retry_after_header_http_date_in_past_clamps_to_zero() {
        let value = chrono::DateTime::from_timestamp(RL_NOW - 90, 0).unwrap().to_rfc2822();
        assert_eq!(retry_after_ms(&headers_with_retry_after(&value), RL_NOW), Some(0));
    }

    #[test]
    fn retry_after_header_ignores_garbage() {
        assert_eq!(retry_after_ms(&headers_with_retry_after("soon"), RL_NOW), None);
    }

    #[test]
    fn body_retry_timestamp_is_parsed_from_gmail_error_message() {
        assert_eq!(
            retry_until_millis_from_body(&rate_limit_body(RL_NOW + 600)),
            Some((RL_NOW + 600) * 1000 + 861)
        );
    }

    #[test]
    fn body_without_retry_timestamp_yields_none() {
        assert_eq!(
            retry_until_millis_from_body(r#"{"error":{"code":503,"message":"Backend Error"}}"#),
            None
        );
    }

    #[test]
    fn short_header_window_waits_in_loop() {
        let plan = plan_rate_limit_wait(&headers_with_retry_after("5"), "", 1_000, RL_NOW);
        assert_eq!(plan, RetryPlan::Wait(5_000));
    }

    #[test]
    fn short_body_window_waits_in_loop() {
        let plan = plan_rate_limit_wait(
            &reqwest::header::HeaderMap::new(),
            &rate_limit_body(RL_NOW + 10),
            1_000,
            RL_NOW,
        );
        // Millisecond precision is preserved so we never retry before the window opens.
        assert_eq!(plan, RetryPlan::Wait(10_861));
    }

    #[test]
    fn no_retry_info_falls_back_to_exponential_delay() {
        let plan = plan_rate_limit_wait(&reqwest::header::HeaderMap::new(), "", 4_000, RL_NOW);
        assert_eq!(plan, RetryPlan::Wait(4_000));
    }

    #[test]
    fn long_body_window_gates_the_account() {
        let plan = plan_rate_limit_wait(
            &reqwest::header::HeaderMap::new(),
            &rate_limit_body(RL_NOW + 600),
            1_000,
            RL_NOW,
        );
        assert_eq!(plan, RetryPlan::GateUntil(RL_NOW + 601));
    }

    #[test]
    fn long_header_window_gates_the_account() {
        let plan = plan_rate_limit_wait(&headers_with_retry_after("300"), "", 1_000, RL_NOW);
        assert_eq!(plan, RetryPlan::GateUntil(RL_NOW + 300));
    }

    #[test]
    fn header_takes_precedence_over_body_timestamp() {
        let plan = plan_rate_limit_wait(
            &headers_with_retry_after("5"),
            &rate_limit_body(RL_NOW + 600),
            1_000,
            RL_NOW,
        );
        assert_eq!(plan, RetryPlan::Wait(5_000));
    }

    // --- rate-limit gate tests ---

    #[test]
    fn gate_blocks_until_expiry_then_clears() {
        let account = "test-gate-blocks-until-expiry";
        set_rate_limit_gate(account, RL_NOW + 120);
        assert_eq!(rate_limit_gate_until(account, RL_NOW), Some(RL_NOW + 120));
        assert_eq!(rate_limit_gate_until(account, RL_NOW + 120), None);
        // The expired entry is removed for good, not just skipped.
        assert_eq!(rate_limit_gate_until(account, RL_NOW), None);
    }

    #[test]
    fn ungated_account_is_not_blocked() {
        assert_eq!(rate_limit_gate_until("test-gate-never-set", RL_NOW), None);
    }

    #[tokio::test]
    async fn far_future_rate_limit_fails_fast_and_gates_account() {
        use wiremock::matchers::any;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let until = chrono::Utc::now().timestamp() + 600;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(429).set_body_raw(rate_limit_body(until), "application/json"))
            .mount(&server)
            .await;

        let client =
            GmailClient::new("tok".into(), None, None, Some("test-gate-wiremock".into())).with_base_url(server.uri());

        let started = std::time::Instant::now();
        let err = client.get_profile().await.unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "should fail fast instead of sleeping through the backoff ladder"
        );
        assert!(
            err.to_string().to_lowercase().contains("rate limit"),
            "unexpected error: {err}"
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "must not burn retries on an exhausted quota"
        );

        // Follow-up calls short-circuit without touching the network.
        let err = client.get_profile().await.unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("rate limit"),
            "unexpected error: {err}"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[test]
    fn gmail_send_response_yields_full_sent_meta() {
        let response: GmailSendResponse =
            serde_json::from_str(r#"{"id":"18f2a","threadId":"18e11","labelIds":["SENT"]}"#).unwrap();
        let meta = sent_meta_from_response(Some(response), Some("<mid@local>".into()));
        assert_eq!(meta.provider_message_id.as_deref(), Some("18f2a"));
        assert_eq!(meta.provider_thread_id.as_deref(), Some("18e11"));
        assert_eq!(meta.message_id_header.as_deref(), Some("<mid@local>"));
    }

    #[test]
    fn missing_send_response_body_degrades_to_header_only_meta() {
        let meta = sent_meta_from_response(None, Some("<mid@local>".into()));
        assert!(meta.provider_message_id.is_none());
        assert!(meta.provider_thread_id.is_none());
        assert_eq!(meta.message_id_header.as_deref(), Some("<mid@local>"));
    }

    /// A `drafts.list` page whose entries carry MINIMAL message stubs.
    fn drafts_list_page(entries: &[(&str, &str)], next_page_token: Option<&str>) -> String {
        let drafts = entries
            .iter()
            .map(|(draft_id, message_id)| {
                format!(r#"{{"id":"{draft_id}","message":{{"id":"{message_id}","threadId":"t-{message_id}"}}}}"#)
            })
            .collect::<Vec<_>>()
            .join(",");
        match next_page_token {
            Some(token) => format!(r#"{{"drafts":[{drafts}],"nextPageToken":"{token}"}}"#),
            None => format!(r#"{{"drafts":[{drafts}]}}"#),
        }
    }

    /// A `format=full` draft read, minimal but complete enough to parse.
    fn full_draft_body(draft_id: &str, message_id: &str, subject: &str) -> String {
        format!(
            r#"{{"id":"{draft_id}","message":{{"id":"{message_id}","threadId":"t-{message_id}",
               "labelIds":["DRAFT"],"internalDate":"1700000000000","snippet":"",
               "payload":{{"mimeType":"text/plain","headers":[{{"name":"Subject","value":"{subject}"}},
               {{"name":"To","value":"dest@example.com"}}],"body":{{"size":0}}}}}}}}"#
        )
    }

    #[tokio::test]
    async fn unchanged_drafts_are_not_re_read_on_the_next_pull() {
        // Regression: 91k `drafts.get` calls against ~100 drafts, because the
        // pull pass downloaded every draft in full on every 60-second tick.
        use wiremock::matchers::{method, path, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/me/drafts"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                drafts_list_page(&[("d-1", "m-1"), ("d-2", "m-2")], None),
                "application/json",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/users/me/drafts/d-\d$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(full_draft_body("d-1", "m-1", "Hello"), "application/json"),
            )
            .mount(&server)
            .await;

        let client = GmailClient::new("tok".into(), None, None, None).with_base_url(server.uri());

        // Cold: nothing known, so both drafts are read in full.
        let first = client
            .list_drafts(&std::collections::HashMap::new())
            .await
            .expect("first");
        assert_eq!(first.changed.len(), 2);
        assert_eq!(first.present_ids, vec!["d-1".to_string(), "d-2".to_string()]);
        assert_eq!(
            first.changed[0].provider_message_id.as_deref(),
            Some("m-1"),
            "the change token must be carried back for storage"
        );

        // Warm: the stored tokens still match, so no content read is issued.
        let known: std::collections::HashMap<String, String> = [
            ("d-1".to_string(), "m-1".to_string()),
            ("d-2".to_string(), "m-2".to_string()),
        ]
        .into_iter()
        .collect();
        let second = client.list_drafts(&known).await.expect("second");
        assert!(second.changed.is_empty(), "unchanged drafts must not be re-read");
        assert_eq!(
            second.present_ids,
            vec!["d-1".to_string(), "d-2".to_string()],
            "skipped drafts still count as present, or the prune pass deletes them"
        );

        let gets = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().starts_with("/users/me/drafts/"))
            .count();
        assert_eq!(gets, 2, "the second pull must add zero drafts.get calls");
    }

    #[tokio::test]
    async fn a_repeating_page_token_aborts_instead_of_looping() {
        // A provider that keeps handing back a pageToken would otherwise spin
        // the sync task forever. Failing is also safer than stopping early: a
        // partial list would drive the prune pass into deleting unseen drafts.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/me/drafts"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(r#"{"drafts":[],"nextPageToken":"same"}"#, "application/json"),
            )
            .mount(&server)
            .await;

        let client = GmailClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let err = client
            .list_drafts(&std::collections::HashMap::new())
            .await
            .expect_err("must not loop forever");
        assert!(
            err.to_string().to_lowercase().contains("too many pages"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn drafts_list_follows_every_page() {
        // Regression: `maxResults=100` with no pagination fed a truncated
        // keep-list to the prune pass, deleting drafts past the first page.
        use wiremock::matchers::{method, path, path_regex, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/me/drafts"))
            .and(query_param("pageToken", "page-2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(drafts_list_page(&[("d-2", "m-2")], None), "application/json"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/me/drafts"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(drafts_list_page(&[("d-1", "m-1")], Some("page-2")), "application/json"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/users/me/drafts/d-\d$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(full_draft_body("d-1", "m-1", "Hello"), "application/json"),
            )
            .mount(&server)
            .await;

        let client = GmailClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let pull = client
            .list_drafts(&std::collections::HashMap::new())
            .await
            .expect("pull");

        assert_eq!(
            pull.present_ids,
            vec!["d-1".to_string(), "d-2".to_string()],
            "page 2 drafts must reach the prune keep-list"
        );
    }

    #[test]
    fn new_client_uses_production_gmail_base_by_default() {
        // Guard against an accidental swap of the default URL — production
        // sync must keep hitting gmail.googleapis.com.
        let c = GmailClient::new("tok".into(), None, None, None);
        assert_eq!(c.base_url, GMAIL_API_BASE);
    }

    #[test]
    fn with_base_url_overrides_default_for_test_mock() {
        // Builder used by sync::mock::MockProviderServer to redirect HTTP at
        // a wiremock instance. If this stops working, every cassette-driven
        // test silently calls the real Gmail API.
        let c = GmailClient::new("tok".into(), None, None, None).with_base_url("http://127.0.0.1:9999");
        assert_eq!(c.base_url, "http://127.0.0.1:9999");
    }

    fn make_body(data: Option<&str>) -> Option<GmailBody> {
        Some(GmailBody {
            data: data.map(String::from),
            size: data.map_or(0, |d| d.len() as i64),
            attachment_id: None,
        })
    }

    fn make_part(mime: &str, data: Option<&str>, children: Option<Vec<GmailPart>>) -> GmailPart {
        GmailPart {
            mime_type: mime.to_string(),
            filename: None,
            headers: None,
            body: make_body(data),
            parts: children,
        }
    }

    fn make_payload(mime: &str, data: Option<&str>, parts: Option<Vec<GmailPart>>) -> GmailPayload {
        GmailPayload {
            headers: vec![],
            body: make_body(data),
            parts,
            mime_type: Some(mime.to_string()),
        }
    }

    fn encode(text: &str) -> String {
        base64_url_encode(text.as_bytes())
    }

    fn gmail_client() -> GmailClient {
        GmailClient::new("fake-token".into(), None, None, None)
    }

    // --- base64_url_decode tests ---

    #[test]
    fn decode_no_padding() {
        let encoded = encode("hello world");
        assert_eq!(base64_url_decode(&encoded).unwrap(), "hello world");
    }

    #[test]
    fn decode_with_padding() {
        // "hello" encodes to "aGVsbG8=" in standard base64
        let result = base64_url_decode("aGVsbG8=").unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn decode_with_whitespace() {
        let encoded = encode("test data");
        let with_newlines = format!("{}\n{}", &encoded[..4], &encoded[4..]);
        assert_eq!(base64_url_decode(&with_newlines).unwrap(), "test data");
    }

    #[test]
    fn decode_url_safe_chars() {
        // "n>o?" encodes with URL-safe chars - and _ instead of + and /
        let text = "n>o?n>o?"; // produces chars that differ between standard and URL-safe
        let encoded = base64_url_encode(text.as_bytes());
        let decoded = base64_url_decode(&encoded).unwrap();
        assert_eq!(decoded, text);
    }

    #[test]
    fn decode_html_content() {
        let html = "<html>\r\n<head>\r\n<meta http-equiv=\"Content-Type\">";
        let encoded = base64_url_encode(html.as_bytes());
        assert_eq!(base64_url_decode(&encoded).unwrap(), html);
    }

    // --- find_body_part tests ---

    #[test]
    fn find_body_direct_match() {
        let client = gmail_client();
        let payload = make_payload("text/html", Some(&encode("<b>hi</b>")), None);
        let result = client.find_body_part(&payload, "text/html");
        assert_eq!(result.unwrap(), "<b>hi</b>");
    }

    #[test]
    fn find_body_in_flat_parts() {
        let client = gmail_client();
        let payload = make_payload(
            "multipart/alternative",
            None,
            Some(vec![
                make_part("text/plain", Some(&encode("plain")), None),
                make_part("text/html", Some(&encode("<b>html</b>")), None),
            ]),
        );
        assert_eq!(client.find_body_part(&payload, "text/html").unwrap(), "<b>html</b>");
    }

    #[test]
    fn find_body_nested_two_levels() {
        let client = gmail_client();
        // multipart/mixed > multipart/alternative > text/html
        let payload = make_payload(
            "multipart/mixed",
            None,
            Some(vec![make_part(
                "multipart/alternative",
                None,
                Some(vec![
                    make_part("text/plain", Some(&encode("plain")), None),
                    make_part("text/html", Some(&encode("<p>deep</p>")), None),
                ]),
            )]),
        );
        assert_eq!(client.find_body_part(&payload, "text/html").unwrap(), "<p>deep</p>");
    }

    #[test]
    fn find_body_deeply_nested() {
        let client = gmail_client();
        // multipart/mixed > multipart/related > multipart/alternative > text/html
        let payload = make_payload(
            "multipart/mixed",
            None,
            Some(vec![make_part(
                "multipart/related",
                None,
                Some(vec![make_part(
                    "multipart/alternative",
                    None,
                    Some(vec![
                        make_part("text/plain", Some(&encode("plain")), None),
                        make_part("text/html", Some(&encode("<div>3 levels</div>")), None),
                    ]),
                )]),
            )]),
        );
        assert_eq!(
            client.find_body_part(&payload, "text/html").unwrap(),
            "<div>3 levels</div>"
        );
    }

    #[test]
    fn find_body_prefers_first_match() {
        let client = gmail_client();
        // Two multipart/alternative branches — should return the first text/html
        let payload = make_payload(
            "multipart/mixed",
            None,
            Some(vec![
                make_part(
                    "multipart/alternative",
                    None,
                    Some(vec![make_part("text/html", Some(&encode("<p>first</p>")), None)]),
                ),
                make_part(
                    "application/octet-stream",
                    None,
                    Some(vec![make_part(
                        "multipart/alternative",
                        None,
                        Some(vec![make_part("text/html", Some(&encode("<p>second</p>")), None)]),
                    )]),
                ),
            ]),
        );
        assert_eq!(client.find_body_part(&payload, "text/html").unwrap(), "<p>first</p>");
    }

    #[test]
    fn create_draft_response_decodes_from_minimal_message() {
        // `drafts.create` / `drafts.update` return the message in MINIMAL format
        // (no `payload`/`snippet`/`internalDate`). The response decode must only
        // depend on the draft `id`, or serde fails with "error decoding response
        // body" on an otherwise-successful 2xx.
        let body = serde_json::json!({
            "id": "r-abc123",
            "message": { "id": "m1", "threadId": "t1", "labelIds": ["DRAFT"] }
        });
        let draft: GmailDraftId = serde_json::from_value(body).expect("minimal draft decodes");
        assert_eq!(draft.id, "r-abc123");
    }

    #[test]
    fn draft_list_decodes_from_minimal_entries() {
        // `drafts.list` entries also carry a minimal `message` (id + threadId).
        let body = serde_json::json!({
            "drafts": [
                { "id": "r-1", "message": { "id": "m1", "threadId": "t1" } },
                { "id": "r-2", "message": { "id": "m2", "threadId": "t2" } }
            ],
            "resultSizeEstimate": 2
        });
        let list: GmailDraftList = serde_json::from_value(body).expect("minimal list decodes");
        let ids: Vec<_> = list.drafts.unwrap_or_default().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, vec!["r-1", "r-2"]);
    }

    #[test]
    fn find_body_none_when_missing() {
        let client = gmail_client();
        let payload = make_payload(
            "multipart/mixed",
            None,
            Some(vec![make_part("text/plain", Some(&encode("plain")), None)]),
        );
        assert!(client.find_body_part(&payload, "text/html").is_none());
    }

    #[test]
    fn find_body_skips_no_data_part() {
        let client = gmail_client();
        // text/html part with no body data (attachment-only)
        let payload = make_payload("multipart/mixed", None, Some(vec![make_part("text/html", None, None)]));
        assert!(client.find_body_part(&payload, "text/html").is_none());
    }

    #[test]
    fn find_body_falls_back_to_plain() {
        let client = gmail_client();
        // Only text/plain available, no text/html
        let payload = make_payload(
            "multipart/alternative",
            None,
            Some(vec![make_part("text/plain", Some(&encode("just plain text")), None)]),
        );
        assert!(client.find_body_part(&payload, "text/html").is_none());
        assert_eq!(
            client.find_body_part(&payload, "text/plain").unwrap(),
            "just plain text"
        );
    }

    // --- extract_body tests ---

    #[tokio::test]
    async fn extract_body_prefers_html() {
        let client = gmail_client();
        let payload = make_payload(
            "multipart/alternative",
            None,
            Some(vec![
                make_part("text/plain", Some(&encode("plain")), None),
                make_part("text/html", Some(&encode("<b>html</b>")), None),
            ]),
        );
        assert_eq!(client.extract_body("msg1", &payload).await, "<b>html</b>");
    }

    #[tokio::test]
    async fn extract_body_converts_plain_to_html() {
        let client = gmail_client();
        let payload = make_payload("text/plain", Some(&encode("line1\nline2")), None);
        let body = client.extract_body("msg2", &payload).await;
        assert!(body.contains("line1"));
        assert!(body.contains("line2"));
        assert!(body.contains("<br>"));
    }

    #[tokio::test]
    async fn extract_body_empty_when_no_parts() {
        let client = gmail_client();
        let payload = GmailPayload {
            headers: vec![],
            body: Some(GmailBody {
                data: None,
                size: 0,
                attachment_id: None,
            }),
            parts: None,
            mime_type: Some("multipart/mixed".to_string()),
        };
        assert!(client.extract_body("msg3", &payload).await.is_empty());
    }

    // --- plain_text_to_html tests ---

    #[test]
    fn plain_text_preserves_lines() {
        let result = plain_text_to_html("line1\nline2\nline3");
        assert!(result.contains("line1<br>"));
        assert!(result.contains("line2<br>"));
        assert!(result.contains("line3"));
    }

    #[test]
    fn plain_text_escapes_html() {
        let result = plain_text_to_html("<script>alert('xss')</script>");
        assert!(!result.contains("<script>"));
        assert!(result.contains("&lt;script&gt;"));
    }

    #[test]
    fn plain_text_linkifies_urls() {
        let result = plain_text_to_html("Visit https://example.com for info");
        assert!(result.contains("href="));
        assert!(result.contains("https://example.com"));
    }

    // --- find_body_attachment_id tests ---

    fn make_part_with_attachment(mime: &str, attachment_id: &str) -> GmailPart {
        GmailPart {
            mime_type: mime.to_string(),
            filename: None,
            headers: None,
            body: Some(GmailBody {
                data: None,
                size: 10000,
                attachment_id: Some(attachment_id.to_string()),
            }),
            parts: None,
        }
    }

    #[test]
    fn find_attachment_id_flat() {
        let payload = make_payload(
            "multipart/mixed",
            None,
            Some(vec![make_part_with_attachment("text/html", "att-123")]),
        );
        assert_eq!(
            GmailClient::find_body_attachment_id(&payload, "text/html").unwrap(),
            "att-123"
        );
    }

    #[test]
    fn find_attachment_id_nested() {
        let payload = make_payload(
            "multipart/mixed",
            None,
            Some(vec![GmailPart {
                mime_type: "multipart/alternative".to_string(),
                filename: None,
                headers: None,
                body: None,
                parts: Some(vec![
                    make_part_with_attachment("text/plain", "att-plain"),
                    make_part_with_attachment("text/html", "att-html"),
                ]),
            }]),
        );
        assert_eq!(
            GmailClient::find_body_attachment_id(&payload, "text/html").unwrap(),
            "att-html"
        );
    }

    #[test]
    fn find_attachment_id_none_when_inline() {
        let payload = make_payload(
            "multipart/mixed",
            None,
            Some(vec![make_part("text/html", Some(&encode("<b>inline</b>")), None)]),
        );
        // Has inline data, no attachment_id
        assert!(GmailClient::find_body_attachment_id(&payload, "text/html").is_none());
    }

    #[test]
    fn find_attachment_id_none_when_missing_type() {
        let payload = make_payload(
            "multipart/mixed",
            None,
            Some(vec![make_part_with_attachment("text/plain", "att-plain")]),
        );
        assert!(GmailClient::find_body_attachment_id(&payload, "text/html").is_none());
    }

    // --- mailbox_from_labels tests (regression: standalone sent emails leaked into inbox) ---

    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Minimal `format=full` message carrying the given labels.
    fn test_message(label_ids: &[&str]) -> GmailMessage {
        let labels_json = serde_json::to_string(label_ids).expect("labels");
        serde_json::from_str(&format!(
            r#"{{
                "id": "m1",
                "threadId": "t1",
                "labelIds": {labels_json},
                "snippet": "hello",
                "internalDate": "1700000000000",
                "payload": {{
                    "mimeType": "text/plain",
                    "headers": [
                        {{"name": "Subject", "value": "Hi"}},
                        {{"name": "From", "value": "Me <me@alias.example>"}},
                        {{"name": "To", "value": "me@example.com"}}
                    ],
                    "body": {{"size": 0}}
                }}
            }}"#
        ))
        .expect("decode test message")
    }

    #[test]
    fn mailbox_sent_only_label_routes_to_sent() {
        // Sent email to someone else: Gmail labels = [SENT]
        assert_eq!(mailbox_from_labels(&labels(&["SENT"])), "sent");
    }

    #[test]
    fn mailbox_inbox_label_routes_to_inbox() {
        assert_eq!(mailbox_from_labels(&labels(&["INBOX"])), "inbox");
        assert_eq!(mailbox_from_labels(&labels(&["INBOX", "UNREAD"])), "inbox");
    }

    #[test]
    fn mailbox_self_sent_email_stays_in_inbox() {
        // Self-sent emails carry both labels — they should remain visible in inbox.
        assert_eq!(mailbox_from_labels(&labels(&["INBOX", "SENT"])), "inbox");
    }

    #[tokio::test]
    async fn parse_message_records_the_sent_label_alongside_the_inbox_mailbox() {
        // A self-sent message keeps mailbox='inbox' so the thread stays in the
        // inbox view, but `is_sent` must still record that Gmail filed it under
        // Sent — otherwise the Sent view can never find it.
        let client = GmailClient::new("token".to_string(), None, None, None);

        let self_sent = client
            .parse_message(test_message(&["INBOX", "SENT"]))
            .await
            .expect("parse");
        assert_eq!(self_sent.0.mailbox, "inbox", "stays in the inbox view");
        assert!(self_sent.0.is_sent, "and is still sent mail");

        let received = client.parse_message(test_message(&["INBOX"])).await.expect("parse");
        assert!(!received.0.is_sent, "received mail is not sent mail");

        let pure_sent = client.parse_message(test_message(&["SENT"])).await.expect("parse");
        assert_eq!(pure_sent.0.mailbox, "sent");
        assert!(pure_sent.0.is_sent);
    }

    #[test]
    fn mailbox_no_relevant_labels_defaults_to_inbox() {
        // Archived/labelled emails (no INBOX, no SENT) default to inbox — the
        // extra-mailbox sync pass overrides this for trash/spam/etc.
        assert_eq!(mailbox_from_labels(&labels(&[])), "inbox");
        assert_eq!(mailbox_from_labels(&labels(&["CATEGORY_PERSONAL"])), "inbox");
    }

    // --- collect_inline_image_refs tests ---

    fn image_part(cid: &str, data: Option<&str>, attachment_id: Option<&str>) -> GmailPart {
        GmailPart {
            mime_type: "image/png".to_string(),
            filename: Some("inline.png".to_string()),
            headers: Some(vec![GmailHeader {
                name: "Content-Id".to_string(),
                value: format!("<{}>", cid),
            }]),
            body: Some(GmailBody {
                data: data.map(String::from),
                size: data.map_or(0, |d| d.len() as i64),
                attachment_id: attachment_id.map(String::from),
            }),
            parts: None,
        }
    }

    #[test]
    fn collect_inline_image_refs_returns_both_inline_and_attachment_id_parts() {
        // Regression: Gmail returns small inline images with `body.data` but
        // larger ones with only `body.attachmentId`. The earlier planner kept
        // the data-bearing parts and silently dropped the attachment-id-only
        // ones, leaving broken cid: references in the rendered HTML.
        let payload = GmailPayload {
            headers: vec![],
            body: None,
            mime_type: Some("multipart/related".to_string()),
            parts: Some(vec![
                image_part("inline-cid", Some("aGVsbG8"), None),
                image_part("att-cid", None, Some("att-123")),
            ]),
        };

        let refs = collect_inline_image_refs(&payload);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].content_id, "inline-cid");
        assert_eq!(refs[0].inline_data.as_deref(), Some("aGVsbG8"));
        assert_eq!(refs[0].attachment_id, None);
        assert_eq!(refs[1].content_id, "att-cid");
        assert_eq!(refs[1].inline_data, None);
        assert_eq!(refs[1].attachment_id.as_deref(), Some("att-123"));
    }

    #[test]
    fn collect_inline_image_refs_recurses_into_nested_parts() {
        // Real emails often nest multipart/related inside multipart/mixed.
        let payload = GmailPayload {
            headers: vec![],
            body: None,
            mime_type: Some("multipart/mixed".to_string()),
            parts: Some(vec![GmailPart {
                mime_type: "multipart/related".to_string(),
                filename: None,
                headers: None,
                body: None,
                parts: Some(vec![image_part("deep-cid", None, Some("att-deep"))]),
            }]),
        };

        let refs = collect_inline_image_refs(&payload);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].content_id, "deep-cid");
        assert_eq!(refs[0].attachment_id.as_deref(), Some("att-deep"));
    }

    // --- collect_attachment_infos tests ---

    #[test]
    fn collect_attachment_infos_assigns_unique_filenames_to_inline_images_without_filename() {
        // Regression: two inline images, both with no filename and the same
        // default extension, used to collapse into a single row because of the
        // `(email_id, filename)` unique index. Suffixing with Content-Id keeps
        // them distinct.
        let payload = GmailPayload {
            headers: vec![],
            body: None,
            mime_type: Some("multipart/related".to_string()),
            parts: Some(vec![
                GmailPart {
                    mime_type: "image/png".to_string(),
                    filename: Some(String::new()), // empty filename → treated as unnamed
                    headers: Some(vec![GmailHeader {
                        name: "Content-Id".to_string(),
                        value: "<img-1>".to_string(),
                    }]),
                    body: Some(GmailBody {
                        data: None,
                        size: 100,
                        attachment_id: Some("att-1".to_string()),
                    }),
                    parts: None,
                },
                GmailPart {
                    mime_type: "image/png".to_string(),
                    filename: Some(String::new()),
                    headers: Some(vec![GmailHeader {
                        name: "Content-Id".to_string(),
                        value: "<img-2>".to_string(),
                    }]),
                    body: Some(GmailBody {
                        data: None,
                        size: 200,
                        attachment_id: Some("att-2".to_string()),
                    }),
                    parts: None,
                },
            ]),
        };

        let mut infos = Vec::new();
        GmailClient::collect_attachment_infos_recursive(payload.parts.as_ref().unwrap(), &mut infos);

        assert_eq!(infos.len(), 2);
        assert_ne!(
            infos[0].filename, infos[1].filename,
            "inline images with distinct Content-Ids must produce distinct filenames"
        );
        assert!(infos[0].filename.contains("img-1"));
        assert!(infos[1].filename.contains("img-2"));
    }

    #[test]
    fn sanitize_filename_fragment_strips_unsafe_chars() {
        assert_eq!(sanitize_filename_fragment("abc-123_x"), "abc-123_x");
        assert_eq!(sanitize_filename_fragment("a@b/c.d"), "a_b_c_d");
        assert_eq!(sanitize_filename_fragment(""), "x");
        // Very long inputs are truncated.
        let long = "a".repeat(200);
        assert_eq!(sanitize_filename_fragment(&long).len(), 64);
    }

    #[test]
    fn collect_inline_image_refs_skips_images_without_content_id() {
        // Regular attachments (no Content-Id header) must not be treated as
        // inline images — they belong in the attachment list, not embedded in
        // the body.
        let payload = GmailPayload {
            headers: vec![],
            body: None,
            mime_type: Some("multipart/mixed".to_string()),
            parts: Some(vec![GmailPart {
                mime_type: "image/png".to_string(),
                filename: Some("photo.png".to_string()),
                headers: Some(vec![GmailHeader {
                    name: "Content-Disposition".to_string(),
                    value: "attachment; filename=\"photo.png\"".to_string(),
                }]),
                body: Some(GmailBody {
                    data: None,
                    size: 0,
                    attachment_id: Some("att-x".to_string()),
                }),
                parts: None,
            }]),
        };

        assert!(collect_inline_image_refs(&payload).is_empty());
    }
}
