//! Microsoft Graph (Outlook / Office 365) email provider.
//!
//! Mirrors the Gmail provider's responsibilities against Microsoft Graph v1.0
//! so the rest of the app can treat Outlook accounts the same way:
//!   - OAuth access token held in an interior-mutable mutex so the client can
//!     transparently refresh on a mid-sync 401 without requiring `&mut self`.
//!   - Sync paths return `(Email, EmailCategory, Vec<AttachmentInfo>)` matching
//!     the shape services/emails.rs expects.
//!   - Retryable transport errors and HTTP 429 / 5xx responses are retried with
//!     exponential backoff and the provider emits `app-log` warnings for
//!     visibility.
//!
//! Differences from Gmail worth flagging:
//!   - Graph does not expose Gmail-style category labels (primary / social /
//!     promotions / updates / forums). We instead map `inferenceClassification`
//!     (`focused` | `other`) to Primary / Updates so the existing UI filters
//!     keep working. The `label_filter` parameter on `list_messages` is
//!     ignored — it is a Gmail query fragment that would not apply here.
//!   - Sending uses `/me/sendMail` with a JSON message (no raw MIME). Replies
//!     use `/me/messages/{id}/reply` which preserves threading automatically.
//!   - Attachments under 3 MB arrive inline as base64 `contentBytes`; larger
//!     ones only surface as metadata and are fetched on demand via
//!     `/attachments/{id}/$value`.

use async_trait::async_trait;
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::time::sleep;

use crate::models::error::{AppError, Result};
use crate::models::{AppLogEvent, Email};
use crate::sync::provider::{self, AttachmentInfo, EmailBody, EmailCategory, EmailProvider, MessageRef};

pub use crate::sync::provider::EmailAttachment;

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";
const MAX_RETRIES: u32 = 5;
const INITIAL_BACKOFF_MS: u64 = 1_000;
const MAX_BACKOFF_MS: u64 = 30_000;

/// Fields fetched on every message list/get so we can build a full `Email`
/// without a second round-trip. Kept as a constant so every query stays in
/// sync — if you change one, change both.
const MESSAGE_SELECT_FIELDS: &str = "id,conversationId,internetMessageId,subject,bodyPreview,\
    body,from,toRecipients,ccRecipients,receivedDateTime,isRead,hasAttachments,inferenceClassification";

