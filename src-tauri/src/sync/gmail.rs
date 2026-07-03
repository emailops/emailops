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
/// id alone (serde ignores the extra `message` field). Read with
/// `format=full` — see [`GmailDraft`] — when the message body is actually needed.
#[derive(Debug, Deserialize)]
struct GmailDraftId {
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
    ) -> Result<()> {
        let normalized_subject = reply_subject(subject);
        let mime = crate::sync::mime_builder::build_send_mime(&crate::sync::mime_builder::SendMimeParams {
            from_email,
            to_emails,
            cc_emails,
            subject: &normalized_subject,
            in_reply_to: original_message_id.filter(|v| !v.trim().is_empty()),
            body,
            attachments,
        })?;
        let raw = base64_url_encode(mime.as_bytes());
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

        Ok(())
    }

    pub async fn send_new_email(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<()> {
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
        let payload = serde_json::json!({ "raw": raw });
        let url = format!("{}/users/me/messages/send", self.base_url);

        let response = self.send_post_json_with_retry(&url, &payload, "send new email").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to send email: {}", error_text)));
        }

        Ok(())
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

    async fn list_drafts(&self) -> Result<Vec<crate::models::ProviderDraft>> {
        // Page 1 only (maxResults=100) — the Drafts folder is small in practice
        // and the pull pass is best-effort.
        let url = format!("{}/users/me/drafts?maxResults=100", self.base_url);
        let response = self.send_get_with_retry(&url, "list drafts").await?;
        let list: GmailDraftList = response.json().await?;
        let mut out = Vec::new();
        for entry in list.drafts.unwrap_or_default() {
            let full_url = format!("{}/users/me/drafts/{}?format=full", self.base_url, entry.id);
            let full_resp = self.send_get_with_retry(&full_url, "get draft").await?;
            let full: GmailDraft = full_resp.json().await?;
            let Some(msg) = full.message else { continue };
            let (email, _cat, _atts) = self.parse_message(msg).await?;
            // The parsed body is HTML; split it so the composer renders the rich
            // source instead of escaping it as literal text.
            let (body, body_html) = crate::util::html::split_draft_body(&email.body);
            out.push(crate::models::ProviderDraft {
                provider_draft_id: entry.id,
                to_addresses: email.recipients,
                cc_addresses: email.cc,
                subject: email.subject,
                body,
                body_html,
            });
        }
        Ok(out)
    }

    async fn parse_message(&self, msg: GmailMessage) -> Result<(Email, EmailCategory, Vec<AttachmentInfo>)> {
        let headers = &msg.payload.headers;

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

                    let retry_after = retry_after_ms(response.headers());
                    let body = response.text().await.unwrap_or_default();
                    let should_retry = is_retryable_gmail_error(status, &body);

                    if should_retry && attempt < GMAIL_MAX_RETRIES {
                        let wait_ms = retry_after.unwrap_or(delay_ms).min(GMAIL_MAX_BACKOFF_MS);
                        self.emit_retry_log(operation, attempt + 1, wait_ms, status);
                        sleep(Duration::from_millis(wait_ms)).await;
                        delay_ms = (delay_ms * 2).min(GMAIL_MAX_BACKOFF_MS);
                        continue;
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
    ) -> Result<()> {
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
    ) -> Result<()> {
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

    async fn list_drafts(&self) -> Result<Vec<crate::models::ProviderDraft>> {
        self.list_drafts().await
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

fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|seconds| seconds.saturating_mul(1000))
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

fn reply_subject(subject: &str) -> String {
    if subject.to_ascii_lowercase().starts_with("re:") {
        subject.to_string()
    } else {
        format!("Re: {}", subject)
    }
}

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