// ── Deserialization types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GraphUser {
    #[serde(rename = "userPrincipalName")]
    user_principal_name: Option<String>,
    mail: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphMessageList {
    value: Vec<GraphMessageRef>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphMessageRef {
    id: String,
    #[serde(rename = "conversationId")]
    conversation_id: Option<String>,
}

/// Drafts-folder listing: full `Message` resources (with body + recipients),
/// unlike [`GraphMessageList`] which carries only lightweight refs.
#[derive(Debug, Deserialize)]
struct GraphMessageDraftList {
    value: Vec<GraphMessage>,
}

#[derive(Debug, Deserialize)]
struct GraphMessage {
    id: String,
    #[serde(rename = "conversationId")]
    conversation_id: Option<String>,
    #[serde(rename = "internetMessageId")]
    internet_message_id: Option<String>,
    subject: Option<String>,
    #[serde(rename = "bodyPreview")]
    body_preview: Option<String>,
    body: Option<GraphBody>,
    from: Option<GraphRecipientWrapper>,
    #[serde(rename = "toRecipients")]
    to_recipients: Option<Vec<GraphRecipientWrapper>>,
    #[serde(rename = "ccRecipients")]
    cc_recipients: Option<Vec<GraphRecipientWrapper>>,
    #[serde(rename = "receivedDateTime")]
    received_date_time: Option<String>,
    #[serde(rename = "isRead")]
    is_read: Option<bool>,
    #[serde(rename = "hasAttachments")]
    has_attachments: Option<bool>,
    #[serde(rename = "inferenceClassification")]
    inference_classification: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphBody {
    #[serde(rename = "contentType")]
    content_type: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphRecipientWrapper {
    #[serde(rename = "emailAddress")]
    email_address: Option<GraphEmailAddress>,
}

#[derive(Debug, Deserialize, Clone)]
struct GraphEmailAddress {
    name: Option<String>,
    address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphAttachmentList {
    value: Vec<GraphAttachment>,
}

#[derive(Debug, Deserialize)]
struct GraphAttachment {
    id: String,
    name: Option<String>,
    #[serde(rename = "contentType")]
    content_type: Option<String>,
    size: Option<i64>,
    #[serde(rename = "@odata.type")]
    odata_type: Option<String>,
    #[serde(rename = "contentBytes")]
    content_bytes: Option<String>,
    /// Set for inline images referenced from the HTML body via `cid:<value>`.
    /// Graph strips the angle brackets, e.g. body says `cid:abc` and this
    /// field returns `"abc"`.
    #[serde(rename = "contentId")]
    content_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphErrorEnvelope {
    error: Option<GraphError>,
}

#[derive(Debug, Deserialize)]
struct GraphError {
    code: Option<String>,
    message: Option<String>,
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct OutlookClient {
    client: Client,
    access_token: std::sync::Mutex<String>,
    refresh_token: Option<String>,
    app: Option<AppHandle>,
    account_id: Option<String>,
    /// Base URL for the Microsoft Graph API. Defaults to [`GRAPH_API_BASE`];
    /// override via [`OutlookClient::with_base_url`] in tests so the client
    /// can be pointed at a `MockProviderServer` (see `sync::mock`).
    base_url: String,
}

impl OutlookClient {
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
            base_url: GRAPH_API_BASE.to_string(),
        }
    }

    /// Override the Graph API base URL. Production code never calls this —
    /// the `MockProviderServer` test harness uses it to redirect HTTP traffic
    /// at a `wiremock` instance loaded from a recorded cassette.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Refresh access token on a transparent mid-sync 401. No user-visible log
    /// is emitted so normal token rotation doesn't pollute the output panel.
    async fn refresh_access_token(&self) -> Result<()> {
        let Some(refresh_token) = &self.refresh_token else {
            return Err(AppError::AuthError(
                "Outlook session expired and no refresh token is stored. Please re-authenticate.".to_string(),
            ));
        };
        let Some(account_id) = &self.account_id else {
            return Err(AppError::AuthError(
                "Outlook token refresh failed: account ID unknown.".to_string(),
            ));
        };
        let config = crate::sync::oauth::OAuthConfig::for_provider("outlook");
        let new_tokens = crate::sync::oauth::refresh_oauth_token(&config, refresh_token).await?;
        crate::services::accounts::store_tokens(account_id, &new_tokens)?;
        // Recover from mutex poisoning rather than panicking — the protected
        // value is a single String, no invariant to violate.
        *self
            .access_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_tokens.access_token;
        Ok(())
    }

    // ── Profile ──────────────────────────────────────────────────────────────

    pub async fn get_profile(&self) -> Result<(String, String)> {
        let url = format!("{}/me", self.base_url);
        let response = self.send_get_with_retry(&url, "get profile").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to get profile: {}", error_text)));
        }
        let user: GraphUser = response.json().await?;
        // Work/school tenants use `mail`; personal accounts sometimes only
        // populate `userPrincipalName`. Prefer whichever is present.
        let email = user
            .mail
            .clone()
            .or(user.user_principal_name.clone())
            .ok_or_else(|| AppError::SyncError("Graph profile missing email address".to_string()))?;
        let name = user.display_name.clone().unwrap_or_else(|| email.clone());
        Ok((email, name))
    }

    // ── List messages ────────────────────────────────────────────────────────

    /// Graph equivalent of Gmail's `list_messages`. Timestamps are applied as
    /// `$filter=receivedDateTime ge/le <iso8601>`. `label_filter` is ignored
    /// (Gmail-only query syntax). Paginates via `@odata.nextLink`.
    async fn list_messages(
        &self,
        max_results: u32,
        page_token: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        _label_filter: Option<&str>,
    ) -> Result<(Vec<GraphMessageRef>, Option<String>)> {
        // When a page_token is present it IS a full next_link URL returned by
        // the previous page — use it verbatim so $skiptoken state is preserved.
        // Otherwise scope to the Inbox folder (excludes Junk/Archive/Sent by
        // construction) and sort by receivedDateTime — see `build_inbox_list_url`.
        let url = match page_token {
            Some(token) => token.to_string(),
            None => build_inbox_list_url(&self.base_url, max_results, after_timestamp, before_timestamp),
        };

        let response = self.send_get_with_retry(&url, "list messages").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to list messages: {}", error_text)));
        }
        let list: GraphMessageList = response.json().await?;
        Ok((list.value, list.next_link))
    }

    // ── Get message ──────────────────────────────────────────────────────────

    pub async fn get_message(&self, message_id: &str) -> Result<(Email, EmailCategory, Vec<AttachmentInfo>)> {
        let url = format!(
            "{}/me/messages/{}?$select={}",
            self.base_url,
            urlencoding::encode(message_id),
            MESSAGE_SELECT_FIELDS
        );
        let response = self.send_get_with_retry(&url, "get message").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to get message: {}", error_text)));
        }
        let msg: GraphMessage = response.json().await?;

        // Only skip the attachments call when Graph explicitly reports `false`.
        // Some Outlook mailboxes return `hasAttachments` as `null` even when
        // attachments exist; defaulting that to "no attachments" silently drops
        // them from the UI. `list_attachments` is cheap and returns empty when
        // there really are none.
        let (attachments, inline_images) = match msg.has_attachments {
            Some(false) => (Vec::new(), Vec::new()),
            _ => self.list_attachments(&msg.id).await.unwrap_or_default(),
        };

        let (mut email, category) = parse_message(msg);

        // Replace `cid:<id>` references in the HTML body with data URIs so
        // inline images render in the WebView. Outlook's HTML bodies reference
        // attached images by content-id (e.g. <img src="cid:abc"/>); without
        // this substitution they show as broken images.
        if !inline_images.is_empty() && email.body.contains("cid:") {
            for (cid, mime, b64) in &inline_images {
                let data_uri = format!("data:{};base64,{}", mime, b64);
                email.body = email.body.replace(&format!("cid:{}", cid), &data_uri);
            }
        }

        Ok((email, category, attachments))
    }

    /// Returns `(attachments, inline_images)`.
    /// - `attachments`: every fileAttachment (inline or not), to be saved to
    ///   `email_attachment_meta` and shown in the UI.
    /// - `inline_images`: `(content_id, mime_type, base64_data)` triples for
    ///   attachments that have both a `contentId` and inline `contentBytes`,
    ///   so `cid:` references in the HTML body can be substituted with data URIs.
    async fn list_attachments(&self, message_id: &str) -> Result<(Vec<AttachmentInfo>, Vec<(String, String, String)>)> {
        // Don't use `$select` here: `contentBytes` and `contentId` live on the
        // derived type `microsoft.graph.fileAttachment`, not on the base
        // `microsoft.graph.attachment`, so Graph returns HTTP 400 when they
        // are named in `$select` against the base resource. The full payload
        // is small enough — Graph already omits `contentBytes` for files
        // larger than ~3 MB by default.
        let url = format!(
            "{}/me/messages/{}/attachments",
            self.base_url,
            urlencoding::encode(message_id),
        );
        let response = self.send_get_with_retry(&url, "list attachments").await?;
        if !response.status().is_success() {
            // Attachments are non-essential for message indexing — log and
            // return empty rather than failing the whole sync.
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            self.log(&format!(
                "Graph: failed to list attachments for {}: HTTP {} body={}",
                message_id, status, body
            ));
            return Ok((Vec::new(), Vec::new()));
        }
        let list: GraphAttachmentList = response.json().await?;
        let mut out = Vec::with_capacity(list.value.len());
        let mut inline_images = Vec::new();
        for att in list.value {
            // Skip non-file attachments (referenceAttachment, itemAttachment).
            // We only know how to download bytes for fileAttachment.
            let is_file = att
                .odata_type
                .as_deref()
                .map(|t| t.eq_ignore_ascii_case("#microsoft.graph.fileAttachment"))
                .unwrap_or(false);
            if !is_file {
                continue;
            }
            let mime_type = att
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string());

            // Collect cid → data URI mapping for inline images. We rely on the
            // presence of `contentId` (not the `isInline` flag, which Outlook
            // sets unreliably for paperclip attachments). When `contentId` is
            // set AND we got `contentBytes` inline, we can do the substitution
            // without a follow-up fetch.
            if let (Some(cid), Some(ref bytes)) = (att.content_id.as_ref(), att.content_bytes.as_ref()) {
                if !cid.is_empty() && !bytes.is_empty() {
                    inline_images.push((cid.clone(), mime_type.clone(), bytes.to_string()));
                }
            }

            // Don't filter on `isInline`. Microsoft Graph marks many real
            // paperclip attachments as inline (especially for items sent from
            // Outlook desktop/OWA), so filtering here silently drops them.
            // Inline images embedded in the body are still listed as
            // attachments so users can download the original file if they want.
            out.push(AttachmentInfo {
                attachment_id: att.id,
                filename: att.name.unwrap_or_else(|| "attachment".to_string()),
                mime_type,
                size: att.size.unwrap_or(0),
                inline_data: att.content_bytes, // None for large attachments; caller fetches on demand
            });
        }
        Ok((out, inline_images))
    }

    // ── Send ─────────────────────────────────────────────────────────────────

    pub async fn send_reply(
        &self,
        _from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        _thread_id: &str,
        original_message_id: Option<&str>,
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<crate::sync::provider::SentMessageMeta> {
        // Graph's `/reply` endpoint auto-preserves subject prefix, In-Reply-To,
        // References, and (with the `comment` form) the quoted thread history.
        let Some(msg_id) = original_message_id.filter(|s| !s.trim().is_empty()) else {
            return Err(AppError::InvalidInput(
                "send_reply requires original message ID for Outlook".to_string(),
            ));
        };

        let payload =
            crate::sync::outlook_payload::build_reply_payload(&crate::sync::outlook_payload::OutlookSendParams {
                to_emails,
                cc_emails,
                subject,
                body,
                attachments,
            });
        let url = format!("{}/me/messages/{}/reply", self.base_url, urlencoding::encode(msg_id),);
        let response = self.send_post_json_with_retry(&url, &payload, "send reply").await?;
        // /reply returns 202 Accepted with no body on success — Graph reports
        // nothing about the created Sent message, so the meta stays empty and
        // the optimistic local row is reconciled heuristically at sync time.
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to send reply: {}", error_text)));
        }
        Ok(crate::sync::provider::SentMessageMeta::default())
    }

    pub async fn send_new_email(
        &self,
        _from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<crate::sync::provider::SentMessageMeta> {
        let payload =
            crate::sync::outlook_payload::build_send_mail_payload(&crate::sync::outlook_payload::OutlookSendParams {
                to_emails,
                cc_emails,
                subject,
                body,
                attachments,
            });

        let url = format!("{}/me/sendMail", self.base_url);
        let response = self.send_post_json_with_retry(&url, &payload, "send new email").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to send email: {}", error_text)));
        }
        // /sendMail returns 202 Accepted with no body — no meta available.
        Ok(crate::sync::provider::SentMessageMeta::default())
    }

    // ── Drafts ───────────────────────────────────────────────────────────────

    pub async fn create_draft(
        &self,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<String> {
        let payload =
            crate::sync::outlook_payload::build_draft_payload(&crate::sync::outlook_payload::OutlookSendParams {
                to_emails,
                cc_emails,
                subject,
                body,
                attachments,
            });
        // POST to /me/messages creates the message as a draft.
        let url = format!("{}/me/messages", self.base_url);
        let response = self.send_post_json_with_retry(&url, &payload, "create draft").await?;
        let msg: GraphMessage = response.json().await?;
        Ok(msg.id)
    }

    pub async fn update_draft(
        &self,
        provider_draft_id: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<String> {
        let payload =
            crate::sync::outlook_payload::build_draft_payload(&crate::sync::outlook_payload::OutlookSendParams {
                to_emails,
                cc_emails,
                subject,
                body,
                attachments,
            });
        let url = format!(
            "{}/me/messages/{}",
            self.base_url,
            urlencoding::encode(provider_draft_id)
        );
        let response = self
            .send_request_with_retry("update draft", |client, token| {
                client.patch(&url).bearer_auth(token).json(&payload)
            })
            .await?;
        let msg: GraphMessage = response.json().await?;
        Ok(msg.id)
    }

    pub async fn delete_draft(&self, provider_draft_id: &str) -> Result<()> {
        let url = format!(
            "{}/me/messages/{}",
            self.base_url,
            urlencoding::encode(provider_draft_id)
        );
        self.send_request_with_retry("delete draft", |client, token| client.delete(&url).bearer_auth(token))
            .await?;
        Ok(())
    }

    pub async fn list_drafts(&self) -> Result<Vec<crate::models::ProviderDraft>> {
        let url = format!("{}/me/mailFolders/drafts/messages?$top=100", self.base_url);
        let response = self.send_get_with_retry(&url, "list drafts").await?;
        let list: GraphMessageDraftList = response.json().await?;
        let mut out = Vec::new();
        for msg in list.value {
            let provider_draft_id = msg.id.clone();
            let (email, _cat) = parse_message(msg);
            // Graph draft bodies are HTML; split so the composer renders the
            // rich source instead of escaping it as literal text.
            let (body, body_html) = crate::util::html::split_draft_body(&email.body);
            out.push(crate::models::ProviderDraft {
                provider_draft_id,
                to_addresses: email.recipients,
                cc_addresses: email.cc,
                subject: email.subject,
                body,
                body_html,
                // Graph stamps `receivedDateTime` on a draft when it is saved.
                updated_at: Some(email.timestamp),
            });
        }
        Ok(out)
    }

    // ── Attachment bytes ─────────────────────────────────────────────────────

    pub async fn fetch_attachment_bytes(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        // `$value` returns the raw binary payload of a fileAttachment without
        // the base64 JSON wrapper — cheaper for large files than fetching the
        // full attachment resource.
        let url = format!(
            "{}/me/messages/{}/attachments/{}/$value",
            self.base_url,
            urlencoding::encode(message_id),
            urlencoding::encode(attachment_id),
        );
        let response = self.send_get_with_retry(&url, "get attachment bytes").await?;
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::SyncError(format!("Failed to get attachment: {}", error_text)));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| AppError::SyncError(format!("Failed to read attachment body: {}", e)))?;
        Ok(bytes.to_vec())
    }

    // ── HTTP helpers ─────────────────────────────────────────────────────────

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
        let mut delay_ms = INITIAL_BACKOFF_MS;
        let mut auth_retried = false;

        for attempt in 0..=MAX_RETRIES {
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

                    if status == StatusCode::UNAUTHORIZED && !auth_retried {
                        auth_retried = true;
                        match self.refresh_access_token().await {
                            Ok(()) => continue,
                            Err(e) => return Err(e),
                        }
                    }

                    let retry_after = retry_after_ms(response.headers());
                    let body = response.text().await.unwrap_or_default();
                    let should_retry = is_retryable_graph_status(status);

                    if should_retry && attempt < MAX_RETRIES {
                        let wait_ms = retry_after.unwrap_or(delay_ms).min(MAX_BACKOFF_MS);
                        self.emit_retry_log(operation, attempt + 1, wait_ms, status);
                        sleep(Duration::from_millis(wait_ms)).await;
                        delay_ms = (delay_ms * 2).min(MAX_BACKOFF_MS);
                        continue;
                    }

                    return Err(AppError::SyncError(format!(
                        "Failed to {}: {}",
                        operation,
                        format_graph_error(status, &body)
                    )));
                }
                Err(error) => {
                    if is_retryable_transport_error(&error) && attempt < MAX_RETRIES {
                        self.emit_transport_retry_log(operation, attempt + 1, delay_ms, &error);
                        sleep(Duration::from_millis(delay_ms.min(MAX_BACKOFF_MS))).await;
                        delay_ms = (delay_ms * 2).min(MAX_BACKOFF_MS);
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

    // ── Logging ──────────────────────────────────────────────────────────────

    fn log(&self, message: &str) {
        let Some(app) = &self.app else {
            println!("{}", message);
            return;
        };
        let _ = app.emit(
            "app-log",
            AppLogEvent {
                level: "debug".to_string(),
                source: "sync".to_string(),
                message: message.to_string(),
            },
        );
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
                    "Outlook rate limited {} for {} (HTTP {}). Retrying in {}s (attempt {}/{})...",
                    operation,
                    account,
                    status.as_u16(),
                    seconds,
                    attempt + 1,
                    MAX_RETRIES + 1
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
                    "Transient Outlook error during {} for {}: {}. Retrying in {}s (attempt {}/{})...",
                    operation,
                    account,
                    error,
                    seconds,
                    attempt + 1,
                    MAX_RETRIES + 1
                ),
            },
        );
    }
}

// ── EmailProvider trait impl ──────────────────────────────────────────────────

#[async_trait]
impl EmailProvider for OutlookClient {
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
                // Graph messages without a conversationId (very rare — drafts,
                // single-participant loops) fall back to their own ID so the
                // rest of the app still has something to thread by.
                thread_id: r.conversation_id.unwrap_or_else(|| r.id.clone()),
                id: r.id,
            })
            .collect();
        Ok((message_refs, token))
    }

    async fn get_message(
        &self,
        message_id: &str,
    ) -> Result<(Email, provider::EmailCategory, Vec<provider::AttachmentInfo>)> {
        self.get_message(message_id).await
    }

    async fn list_mailbox_messages(
        &self,
        mailbox: provider::ExtraMailbox,
        max_results: u32,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
    ) -> Result<Vec<MessageRef>> {
        // Graph exposes well-known folder names as path segments so we can list
        // each secondary mailbox directly without resolving its folder id.
        let folder = match mailbox {
            provider::ExtraMailbox::Sent => "sentitems",
            provider::ExtraMailbox::Spam => "junkemail",
            provider::ExtraMailbox::Trash => "deleteditems",
        };

        let mut collected: Vec<MessageRef> = Vec::new();
        let mut next_url: Option<String> = None;
        loop {
            if collected.len() as u32 >= max_results {
                break;
            }
            let remaining = max_results - collected.len() as u32;
            let top = remaining.min(100);

            let url = if let Some(n) = next_url.as_ref() {
                n.clone()
            } else {
                let mut u = format!(
                    "{}/me/mailFolders/{}/messages?$top={}&$select=id,conversationId&$orderby=receivedDateTime desc",
                    self.base_url, folder, top
                );
                // Combine the incremental watermark (`receivedDateTime gt …`) and
                // the backfill upper bound (`receivedDateTime lt …`) into a
                // single `$filter` expression — Graph rejects multiple `$filter`
                // query params on the same request.
                let mut filter_clauses: Vec<String> = Vec::new();
                if let Some(ts) = after_timestamp {
                    filter_clauses.push(format!("receivedDateTime gt {}", unix_to_iso(ts)));
                }
                if let Some(ts) = before_timestamp {
                    filter_clauses.push(format!("receivedDateTime lt {}", unix_to_iso(ts)));
                }
                if !filter_clauses.is_empty() {
                    let filter = filter_clauses.join(" and ");
                    u.push_str(&format!("&$filter={}", urlencoding::encode(&filter)));
                }
                u
            };

            let response = self.send_get_with_retry(&url, "list mailbox messages").await?;
            if !response.status().is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(AppError::SyncError(format!(
                    "Failed to list {} folder: {}",
                    folder, body
                )));
            }
            let list: GraphMessageList = response.json().await?;
            for r in list.value {
                collected.push(MessageRef {
                    thread_id: r.conversation_id.unwrap_or_else(|| r.id.clone()),
                    id: r.id,
                });
                if collected.len() as u32 >= max_results {
                    break;
                }
            }
            match list.next_link {
                Some(link) => next_url = Some(link),
                None => break,
            }
        }
        Ok(collected)
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
    ) -> Result<crate::sync::provider::SentMessageMeta> {
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
    ) -> Result<crate::sync::provider::SentMessageMeta> {
        self.send_new_email(from_email, to_emails, cc_emails, subject, body, attachments)
            .await
    }

    async fn fetch_attachment_bytes(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        self.fetch_attachment_bytes(message_id, attachment_id).await
    }

    async fn create_draft(
        &self,
        _from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<String> {
        self.create_draft(to_emails, cc_emails, subject, body, attachments)
            .await
    }

    async fn update_draft(
        &self,
        provider_draft_id: &str,
        _from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<String> {
        self.update_draft(provider_draft_id, to_emails, cc_emails, subject, body, attachments)
            .await
    }

    async fn delete_draft(&self, provider_draft_id: &str) -> Result<()> {
        self.delete_draft(provider_draft_id).await
    }

    async fn list_drafts(&self) -> Result<Vec<crate::models::ProviderDraft>> {
        self.list_drafts().await
    }
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

fn parse_message(msg: GraphMessage) -> (Email, EmailCategory) {
    let (sender_name, sender_email) = recipient_name_email(msg.from.as_ref());
    let recipients = flatten_recipients(msg.to_recipients.as_deref());
    let cc = flatten_recipients(msg.cc_recipients.as_deref());

    let (body_html, snippet) = extract_body(msg.body.as_ref(), msg.body_preview.as_deref());

    // Map Outlook's focused-inbox signal to Gmail-style categories so the
    // existing UI filter chips ("Primary" / "Updates") remain meaningful.
    let category = match msg.inference_classification.as_deref() {
        Some("other") => EmailCategory::Updates,
        _ => EmailCategory::Primary,
    };

    let timestamp = msg
        .received_date_time
        .as_deref()
        .and_then(parse_iso_to_unix)
        .unwrap_or_else(|| chrono::Utc::now().timestamp());

    let email = Email {
        id: msg.id.clone(),
        account_id: String::new(), // set by caller
        thread_id: msg.conversation_id.unwrap_or_else(|| msg.id.clone()),
        message_id: msg.internet_message_id,
        subject: msg.subject.unwrap_or_default(),
        sender: sender_name,
        sender_email,
        recipients,
        cc,
        body: body_html,
        snippet,
        timestamp,
        is_read: msg.is_read.unwrap_or(false),
        triage_status: None,
        category: category.as_str().to_string(),
        // Caller (sync_folder) overrides per mailbox pass.
        mailbox: "inbox".to_string(),
        // Graph reports no per-message sent marker on this projection — a
        // message is sent iff it came from the Sent folder, which the insert
        // derives from the caller's `mailbox` value.
        is_sent: false,
    };

    (email, category)
}

fn recipient_name_email(from: Option<&GraphRecipientWrapper>) -> (String, String) {
    match from.and_then(|w| w.email_address.as_ref()) {
        Some(addr) => {
            let email = addr.address.clone().unwrap_or_default();
            let name = addr
                .name
                .clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| email.clone());
            (name, email)
        }
        None => (String::new(), String::new()),
    }
}

fn flatten_recipients(recipients: Option<&[GraphRecipientWrapper]>) -> Vec<String> {
    recipients
        .unwrap_or(&[])
        .iter()
        .filter_map(|w| w.email_address.as_ref())
        .filter_map(|addr| addr.address.clone())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Return (html_body, snippet). Graph returns either HTML or text — always
/// promote text to HTML so the frontend renderer (which assumes HTML) still
/// displays line breaks correctly.
fn extract_body(body: Option<&GraphBody>, preview: Option<&str>) -> (String, String) {
    let snippet = preview.unwrap_or("").to_string();
    let Some(body) = body else {
        return (String::new(), snippet);
    };
    let content = body.content.clone().unwrap_or_default();
    let html = match body.content_type.as_deref() {
        Some(ct) if ct.eq_ignore_ascii_case("html") => content,
        _ => plain_text_to_html(&content),
    };
    (html, snippet)
}

fn plain_text_to_html(text: &str) -> String {
    let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut out = String::with_capacity(escaped.len() + 64);
    out.push_str("<div style=\"white-space:pre-wrap;\">");
    for line in escaped.lines() {
        out.push_str(line);
        out.push_str("<br>");
    }
    out.push_str("</div>");
    out
}

/// Parse RFC 3339 / ISO 8601 (`2025-01-02T15:04:05Z`) into a Unix timestamp.
fn parse_iso_to_unix(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.timestamp())
}

/// Format a Unix timestamp as RFC 3339 / ISO 8601 UTC for Graph `$filter`.
fn unix_to_iso(ts: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

/// Build the Graph URL for one page of the inbox message list.
///
/// Scopes to the Inbox **folder** (`/me/mailFolders/inbox/messages`) rather
/// than the whole-mailbox `/me/messages` collection, for two reasons:
///   1. `/me/messages` spans every folder (Archive, Deleted, custom folders),
///      so a fresh sync pulled decade-old archived mail into the inbox view.
///   2. It lets us sort by `receivedDateTime` without filtering on
///      `parentFolderId`. Graph rejects `$orderby` combined with a `$filter`
///      on any property absent from the `$orderby` (error `InefficientFilter`),
///      which silently drops the sort order and interleaves ancient mail. The
///      only filter we attach here is `receivedDateTime`, matching the sort.
fn build_inbox_list_url(base: &str, top: u32, after_timestamp: Option<i64>, before_timestamp: Option<i64>) -> String {
    let top = top.min(1000); // Graph caps $top at 1000
    let mut url = format!(
        "{}/me/mailFolders/inbox/messages?$top={}&$select=id,conversationId&$orderby=receivedDateTime desc",
        base, top
    );
    let mut filters: Vec<String> = Vec::new();
    if let Some(ts) = after_timestamp {
        filters.push(format!("receivedDateTime ge {}", unix_to_iso(ts)));
    }
    if let Some(ts) = before_timestamp {
        filters.push(format!("receivedDateTime le {}", unix_to_iso(ts)));
    }
    if !filters.is_empty() {
        url.push_str(&format!("&$filter={}", urlencoding::encode(&filters.join(" and "))));
    }
    url
}

fn is_retryable_graph_status(status: StatusCode) -> bool {
    // 429 = throttled, 503 = service unavailable, 504 = gateway timeout,
    // 509 = bandwidth (rare). Graph docs also call out 500 occasionally but
    // retrying a real server error doesn't help and masks outages.
    matches!(status.as_u16(), 429 | 503 | 504 | 509)
}

fn is_retryable_transport_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|secs| secs.saturating_mul(1000))
}

fn format_graph_error(status: StatusCode, body: &str) -> String {
    let parsed: Option<GraphErrorEnvelope> = serde_json::from_str(body).ok();
    let message = parsed
        .and_then(|e| e.error)
        .and_then(|err| match (err.code, err.message) {
            (Some(code), Some(msg)) => Some(format!("{}: {}", code, msg)),
            (_, Some(msg)) => Some(msg),
            (Some(code), _) => Some(code),
            _ => None,
        });
    message
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| format!("HTTP {} {}", status.as_u16(), body))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_to_iso_formats_epoch() {
        assert_eq!(unix_to_iso(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn new_client_uses_production_graph_base_by_default() {
        // Guard against an accidental swap of the default URL — production
        // sync must keep hitting graph.microsoft.com.
        let c = OutlookClient::new("tok".into(), None, None, None);
        assert_eq!(c.base_url, GRAPH_API_BASE);
    }

    #[test]
    fn with_base_url_overrides_default_for_test_mock() {
        // Builder used by sync::mock::MockProviderServer to redirect HTTP at
        // a wiremock instance. If this stops working, every cassette-driven
        // test silently calls the real Graph API.
        let c = OutlookClient::new("tok".into(), None, None, None).with_base_url("http://127.0.0.1:9999");
        assert_eq!(c.base_url, "http://127.0.0.1:9999");
    }

    #[test]
    fn inbox_list_url_targets_inbox_folder_not_whole_mailbox() {
        // `/me/messages` spans every folder — including Archive, where decade-
        // old mail lives. A fresh inbox sync that queried it surfaced 2007
        // emails. Scope the inbox pass to the Inbox folder instead.
        let url = build_inbox_list_url(GRAPH_API_BASE, 100, Some(0), None);
        assert!(url.contains("/me/mailFolders/inbox/messages"), "got: {url}");
        assert!(
            !url.contains("/me/messages?"),
            "must not query the whole mailbox: {url}"
        );
    }

    #[test]
    fn inbox_list_url_filter_is_orderby_compatible() {
        // Graph rejects `$orderby=receivedDateTime` combined with a `$filter`
        // on any other property (error `InefficientFilter`), silently dropping
        // the sort order and interleaving ancient mail. The only filter allowed
        // alongside the receivedDateTime sort is receivedDateTime itself.
        let url = build_inbox_list_url(GRAPH_API_BASE, 100, Some(0), Some(1_700_000_000));
        assert!(url.contains("$orderby=receivedDateTime desc"), "orderby missing: {url}");
        assert!(
            !url.contains("parentFolderId"),
            "parentFolderId filter breaks the sort order: {url}"
        );
        // The receivedDateTime bound is present (url-encoded space → %20).
        assert!(url.contains("receivedDateTime%20ge"), "got: {url}");
    }

    #[test]
    fn inbox_list_url_omits_filter_when_unbounded() {
        let url = build_inbox_list_url(GRAPH_API_BASE, 50, None, None);
        assert!(!url.contains("$filter="), "no filter expected without bounds: {url}");
        assert!(url.contains("$orderby=receivedDateTime desc"), "got: {url}");
    }

    #[test]
    fn unix_to_iso_round_trips() {
        let ts = 1_700_000_000;
        let iso = unix_to_iso(ts);
        assert_eq!(parse_iso_to_unix(&iso), Some(ts));
    }

    #[test]
    fn parse_iso_handles_offset() {
        // Graph typically returns `Z` but we should still accept `+00:00`.
        assert_eq!(parse_iso_to_unix("2025-01-02T15:04:05+00:00"), Some(1_735_830_245));
    }

    #[test]
    fn plain_text_to_html_escapes_and_breaks() {
        let html = plain_text_to_html("line1\nline2 <script>\n");
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("line1<br>"));
        assert!(html.contains("line2"));
    }

    #[test]
    fn category_maps_focused_to_primary() {
        let msg = GraphMessage {
            id: "m1".to_string(),
            conversation_id: Some("c1".to_string()),
            internet_message_id: None,
            subject: Some("hi".to_string()),
            body_preview: None,
            body: None,
            from: None,
            to_recipients: None,
            cc_recipients: None,
            received_date_time: Some("2025-01-02T15:04:05Z".to_string()),
            is_read: Some(true),
            has_attachments: Some(false),
            inference_classification: Some("focused".to_string()),
        };
        let (email, cat) = parse_message(msg);
        assert_eq!(cat, EmailCategory::Primary);
        assert_eq!(email.category, "primary");
        assert_eq!(email.thread_id, "c1");
        assert!(email.is_read);
    }

    #[test]
    fn category_maps_other_to_updates() {
        let msg = GraphMessage {
            id: "m1".to_string(),
            conversation_id: None, // also tests thread_id fallback
            internet_message_id: None,
            subject: None,
            body_preview: Some("preview text".to_string()),
            body: Some(GraphBody {
                content_type: Some("text".to_string()),
                content: Some("hello\nworld".to_string()),
            }),
            from: Some(GraphRecipientWrapper {
                email_address: Some(GraphEmailAddress {
                    name: Some("Alice".to_string()),
                    address: Some("alice@example.com".to_string()),
                }),
            }),
            to_recipients: None,
            cc_recipients: None,
            received_date_time: None,
            is_read: None,
            has_attachments: None,
            inference_classification: Some("other".to_string()),
        };
        let (email, cat) = parse_message(msg);
        assert_eq!(cat, EmailCategory::Updates);
        assert_eq!(email.sender, "Alice");
        assert_eq!(email.sender_email, "alice@example.com");
        assert_eq!(email.thread_id, "m1", "falls back to id when conversationId missing");
        assert_eq!(email.snippet, "preview text");
        assert!(email.body.contains("hello<br>"));
        assert!(!email.is_read);
    }

    #[test]
    fn retry_after_parses_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Retry-After", "12".parse().unwrap());
        assert_eq!(retry_after_ms(&headers), Some(12_000));
    }

    #[test]
    fn retryable_statuses() {
        assert!(is_retryable_graph_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_graph_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_graph_status(StatusCode::GATEWAY_TIMEOUT));
        assert!(!is_retryable_graph_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_graph_status(StatusCode::FORBIDDEN));
        assert!(!is_retryable_graph_status(StatusCode::NOT_FOUND));
    }

    #[test]
    fn format_graph_error_extracts_code_message() {
        let body = r#"{"error":{"code":"InvalidAuthenticationToken","message":"Access token has expired"}}"#;
        let msg = format_graph_error(StatusCode::UNAUTHORIZED, body);
        assert!(msg.contains("InvalidAuthenticationToken"));
        assert!(msg.contains("expired"));
    }

    #[test]
    fn format_graph_error_falls_back_to_raw_body() {
        let msg = format_graph_error(StatusCode::BAD_GATEWAY, "nginx fail");
        assert!(msg.contains("502"));
        assert!(msg.contains("nginx fail"));
    }
}
