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

use crate::services::app_handle::AppHandle;
use async_trait::async_trait;
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use std::time::Duration;
#[cfg(feature = "desktop")]
use tauri::Emitter;
use tokio::time::sleep;

use crate::models::error::{AppError, Result};
use crate::models::{AppLogEvent, Email};
use crate::sync::http_retry::RetryPolicy;
use crate::sync::outlook_payload::{self, OutlookSendParams};
use crate::sync::outlook_upload::{self, AttachmentPlan, AttachmentRoute, EncodedContent, UPLOAD_CHUNK_SIZE};
use crate::sync::provider::{self, AttachmentInfo, EmailBody, EmailCategory, EmailProvider, MessageRef};

pub use crate::sync::provider::EmailAttachment;

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";
const MAX_RETRIES: u32 = 5;
const INITIAL_BACKOFF_MS: u64 = 1_000;
const MAX_BACKOFF_MS: u64 = 30_000;

/// Fields fetched on the single-message get so we can build a full `Email`
/// without a second round-trip. List queries use their own, much narrower
/// projection ([`GraphMessageRef`]).
///
/// `internetMessageHeaders` is the only way to get RFC 5322 headers out of
/// Graph, and Graph omits the property entirely unless it is explicitly
/// selected — unlike Gmail and IMAP, where the headers already arrive with the
/// message. It is requested only on the single-message get (this constant's one
/// caller), never on list queries, because it is bulky.
///
/// Graph caps the returned header list (~256 entries) and some tenants omit it
/// altogether. That is why an absent header set must degrade to `Unknown`
/// rather than to a confident `Clean`.
/// Graph caps one `$batch` at 20 sub-requests, which is also the sync loop's
/// chunk size — kept as its own constant so the two can move independently.
const GRAPH_BATCH_LIMIT: usize = 20;

const MESSAGE_SELECT_FIELDS: &str = "id,conversationId,internetMessageId,subject,bodyPreview,\
    body,from,toRecipients,ccRecipients,receivedDateTime,isRead,hasAttachments,inferenceClassification,\
    internetMessageHeaders";

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
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
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
    /// Only present when `internetMessageHeaders` is in `$select`, and even
    /// then some tenants withhold it.
    #[serde(rename = "internetMessageHeaders")]
    internet_message_headers: Option<Vec<GraphHeader>>,
}

#[derive(Debug, Deserialize)]
struct GraphHeader {
    name: Option<String>,
    value: Option<String>,
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
            client: crate::sync::http_client::provider_http_client(crate::sync::http_client::MAIL_REQUEST_TIMEOUT),
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
        self.finalize_message(msg).await
    }

    /// Turn a fetched `GraphMessage` into a stored email: attachments, parse,
    /// and inline-image substitution. Shared by the single-message path and the
    /// `$batch` path so both produce identical rows.
    async fn finalize_message(&self, msg: GraphMessage) -> Result<(Email, EmailCategory, Vec<AttachmentInfo>)> {
        // Only skip the attachments call when Graph explicitly reports `false`.
        // Some Outlook mailboxes return `hasAttachments` as `null` even when
        // attachments exist; defaulting that to "no attachments" silently drops
        // them from the UI. `list_attachments` is cheap and returns empty when
        // there really are none.
        let (attachments, inline_images) = match msg.has_attachments {
            Some(false) => (Vec::new(), Vec::new()),
            _ => match self.list_attachments(&msg.id).await {
                Ok(found) => found,
                // Non-fatal for the message, but never silent: an error here
                // stores the email with no attachment rows at all.
                Err(e) => {
                    self.log(&format!("Graph: could not list attachments for {}: {e}", msg.id));
                    (Vec::new(), Vec::new())
                }
            },
        };

        let (mut email, category) = parse_message(msg);

        // Replace `cid:<id>` references in the HTML body with data URIs so
        // inline images render in the WebView. Outlook's HTML bodies reference
        // attached images by content-id (e.g. <img src="cid:abc"/>); without
        // this substitution they show as broken images.
        if !inline_images.is_empty() && email.body.contains("cid:") {
            for (cid, mime, b64) in &inline_images {
                let data_uri = format!("data:{};base64,{}", mime, b64);
                email.body = crate::util::html::replace_cid_reference(&email.body, cid, &data_uri);
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
        item_id: &str,
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<crate::sync::provider::SentMessageMeta> {
        // Graph's `/reply` endpoint auto-preserves subject prefix, In-Reply-To,
        // References, and (with the `comment` form) the quoted thread history.
        //
        // It addresses the parent by *item id* — the opaque `AQMkAD…` handle we
        // store as `emails.id`. Passing `internetMessageId` (`<abc@host>`) here
        // is not a slower path, it is a different resource: Graph rejects it,
        // and it is `NULL` on many rows besides, which turned a perfectly
        // replyable message into a hard error before it reached the network.
        let item_id = item_id.trim();
        if item_id.is_empty() {
            return Err(AppError::InvalidInput(
                "send_reply requires the Graph item id of the message being answered".to_string(),
            ));
        }

        let outgoing = outgoing_attachments(body, attachments);
        let plan = plan_outgoing(&outgoing)?;
        if !plan.single_request {
            // Too much for one request: a reply draft, filled, then sent.
            let bare = body_without_inline_images(body);
            let payload = outlook_payload::build_reply_payload(&OutlookSendParams {
                to_emails,
                cc_emails,
                subject,
                body: &bare,
                attachments: &[],
            });
            let url = format!(
                "{}/me/messages/{}/createReply",
                self.base_url,
                urlencoding::encode(item_id)
            );
            let draft_id = self.create_message(&url, &payload, "create reply draft").await?;
            return self.fill_and_send(&draft_id, &outgoing, &plan).await;
        }

        let payload =
            crate::sync::outlook_payload::build_reply_payload(&crate::sync::outlook_payload::OutlookSendParams {
                to_emails,
                cc_emails,
                subject,
                body,
                attachments,
            });
        let url = format!("{}/me/messages/{}/reply", self.base_url, urlencoding::encode(item_id),);
        let response = self.send_post_json_no_resend(&url, &payload, "send reply").await?;
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
        let outgoing = outgoing_attachments(body, attachments);
        let plan = plan_outgoing(&outgoing)?;
        if !plan.single_request {
            // Too much for one request: a draft, filled, then sent.
            let draft_id = self
                .create_bare_draft(to_emails, cc_emails, subject, body, "create draft to send")
                .await?;
            return self.fill_and_send(&draft_id, &outgoing, &plan).await;
        }

        let payload =
            crate::sync::outlook_payload::build_send_mail_payload(&crate::sync::outlook_payload::OutlookSendParams {
                to_emails,
                cc_emails,
                subject,
                body,
                attachments,
            });

        let url = format!("{}/me/sendMail", self.base_url);
        let response = self.send_post_json_no_resend(&url, &payload, "send new email").await?;
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
        let outgoing = outgoing_attachments(body, attachments);
        let plan = plan_outgoing(&outgoing)?;
        if !plan.single_request {
            let draft_id = self
                .create_bare_draft(to_emails, cc_emails, subject, body, "create draft")
                .await?;
            // A draft that could not be filled is not recorded by the caller,
            // which would create another one on the next save: remove it.
            if let Err(e) = self.add_attachments(&draft_id, &outgoing, &plan).await {
                self.discard_draft(&draft_id).await;
                return Err(e);
            }
            return Ok(draft_id);
        }

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
        let response = self.send_post_json_no_resend(&url, &payload, "create draft").await?;
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
        let outgoing = outgoing_attachments(body, attachments);
        let plan = plan_outgoing(&outgoing)?;
        // Too much for one request: the text goes in the update, and the
        // draft's attachments are replaced one by one after it.
        let bare = (!plan.single_request).then(|| body_without_inline_images(body));
        let (body, attachments) = match &bare {
            Some(bare) => (bare, &[][..]),
            None => (body, attachments),
        };
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
        if !plan.single_request {
            self.remove_attachments(&msg.id).await?;
            self.add_attachments(&msg.id, &outgoing, &plan).await?;
        }
        Ok(msg.id)
    }

    // ── Attachments that do not fit in one request ───────────────────────────
    //
    // Graph takes an attachment inline only under 3 MB and a request only up to
    // about 4 MB (see `outlook_upload`). Past that the message has to exist as
    // a draft first; each attachment is then added to it on its own — a small
    // one with one POST, a large one through an upload session — and the draft
    // is sent.

    /// `POST` a message payload that creates a draft and return the draft's id.
    /// Not re-sent on a 5xx: it may have created the draft already.
    async fn create_message(&self, url: &str, payload: &serde_json::Value, operation: &str) -> Result<String> {
        let response = self.send_post_json_no_resend(url, payload, operation).await?;
        let msg: GraphMessage = response.json().await?;
        Ok(msg.id)
    }

    /// Create a draft carrying the message without any attachment.
    async fn create_bare_draft(
        &self,
        to_emails: &[String],
        cc_emails: &[String],
        subject: &str,
        body: &EmailBody,
        operation: &str,
    ) -> Result<String> {
        let bare = body_without_inline_images(body);
        let payload = outlook_payload::build_draft_payload(&OutlookSendParams {
            to_emails,
            cc_emails,
            subject,
            body: &bare,
            attachments: &[],
        });
        self.create_message(&format!("{}/me/messages", self.base_url), &payload, operation)
            .await
    }

    /// Add the attachments to a draft and send it.
    ///
    /// - An attachment that cannot be added: the draft is removed, nothing was
    ///   sent.
    /// - A send that fails: the draft is **kept**. A 5xx or a dropped
    ///   connection may have sent it all the same (which is why the send is
    ///   never repeated), and deleting it then could take a message out of the
    ///   Outbox; when it really was not sent, the draft still holds the
    ///   uploaded attachments and can be sent from Outlook.
    async fn fill_and_send(
        &self,
        draft_id: &str,
        outgoing: &[OutgoingAttachment<'_>],
        plan: &AttachmentPlan,
    ) -> Result<crate::sync::provider::SentMessageMeta> {
        if let Err(e) = self.add_attachments(draft_id, outgoing, plan).await {
            self.discard_draft(draft_id).await;
            return Err(e);
        }
        let url = format!("{}/me/messages/{}/send", self.base_url, urlencoding::encode(draft_id));
        let sent = self
            .send_request_with_policy("send message", RetryPolicy::NoRetryAfterSend, |client, token| {
                client
                    .post(&url)
                    .bearer_auth(token)
                    .header(reqwest::header::CONTENT_LENGTH, 0)
            })
            .await;
        match sent {
            // 202 Accepted with no body, like /sendMail: no meta available.
            Ok(_) => Ok(crate::sync::provider::SentMessageMeta::default()),
            Err(e) => {
                let message = format!(
                    "{}. The message and its attachments were left in the account's Drafts folder; \
                     check Sent before sending it again",
                    error_detail(&e)
                );
                crate::services::logger::log("error", "sync", message.clone());
                Err(AppError::SyncError(message))
            }
        }
    }

    /// Add each attachment to an existing draft by the route planned for it.
    async fn add_attachments(
        &self,
        draft_id: &str,
        outgoing: &[OutgoingAttachment<'_>],
        plan: &AttachmentPlan,
    ) -> Result<()> {
        for (item, route) in outgoing.iter().zip(&plan.routes) {
            match route {
                AttachmentRoute::Inline => {
                    let url = format!(
                        "{}/me/messages/{}/attachments",
                        self.base_url,
                        urlencoding::encode(draft_id)
                    );
                    let payload = outlook_payload::build_attachment_json(item.attachment, item.force_inline);
                    // Not re-sent on a 5xx: a second copy would be attached.
                    self.send_post_json_no_resend(&url, &payload, "add attachment").await?;
                }
                AttachmentRoute::UploadSession => self.upload_in_session(draft_id, item).await?,
            }
        }
        Ok(())
    }

    /// Delete every attachment a draft has at the provider.
    async fn remove_attachments(&self, draft_id: &str) -> Result<()> {
        let list_url = format!(
            "{}/me/messages/{}/attachments?$select=id",
            self.base_url,
            urlencoding::encode(draft_id)
        );
        let response = self.send_get_with_retry(&list_url, "list draft attachments").await?;
        let listed: GraphAttachmentIds = response.json().await?;
        for attachment in listed.value {
            let url = format!(
                "{}/me/messages/{}/attachments/{}",
                self.base_url,
                urlencoding::encode(draft_id),
                urlencoding::encode(&attachment.id)
            );
            match self
                .send_request_with_retry("delete draft attachment", |client, token| {
                    client.delete(&url).bearer_auth(token)
                })
                .await
            {
                // Already gone: that is the state asked for.
                Ok(_) | Err(AppError::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Remove a draft this client created and could not complete. Best effort:
    /// the failure that led here is the one reported, and a draft that could
    /// not be removed is logged.
    async fn discard_draft(&self, draft_id: &str) {
        match self.delete_draft(draft_id).await {
            Ok(()) | Err(AppError::NotFound(_)) => {}
            Err(e) => crate::services::logger::log(
                "error",
                "sync",
                format!("An incomplete draft could not be removed from Outlook's Drafts folder: {e}"),
            ),
        }
    }

    /// Attach one file to a draft through an upload session: create the
    /// session, then `PUT` the content range by range to its pre-authenticated
    /// URL. A session that cannot be completed is cancelled.
    async fn upload_in_session(&self, draft_id: &str, item: &OutgoingAttachment<'_>) -> Result<()> {
        use crate::services::logger::log;

        let name = item.attachment.filename.as_str();
        let content = EncodedContent::new(&item.attachment.data);
        log(
            "info",
            "sync",
            format!(
                "Uploading attachment \"{name}\" ({:.1} MB) to Outlook",
                megabytes(content.len())
            ),
        );

        let session_url = format!(
            "{}/me/messages/{}/attachments/createUploadSession",
            self.base_url,
            urlencoding::encode(draft_id)
        );
        let payload = outlook_upload::upload_session_payload(item.attachment, content.len(), item.force_inline);
        let uploaded = async {
            // Safe to repeat: a session nothing was uploaded to simply expires.
            let response = self
                .send_post_json_with_retry(&session_url, &payload, "create upload session")
                .await?;
            let session: GraphUploadSession = response.json().await?;
            self.check_upload_url(&session.upload_url)?;
            if let Err(e) = self.put_ranges(&session.upload_url, &content, name).await {
                self.cancel_upload_session(&session.upload_url).await;
                return Err(e);
            }
            Ok(())
        }
        .await;

        match uploaded {
            Ok(()) => {
                log("success", "sync", format!("Uploaded attachment \"{name}\" to Outlook"));
                Ok(())
            }
            Err(e) => {
                let message = format!(
                    "Could not upload attachment \"{name}\" to Outlook: {}",
                    error_detail(&e)
                );
                log("error", "sync", message.clone());
                Err(match e {
                    AppError::InvalidInput(_) => AppError::InvalidInput(message),
                    _ => AppError::SyncError(message),
                })
            }
        }
    }

    /// The upload URL comes from Graph and receives the file's bytes: it has
    /// to be HTTPS. (A test server's own origin is accepted too.)
    fn check_upload_url(&self, upload_url: &str) -> Result<()> {
        let refuse = || AppError::SyncError("Graph returned an upload URL that cannot be used".to_string());
        let url = reqwest::Url::parse(upload_url).map_err(|_| refuse())?;
        let same_origin_as_api = reqwest::Url::parse(&self.base_url)
            .map(|base| base.origin() == url.origin())
            .unwrap_or(false);
        if url.scheme() == "https" || same_origin_as_api {
            Ok(())
        } else {
            Err(refuse())
        }
    }

    /// `PUT` the content to an upload session, [`UPLOAD_CHUNK_SIZE`] bytes at a
    /// time and in order, without an `Authorization` header (the URL carries
    /// its own token). Graph answers 200 with where it expects the upload to
    /// continue, and 201 once the last byte is in. A range that fails with a
    /// retryable status, a transport error, or an answer that does not move
    /// the upload forward is sent again — a range is idempotent — up to
    /// [`MAX_RETRIES`] times in a row.
    async fn put_ranges(&self, upload_url: &str, content: &EncodedContent<'_>, name: &str) -> Result<()> {
        let total = content.len();
        let mut offset: u64 = 0;
        let mut failures: u32 = 0;
        let mut delay_ms = INITIAL_BACKOFF_MS;
        loop {
            let end = (offset + UPLOAD_CHUNK_SIZE).min(total);
            let bytes = content.range(offset, end)?;
            let response = self
                .client
                .put(upload_url)
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .header(
                    reqwest::header::CONTENT_RANGE,
                    outlook_upload::content_range(offset, end, total),
                )
                .body(bytes)
                .send()
                .await;

            let problem = match response {
                Ok(response) if response.status() == StatusCode::CREATED => return Ok(()),
                Ok(response) if response.status().is_success() => {
                    let progress: GraphUploadProgress = response.json().await?;
                    let next = outlook_upload::next_offset(&progress.next_expected_ranges).unwrap_or(end);
                    if next > offset && next < total {
                        offset = next;
                        failures = 0;
                        delay_ms = INITIAL_BACKOFF_MS;
                        crate::services::logger::log(
                            "debug",
                            "sync",
                            format!(
                                "Uploaded {:.1} of {:.1} MB of \"{name}\"",
                                megabytes(offset),
                                megabytes(total)
                            ),
                        );
                        continue;
                    }
                    format!(
                        "Graph expects byte {next} next, after bytes {offset}-{} of {total}",
                        end - 1
                    )
                }
                Ok(response) => {
                    let status = response.status();
                    let retry_after = retry_after_ms(response.headers());
                    let body = response.text().await.unwrap_or_default();
                    let problem = format_graph_error(status, &body);
                    if !is_retryable_graph_status(status) {
                        return Err(AppError::SyncError(problem));
                    }
                    if let Some(wait_ms) = retry_after {
                        delay_ms = wait_ms;
                    }
                    problem
                }
                Err(error) if is_retryable_transport_error(&error) => error.to_string(),
                Err(error) => return Err(error.into()),
            };

            failures += 1;
            if failures > MAX_RETRIES {
                return Err(AppError::SyncError(format!("{problem} (after {MAX_RETRIES} retries)")));
            }
            crate::services::logger::log(
                "debug",
                "sync",
                format!("Retrying a part of \"{name}\" ({failures}/{MAX_RETRIES}): {problem}"),
            );
            sleep(Duration::from_millis(delay_ms.min(MAX_BACKOFF_MS))).await;
            delay_ms = (delay_ms * 2).min(MAX_BACKOFF_MS);
        }
    }

    /// Cancel an upload session that will not be completed. Best effort: an
    /// abandoned session expires by itself.
    async fn cancel_upload_session(&self, upload_url: &str) {
        let cancelled = self.client.delete(upload_url).send().await;
        let failure = match cancelled {
            Ok(response) if response.status().is_success() => return,
            Ok(response) => format!("HTTP {}", response.status().as_u16()),
            Err(e) => e.to_string(),
        };
        crate::services::logger::log(
            "debug",
            "sync",
            format!("An abandoned Outlook upload session could not be cancelled and will expire: {failure}"),
        );
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
        // Enumerate the whole Drafts folder. This must be exhaustive: the ids
        // become the keep-list for `prune_provider_drafts`, so stopping at the
        // first page would delete every draft past it on each sync.
        let mut next_url = Some(format!("{}/me/mailFolders/drafts/messages?$top=100", self.base_url));
        let mut out = Vec::new();
        let mut pages = 0usize;
        while let Some(url) = next_url.take() {
            // Bail rather than truncate: a partial list would make the prune
            // pass delete every draft it never got to see.
            pages += 1;
            if pages > crate::sync::draft_plan::MAX_DRAFT_PAGES {
                return Err(AppError::SyncError(format!(
                    "Outlook drafts listing returned too many pages (over {}); refusing a partial list.",
                    crate::sync::draft_plan::MAX_DRAFT_PAGES
                )));
            }
            let response = self.send_get_with_retry(&url, "list drafts").await?;
            let list: GraphMessageDraftList = response.json().await?;
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
                    // Graph returns full draft bodies in the listing itself, so
                    // there is no per-draft read to skip and no token to track.
                    provider_message_id: None,
                    in_reply_to: None,
                });
            }
            // `@odata.nextLink` is a complete URL carrying $skiptoken state —
            // follow it verbatim rather than rebuilding the query.
            next_url = list.next_link.filter(|link| !link.is_empty());
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

    /// POST for sends and creates: a 5xx or a dropped connection may have
    /// been carried out, so it is not re-sent (see [`RetryPolicy`]).
    async fn send_post_json_no_resend(
        &self,
        url: &str,
        payload: &serde_json::Value,
        operation: &str,
    ) -> Result<Response> {
        self.send_request_with_policy(operation, RetryPolicy::NoRetryAfterSend, |client, token| {
            client.post(url).bearer_auth(token).json(payload)
        })
        .await
    }

    async fn send_request_with_retry<F>(&self, operation: &str, request_builder: F) -> Result<Response>
    where
        F: Fn(&Client, &str) -> reqwest::RequestBuilder,
    {
        self.send_request_with_policy(operation, RetryPolicy::Idempotent, request_builder)
            .await
    }

    async fn send_request_with_policy<F>(
        &self,
        operation: &str,
        policy: RetryPolicy,
        request_builder: F,
    ) -> Result<Response>
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
                    // Typed so callers can tell "the resource is gone" from a
                    // failure — a draft push re-creates a draft that was sent
                    // or deleted elsewhere. Same message text as every other
                    // failure.
                    if status == StatusCode::NOT_FOUND {
                        return Err(AppError::NotFound(format!(
                            "Failed to {}: {}",
                            operation,
                            format_graph_error(status, &body)
                        )));
                    }
                    let should_retry = is_retryable_graph_status(status) && policy.may_retry_status(status.as_u16());

                    if should_retry && attempt < MAX_RETRIES {
                        let wait_ms = retry_after.unwrap_or(delay_ms).min(MAX_BACKOFF_MS);
                        self.emit_retry_log(operation, attempt + 1, wait_ms, status);
                        sleep(Duration::from_millis(wait_ms)).await;
                        delay_ms = (delay_ms * 2).min(MAX_BACKOFF_MS);
                        continue;
                    }

                    let message = format!("Failed to {}: {}", operation, format_graph_error(status, &body));
                    // Typed so callers can tell "Graph no longer has this id"
                    // (a moved or deleted message) from an outage.
                    return Err(if status == StatusCode::NOT_FOUND {
                        AppError::NotFound(message)
                    } else {
                        AppError::SyncError(message)
                    });
                }
                Err(error) => {
                    if is_retryable_transport_error(&error)
                        && policy.may_retry_transport(error.is_connect())
                        && attempt < MAX_RETRIES
                    {
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

    async fn list_message_ids_with_attachments(&self) -> Result<Option<Vec<String>>> {
        let mut ids = Vec::new();
        let mut url = Some(build_attachments_list_url(&self.base_url));
        while let Some(current) = url {
            let response = self
                .send_get_with_retry(&current, "list messages with attachments")
                .await?;
            if !response.status().is_success() {
                let error_text = response.text().await.unwrap_or_default();
                return Err(AppError::SyncError(format!(
                    "Failed to list messages with attachments: {error_text}"
                )));
            }
            let list: GraphMessageList = response.json().await?;
            ids.extend(list.value.into_iter().map(|r| r.id));
            // `@odata.nextLink` is a complete URL carrying the $skiptoken.
            url = list.next_link.filter(|link| !link.is_empty());
        }
        Ok(Some(ids))
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

    /// Fetch a chunk of messages with **one** Graph `$batch` request.
    ///
    /// The trait default calls [`Self::get_message`] per ID — twenty sequential
    /// HTTPS round trips for a twenty-message chunk. Graph's `$batch` endpoint
    /// takes up to [`GRAPH_BATCH_LIMIT`] sub-requests in a single POST, so the
    /// same chunk costs one round trip plus (only for messages that have them)
    /// the per-message attachment call.
    ///
    /// Per-message throttling is normal inside a batch: Graph answers those
    /// sub-requests with 429 while the rest succeed. Those slots are retried on
    /// their own with the same exponential backoff the single-message path
    /// uses, rather than re-fetching the whole chunk.
    async fn batch_get_messages(
        &self,
        message_ids: &[&str],
    ) -> Result<Vec<Result<(Email, provider::EmailCategory, Vec<provider::AttachmentInfo>)>>> {
        type ProviderResult = Result<(Email, provider::EmailCategory, Vec<provider::AttachmentInfo>)>;

        let mut slots: Vec<Option<ProviderResult>> = (0..message_ids.len()).map(|_| None).collect();

        for (chunk_index, chunk) in message_ids.chunks(GRAPH_BATCH_LIMIT).enumerate() {
            let offset = chunk_index * GRAPH_BATCH_LIMIT;
            // Slots of this chunk still waiting for an answer, as indices into
            // `chunk`. Shrinks as sub-responses land; whatever is left after
            // the retries becomes a per-message error.
            let mut pending: Vec<usize> = (0..chunk.len()).collect();
            let mut delay_ms = INITIAL_BACKOFF_MS;

            for attempt in 0..=MAX_RETRIES {
                if pending.is_empty() {
                    break;
                }

                let ids: Vec<&str> = pending.iter().map(|i| chunk[*i]).collect();
                let payload = build_message_batch_payload(&ids);
                let url = format!("{}/$batch", self.base_url);
                let response = self
                    .send_post_json_with_retry(&url, &payload, "batch get messages")
                    .await?;
                let envelope: serde_json::Value = response.json().await?;

                let mut still_pending: Vec<usize> = Vec::new();
                let mut answered = vec![false; pending.len()];

                for sub in parse_batch_response(&envelope) {
                    let Some(chunk_slot) = pending.get(sub.index).copied() else {
                        continue; // an id outside the request we sent
                    };
                    answered[sub.index] = true;

                    if sub.status == 200 {
                        let parsed = match sub.body {
                            Some(body) => match serde_json::from_value::<GraphMessage>(body) {
                                Ok(msg) => self.finalize_message(msg).await,
                                Err(e) => Err(AppError::SyncError(format!("Malformed message in $batch: {e}"))),
                            },
                            None => Err(AppError::SyncError("Batch sub-response had no body".to_string())),
                        };
                        slots[offset + chunk_slot] = Some(parsed);
                    } else if StatusCode::from_u16(sub.status).is_ok_and(is_retryable_graph_status)
                        && attempt < MAX_RETRIES
                    {
                        still_pending.push(chunk_slot);
                    } else {
                        slots[offset + chunk_slot] = Some(Err(AppError::SyncError(format!(
                            "Batch sub-request failed with HTTP {}",
                            sub.status
                        ))));
                    }
                }

                // A slot Graph never answered: treat it like a retryable gap
                // rather than silently dropping the message.
                for (position, chunk_slot) in pending.iter().enumerate() {
                    if !answered[position] {
                        still_pending.push(*chunk_slot);
                    }
                }

                pending = still_pending;
                if !pending.is_empty() && attempt < MAX_RETRIES {
                    sleep(Duration::from_millis(delay_ms.min(MAX_BACKOFF_MS))).await;
                    delay_ms = (delay_ms * 2).min(MAX_BACKOFF_MS);
                }
            }

            for chunk_slot in pending {
                slots[offset + chunk_slot] = Some(Err(AppError::SyncError(
                    "Batch sub-request still throttled after retries".to_string(),
                )));
            }
        }

        Ok(slots
            .into_iter()
            .enumerate()
            .map(|(index, slot)| {
                slot.unwrap_or_else(|| {
                    Err(AppError::SyncError(format!(
                        "Batch returned no result for {}",
                        message_ids[index]
                    )))
                })
            })
            .collect())
    }

    /// Find a message the app already stores. Graph re-keys a message when it
    /// is moved between folders, so the stored id stops resolving and the
    /// Message-ID header is the only handle that survives the move. Checks the
    /// folders a message can reach from Junk, and reports the id it now has.
    async fn locate_message(
        &self,
        _message_id: &str,
        message_id_header: Option<&str>,
    ) -> Result<Option<provider::MessageLocation>> {
        let Some(header) = message_id_header.map(str::trim).filter(|h| !h.is_empty()) else {
            return Ok(None);
        };
        // OData escapes a single quote inside a string literal by doubling it.
        let escaped = header.replace('\'', "''");
        // `archive` maps to inbox: EmailOps has no archive mailbox, and Gmail's
        // archived mail already lands there.
        for (folder, mailbox) in [
            ("inbox", "inbox"),
            ("deleteditems", "trash"),
            ("archive", "inbox"),
            ("junkemail", "spam"),
        ] {
            let filter = format!("internetMessageId eq '{escaped}'");
            let url = format!(
                "{}/me/mailFolders/{}/messages?$top=1&$select=id,conversationId&$filter={}",
                self.base_url,
                folder,
                urlencoding::encode(&filter)
            );
            let response = self.send_get_with_retry(&url, "locate message").await?;
            if !response.status().is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(AppError::SyncError(format!(
                    "Failed to locate message in {folder}: {body}"
                )));
            }
            let list: GraphMessageList = response.json().await?;
            if let Some(found) = list.value.into_iter().next() {
                return Ok(Some(provider::MessageLocation {
                    id: found.id,
                    mailbox: mailbox.to_string(),
                }));
            }
        }
        Ok(None)
    }

    /// One `$batch` of `GET /me/messages/{id}?$select=id,isRead` per
    /// [`GRAPH_BATCH_LIMIT`] ids. Asking by id rather than listing folders
    /// needs no "was the listing complete?" reasoning: every answer is about
    /// exactly one stored message.
    async fn fetch_message_states(
        &self,
        message_ids: &[String],
    ) -> Result<Option<std::collections::HashMap<String, provider::RemoteMessageState>>> {
        let mut states = std::collections::HashMap::with_capacity(message_ids.len());
        let url = format!("{}/$batch", self.base_url);
        for chunk in message_ids.chunks(GRAPH_BATCH_LIMIT) {
            let ids: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let payload = build_batch_payload(&ids, "id,isRead");
            let response = self
                .send_post_json_with_retry(&url, &payload, "refresh message states")
                .await?;
            let envelope: serde_json::Value = response.json().await?;
            states.extend(states_from_batch(chunk, &envelope));
        }
        Ok(Some(states))
    }

    /// `PATCH /me/messages/{id}` with `isRead` (`Mail.ReadWrite`). Setting
    /// the same value twice is a no-op at Graph, so it is safe to retry.
    async fn set_read_state(&self, message_id: &str, read: bool) -> Result<()> {
        let url = format!("{}/me/messages/{}", self.base_url, urlencoding::encode(message_id));
        let payload = serde_json::json!({ "isRead": read });
        self.send_request_with_retry("set read state", |client, token| {
            client.patch(&url).bearer_auth(token).json(&payload)
        })
        .await?;
        Ok(())
    }

    /// Move the message to Deleted Items (`Mail.ReadWrite`). Deliberately not
    /// `DELETE /me/messages/{id}`: the app's delete action is the reversible
    /// one, so the message stays recoverable from the account's own clients.
    async fn trash_message(&self, message_id: &str, _message_id_header: Option<&str>) -> Result<()> {
        let url = format!("{}/me/messages/{}/move", self.base_url, urlencoding::encode(message_id));
        let payload = serde_json::json!({ "destinationId": "deleteditems" });
        self.send_post_json_no_resend(&url, &payload, "trash message").await?;
        Ok(())
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
        _from_name: Option<&str>,
        to_emails: &[String],
        cc_emails: &[String],
        // Graph writes In-Reply-To/References itself from the message being
        // replied to, so the parent's RFC headers are not ours to send; only
        // the item id matters here.
        target: &crate::sync::provider::ReplyTarget<'_>,
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<crate::sync::provider::SentMessageMeta> {
        self.send_reply(
            from_email,
            to_emails,
            cc_emails,
            target.provider_message_id,
            subject,
            body,
            attachments,
        )
        .await
    }

    async fn send_new_email(
        &self,
        from_email: &str,
        _from_name: Option<&str>,
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
        // Graph has no way to set In-Reply-To on a plain draft; a threaded
        // draft needs `createReply`, which is not wired yet. Outlook reply
        // drafts are therefore linked to their thread on this device only.
        _reply: Option<&crate::sync::provider::ReplyTarget<'_>>,
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
        _reply: Option<&crate::sync::provider::ReplyTarget<'_>>,
    ) -> Result<String> {
        self.update_draft(provider_draft_id, to_emails, cc_emails, subject, body, attachments)
            .await
    }

    async fn delete_draft(&self, provider_draft_id: &str) -> Result<()> {
        self.delete_draft(provider_draft_id).await
    }

    async fn list_drafts(
        &self,
        _known_change_tokens: &std::collections::HashMap<String, String>,
    ) -> Result<crate::sync::draft_plan::ProviderDraftPull> {
        // Graph hands back full draft bodies in one listing call, so there is
        // no N+1 to avoid — every listed draft is reported as changed.
        let drafts = self.list_drafts().await?;
        let present_ids = drafts.iter().map(|d| d.provider_draft_id.clone()).collect();
        Ok(crate::sync::draft_plan::ProviderDraftPull {
            changed: drafts,
            present_ids,
        })
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

    // `None` (property withheld, or fetched before it was selected) must stay
    // distinguishable from "captured and empty": the detector treats the former
    // as Unknown, not as Clean.
    let headers = msg.internet_message_headers.as_ref().map(|list| {
        let pairs: Vec<(String, String)> = list
            .iter()
            .filter_map(|h| Some((h.name.clone()?, h.value.clone().unwrap_or_default())))
            .collect();
        crate::sync::header_capture::capture(&pairs)
    });

    // Only present when `internetMessageHeaders` was selected. Graph writes the
    // threading headers itself on `/reply`, so this is stored for completeness
    // (and for any future non-Graph send path) rather than read back today.
    let references = msg.internet_message_headers.as_ref().and_then(|list| {
        list.iter()
            .find(|h| h.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case("References")))
            .and_then(|h| h.value.clone())
    });

    let email = Email {
        id: msg.id.clone(),
        account_id: String::new(), // set by caller
        thread_id: msg.conversation_id.unwrap_or_else(|| msg.id.clone()),
        message_id: msg.internet_message_id,
        references,
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
        headers,
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

/// Every message with attachments, in any folder, ids only — the backfill
/// candidates. `hasAttachments` is filterable without an `$orderby`.
fn build_attachments_list_url(base: &str) -> String {
    format!(
        "{}/me/messages?$top=1000&$select=id&$filter={}",
        base,
        urlencoding::encode("hasAttachments eq true")
    )
}

/// One sub-response of a `$batch`, resolved back to the slot it answers.
#[derive(Debug)]
struct GraphBatchSubResponse {
    /// Index in the caller's `message_ids` slice.
    index: usize,
    status: u16,
    /// `None` for a non-200: Graph puts an error object there, not a message.
    body: Option<serde_json::Value>,
}

/// Build the JSON body for a `$batch` of message GETs.
///
/// Each sub-request is identified by its **index** in `message_ids`, because
/// Graph does not promise to answer in request order — the id is how a body
/// finds its way back to the right slot. The `$select` projection matches the
/// single-message path so both produce the same `GraphMessage`.
fn build_message_batch_payload(message_ids: &[&str]) -> serde_json::Value {
    build_batch_payload(message_ids, MESSAGE_SELECT_FIELDS)
}

/// A `$batch` of message GETs projecting `select`, one sub-request per id.
fn build_batch_payload(message_ids: &[&str], select: &str) -> serde_json::Value {
    let requests: Vec<serde_json::Value> = message_ids
        .iter()
        .enumerate()
        .map(|(index, message_id)| {
            serde_json::json!({
                "id": index.to_string(),
                "method": "GET",
                "url": format!(
                    "/me/messages/{}?$select={}",
                    urlencoding::encode(message_id),
                    select
                ),
            })
        })
        .collect();
    serde_json::json!({ "requests": requests })
}

/// Read one chunk's answers out of a state-refresh `$batch`: 200 is the
/// message's read flag, 404 means Graph no longer has that id (deleted, or
/// moved and re-keyed). Any other status — a throttled sub-request above all —
/// says nothing about the message and is left out.
fn states_from_batch(chunk: &[String], envelope: &serde_json::Value) -> Vec<(String, provider::RemoteMessageState)> {
    parse_batch_response(envelope)
        .into_iter()
        .filter_map(|sub| {
            let id = chunk.get(sub.index)?;
            let state = match sub.status {
                200 => provider::RemoteMessageState::Present {
                    is_read: sub.body.as_ref()?.get("isRead")?.as_bool()?,
                },
                404 => provider::RemoteMessageState::Missing,
                _ => return None,
            };
            Some((id.clone(), state))
        })
        .collect()
}

/// Pull the sub-responses out of a `$batch` envelope.
///
/// Anything that does not match the documented shape is skipped rather than
/// failing the chunk: the caller turns a missing slot into a per-message error,
/// which is how one malformed entry stays one malformed message.
fn parse_batch_response(payload: &serde_json::Value) -> Vec<GraphBatchSubResponse> {
    let Some(responses) = payload.get("responses").and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    responses
        .iter()
        .filter_map(|entry| {
            let index = entry.get("id")?.as_str()?.parse::<usize>().ok()?;
            let status = entry.get("status")?.as_u64()? as u16;
            let body = if status == 200 {
                entry.get("body").cloned()
            } else {
                None
            };
            Some(GraphBatchSubResponse { index, status, body })
        })
        .collect()
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

// ── Attachments of an outgoing message ────────────────────────────────────────

/// `createUploadSession` response.
#[derive(Debug, Deserialize)]
struct GraphUploadSession {
    #[serde(rename = "uploadUrl")]
    upload_url: String,
}

/// What a `PUT` of a byte range answers while more is expected. The upload
/// endpoint is Outlook's own and capitalises differently from Graph.
#[derive(Debug, Deserialize)]
struct GraphUploadProgress {
    #[serde(rename = "nextExpectedRanges", alias = "NextExpectedRanges", default)]
    next_expected_ranges: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GraphAttachmentIds {
    #[serde(default)]
    value: Vec<GraphAttachmentId>,
}

#[derive(Debug, Deserialize)]
struct GraphAttachmentId {
    id: String,
}

/// One attachment of an outgoing message.
struct OutgoingAttachment<'a> {
    attachment: &'a provider::EmailAttachment,
    /// One of the body's inline images, which the payload marks `isInline`
    /// whatever the attachment says.
    force_inline: bool,
    /// Size of the content in bytes.
    size: u64,
}

/// Every attachment of an outgoing message, in the order the JSON payload
/// lists them: the body's inline images, then the files.
fn outgoing_attachments<'a>(
    body: &'a EmailBody,
    attachments: &'a [provider::EmailAttachment],
) -> Vec<OutgoingAttachment<'a>> {
    let with_size = |attachment: &'a provider::EmailAttachment, force_inline: bool| OutgoingAttachment {
        attachment,
        force_inline,
        size: outlook_upload::decoded_len(&attachment.data),
    };
    body.inline_images
        .iter()
        .map(|image| with_size(image, true))
        .chain(attachments.iter().map(|file| with_size(file, false)))
        .collect()
}

fn plan_outgoing(outgoing: &[OutgoingAttachment<'_>]) -> Result<AttachmentPlan> {
    let files: Vec<(&str, u64)> = outgoing
        .iter()
        .map(|item| (item.attachment.filename.as_str(), item.size))
        .collect();
    outlook_upload::plan_attachments(&files)
}

/// The body as the draft is created with when its attachments follow on their
/// own: the inline images are attachments too.
fn body_without_inline_images(body: &EmailBody) -> EmailBody {
    // Field by field: a `clone()` would copy the images only to drop them.
    EmailBody {
        text: body.text.clone(),
        html: body.html.clone(),
        inline_images: Vec::new(),
        language: body.language,
        append_footer: body.append_footer,
    }
}

/// The error's own text, without the variant's prefix, for wrapping it in a
/// message that names what was being done.
fn error_detail(error: &AppError) -> String {
    error
        .params()
        .get("detail")
        .cloned()
        .unwrap_or_else(|| error.to_string())
}

fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_payload_asks_for_every_message_in_one_request() {
        // The win: twenty messages become one HTTP request instead of twenty.
        let payload = build_message_batch_payload(&["msg-a", "msg-b", "msg-c"]);
        let requests = payload["requests"].as_array().expect("requests array");

        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0]["method"], "GET");
        let url = requests[0]["url"].as_str().expect("url");
        assert!(url.starts_with("/me/messages/msg-a"), "got {url}");
        assert!(
            url.contains("$select="),
            "sub-requests must keep the field projection: {url}"
        );
    }

    #[test]
    fn batch_payload_ids_the_sub_requests_by_slot() {
        // Graph may answer in any order, so the id has to carry the position.
        let payload = build_message_batch_payload(&["msg-a", "msg-b"]);
        let requests = payload["requests"].as_array().expect("requests array");
        assert_eq!(requests[0]["id"], "0");
        assert_eq!(requests[1]["id"], "1");
    }

    #[test]
    fn batch_payload_percent_encodes_the_message_id() {
        // Graph message IDs are base64url-ish and can carry characters that
        // would otherwise terminate the path or start a query string.
        let payload = build_message_batch_payload(&["a/b+c=="]);
        let url = payload["requests"][0]["url"].as_str().expect("url");
        assert!(!url.contains("a/b+c=="), "raw id leaked into the path: {url}");
        assert!(url.contains("a%2Fb%2Bc%3D%3D"), "got {url}");
    }

    #[test]
    fn batch_responses_are_matched_by_id_not_by_position() {
        // Graph explicitly does not guarantee response order.
        let body = serde_json::json!({
            "responses": [
                {"id": "1", "status": 200, "body": {"id": "msg-b"}},
                {"id": "0", "status": 200, "body": {"id": "msg-a"}}
            ]
        });
        let parsed = parse_batch_response(&body);

        assert_eq!(parsed.len(), 2);
        let first = parsed.iter().find(|r| r.index == 0).expect("slot 0");
        assert_eq!(first.body.as_ref().expect("body")["id"], "msg-a");
        let second = parsed.iter().find(|r| r.index == 1).expect("slot 1");
        assert_eq!(second.body.as_ref().expect("body")["id"], "msg-b");
    }

    #[test]
    fn a_throttled_sub_response_is_reported_with_its_status() {
        // Per-message 429s are normal in a batch; the caller retries just those
        // rather than re-fetching the whole chunk.
        let body = serde_json::json!({
            "responses": [
                {"id": "0", "status": 200, "body": {"id": "msg-a"}},
                {"id": "1", "status": 429, "headers": {"Retry-After": "3"}}
            ]
        });
        let parsed = parse_batch_response(&body);

        let throttled = parsed.iter().find(|r| r.index == 1).expect("slot 1");
        assert_eq!(throttled.status, 429);
        assert!(throttled.body.is_none());
    }

    #[test]
    fn a_malformed_batch_envelope_yields_no_sub_responses() {
        // Never panic on a shape Graph didn't promise; the caller turns missing
        // slots into per-message errors.
        assert!(parse_batch_response(&serde_json::json!({})).is_empty());
        assert!(parse_batch_response(&serde_json::json!({"responses": "nope"})).is_empty());
    }

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

    /// Regression: `/me/messages/{id}/reply` takes the Graph **item id**.
    ///
    /// The service handed every provider `email.message_id`, which for Outlook
    /// is `internetMessageId` (`<abc@host>`) — a different identifier for a
    /// different lookup. Graph rejects it, so replying from an Outlook account
    /// hit a resource that does not exist.
    /// A message whose attachments cannot be listed is still stored — with
    /// no attachment rows, and a log line saying so.
    #[tokio::test]
    async fn a_message_whose_attachments_cannot_be_listed_is_still_returned() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me/messages/m1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "m1",
                "subject": "Invoice",
                "hasAttachments": true,
                "receivedDateTime": "2026-09-01T10:00:00Z",
                "from": { "emailAddress": { "address": "billing@acme.com", "name": "Acme" } }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me/messages/m1/attachments"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let (email, _, attachments) = EmailProvider::get_message(&client, "m1").await.expect("message");

        assert_eq!(email.id, "m1");
        assert!(attachments.is_empty());
    }

    #[tokio::test]
    async fn attachment_listing_follows_the_next_link_to_the_last_page() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "value": [{ "id": "m1" }],
                "@odata.nextLink": format!("{}/page-2", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/page-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "value": [{ "id": "m2" }]
            })))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let ids = EmailProvider::list_message_ids_with_attachments(&client)
            .await
            .expect("listing");

        assert_eq!(ids, Some(vec!["m1".to_string(), "m2".to_string()]));
    }

    #[tokio::test]
    async fn a_send_mail_that_fails_with_a_server_error_is_not_resent() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/sendMail"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/me/sendMail"))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let result = client
            .send_new_email(
                "me@example.com",
                &["them@example.com".to_string()],
                &[],
                "hi",
                &EmailBody::plain("body"),
                &[],
            )
            .await;

        assert!(
            result.is_err(),
            "a failed send must surface, not be retried into success"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reply_addresses_the_graph_item_id() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/messages/AQMkAD-item-id/reply"))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        EmailProvider::send_reply(
            &client,
            "me@example.com",
            None,
            &["them@example.com".to_string()],
            &[],
            &crate::sync::provider::ReplyTarget {
                provider_message_id: "AQMkAD-item-id",
                thread_id: "conv-1",
                // Present, and deliberately NOT what the URL must use.
                message_id: Some("<abc@example.com>"),
                references: Some("<root@example.com>"),
            },
            "Re: hi",
            &EmailBody::plain("reply"),
            &[],
        )
        .await
        .expect("the mock only matches the item-id path, so reaching it proves the fix");
    }

    /// A message Graph never gave an `internetMessageId` is still replyable.
    ///
    /// The old code hard-errored on a missing Message-ID before touching the
    /// network, even though the item id it actually needed was right there.
    #[tokio::test]
    async fn reply_works_without_an_internet_message_id() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/messages/item-without-header/reply"))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        EmailProvider::send_reply(
            &client,
            "me@example.com",
            None,
            &["them@example.com".to_string()],
            &[],
            &crate::sync::provider::ReplyTarget {
                provider_message_id: "item-without-header",
                thread_id: "conv-2",
                message_id: None,
                references: None,
            },
            "Re: hi",
            &EmailBody::plain("reply"),
            &[],
        )
        .await
        .expect("a missing internetMessageId must not block a reply");
    }

    #[tokio::test]
    async fn updating_a_draft_that_is_gone_reports_not_found() {
        // A draft sent or deleted from another device 404s. The caller has to
        // tell that apart from a transient failure to re-create the draft.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/me/messages/gone-1"))
            .respond_with(ResponseTemplate::new(404).set_body_raw(
                r#"{"error":{"code":"ErrorItemNotFound","message":"The specified object was not found in the store."}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let result = client
            .update_draft(
                "gone-1",
                &["dest@example.com".to_string()],
                &[],
                "Subject",
                &EmailBody::plain("body"),
                &[],
            )
            .await;

        assert!(matches!(result, Err(AppError::NotFound(_))), "got {result:?}");
    }

    #[tokio::test]
    async fn list_drafts_follows_every_page() {
        // Regression: `$top=100` with no `@odata.nextLink` follow-up fed a
        // truncated keep-list to `prune_provider_drafts`, so an account with
        // more than a page of drafts lost the overflow on every sync.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let draft = |id: &str, subject: &str| {
            format!(
                r#"{{"id":"{id}","conversationId":"c-{id}","subject":"{subject}",
                   "receivedDateTime":"2026-07-01T10:00:00Z","isRead":false,
                   "body":{{"contentType":"html","content":"<p>hi</p>"}},
                   "toRecipients":[{{"emailAddress":{{"address":"dest@example.com","name":"D"}}}}]}}"#
            )
        };
        // Page 2 lives at its own path; the client must follow the link verbatim.
        Mock::given(method("GET"))
            .and(path("/drafts-page-2"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(r#"{{"value":[{}]}}"#, draft("d-2", "Second")),
                "application/json",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me/mailFolders/drafts/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"{{"value":[{}],"@odata.nextLink":"{}/drafts-page-2"}}"#,
                    draft("d-1", "First"),
                    server.uri()
                ),
                "application/json",
            ))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        // Fully qualified: the inherent `list_drafts` takes no arguments.
        let pull = EmailProvider::list_drafts(&client, &std::collections::HashMap::new())
            .await
            .expect("pull");

        assert_eq!(
            pull.present_ids,
            vec!["d-1".to_string(), "d-2".to_string()],
            "page 2 drafts must reach the prune keep-list"
        );
        assert_eq!(pull.changed.len(), 2, "both pages' drafts are returned as content");
    }

    #[tokio::test]
    async fn a_self_referential_next_link_aborts_instead_of_looping() {
        // A stuck $skiptoken would otherwise spin the sync task forever. Failing
        // is also safer than stopping early: a partial list would drive the
        // prune pass into deleting drafts it simply never saw.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let uri = server.uri();
        Mock::given(method("GET"))
            .and(path("/me/mailFolders/drafts/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(r#"{{"value":[],"@odata.nextLink":"{uri}/me/mailFolders/drafts/messages"}}"#),
                "application/json",
            ))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let err = EmailProvider::list_drafts(&client, &std::collections::HashMap::new())
            .await
            .expect_err("must not loop forever");
        assert!(
            err.to_string().to_lowercase().contains("too many pages"),
            "unexpected error: {err}"
        );
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
    fn attachments_list_url_filters_on_has_attachments_across_folders() {
        let url = build_attachments_list_url(GRAPH_API_BASE);
        assert!(url.starts_with(&format!("{GRAPH_API_BASE}/me/messages?")), "{url}");
        assert!(url.contains("$select=id"), "{url}");
        assert!(url.contains("$filter=hasAttachments%20eq%20true"), "{url}");
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
            internet_message_headers: None,
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
            internet_message_headers: None,
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

    #[test]
    fn batch_states_read_the_flag_a_missing_id_and_skip_everything_else() {
        let chunk: Vec<String> = ["read", "unread", "gone", "throttled", "unanswered", "malformed"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Out of order on purpose: Graph does not answer in request order.
        let envelope = serde_json::json!({ "responses": [
            { "id": "2", "status": 404, "body": { "error": { "code": "ErrorItemNotFound" } } },
            { "id": "0", "status": 200, "body": { "id": "read", "isRead": true } },
            { "id": "3", "status": 429, "body": {} },
            { "id": "1", "status": 200, "body": { "id": "unread", "isRead": false } },
            { "id": "5", "status": 200, "body": { "id": "malformed" } },
            { "id": "9", "status": 404 },
        ]});

        let mut states = states_from_batch(&chunk, &envelope);
        states.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(
            states,
            vec![
                ("gone".to_string(), provider::RemoteMessageState::Missing),
                (
                    "read".to_string(),
                    provider::RemoteMessageState::Present { is_read: true }
                ),
                (
                    "unread".to_string(),
                    provider::RemoteMessageState::Present { is_read: false }
                ),
            ],
            "a throttled, unanswered or malformed slot says nothing about its message"
        );
    }

    #[tokio::test]
    async fn state_refresh_asks_for_is_read_in_batches_of_twenty() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/$batch"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"responses":[{"id":"0","status":200,"body":{"id":"x","isRead":true}},{"id":"1","status":404}]}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let ids: Vec<String> = (0..25).map(|i| format!("m-{i}")).collect();
        let states = EmailProvider::fetch_message_states(&client, &ids)
            .await
            .expect("refresh")
            .expect("supported");

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 2, "25 ids = one batch of 20 and one of 5");
        let first: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("json body");
        assert_eq!(first["requests"].as_array().map(Vec::len), Some(20));
        assert_eq!(first["requests"][0]["url"], "/me/messages/m-0?$select=id,isRead");
        // Slot 0 and 1 of each batch were answered; the rest are unknown.
        assert_eq!(states.len(), 4);
        assert_eq!(
            states.get("m-0"),
            Some(&provider::RemoteMessageState::Present { is_read: true })
        );
        assert_eq!(states.get("m-21"), Some(&provider::RemoteMessageState::Missing));
        assert_eq!(states.get("m-5"), None);
    }

    #[tokio::test]
    async fn marking_read_patches_is_read_on_the_message() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/me/messages/m-1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"id":"m-1"}"#, "application/json"))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        EmailProvider::set_read_state(&client, "m-1", true)
            .await
            .expect("mark read");
        EmailProvider::set_read_state(&client, "m-1", false)
            .await
            .expect("mark unread");

        let requests = server.received_requests().await.expect("requests");
        let bodies: Vec<serde_json::Value> = requests
            .iter()
            .map(|r| serde_json::from_slice(&r.body).expect("json body"))
            .collect();
        assert_eq!(
            bodies,
            vec![
                serde_json::json!({ "isRead": true }),
                serde_json::json!({ "isRead": false })
            ]
        );
    }

    #[tokio::test]
    async fn trashing_moves_the_message_to_deleted_items() {
        // A move, never `DELETE /me/messages/{id}`: the message must stay
        // recoverable from the account's Deleted Items.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/messages/m-3/move"))
            .respond_with(ResponseTemplate::new(201).set_body_raw(r#"{"id":"m-3-moved"}"#, "application/json"))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        EmailProvider::trash_message(&client, "m-3", None).await.expect("trash");

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("json body");
        assert_eq!(body, serde_json::json!({ "destinationId": "deleteditems" }));
    }

    #[tokio::test]
    async fn a_write_to_a_message_graph_no_longer_has_is_reported_as_not_found() {
        // Graph re-keys a message on every move, so an id stored before the
        // user filed the message elsewhere answers 404. Callers tell that
        // apart from an outage: it is not worth retrying.
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(404).set_body_raw(
                r#"{"error":{"code":"ErrorItemNotFound","message":"The specified object was not found in the store."}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let err = EmailProvider::set_read_state(&client, "stale-id", true)
            .await
            .expect_err("404");

        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn a_trash_that_fails_with_a_server_error_is_not_resent() {
        // The move may have been carried out; replaying it would address an id
        // Graph has already retired.
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        assert!(EmailProvider::trash_message(&client, "m-3", None).await.is_err());
        assert_eq!(server.received_requests().await.expect("requests").len(), 1);
    }

    #[tokio::test]
    async fn locate_message_finds_a_moved_message_by_its_internet_message_id() {
        // Graph re-keys a message when it is moved, so the id stored locally
        // 404s and only the Message-ID header can find it again.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me/mailFolders/inbox/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"value":[{"id":"new-id","conversationId":"c-1"}]}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let located = EmailProvider::locate_message(&client, "old-id", Some("<m-1@example.com>"))
            .await
            .expect("locate")
            .expect("found");

        assert_eq!(located.id, "new-id", "the row must be re-keyed to the new Graph id");
        assert_eq!(located.mailbox, "inbox");
    }

    #[tokio::test]
    async fn locate_message_is_none_when_no_folder_holds_the_message() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"value":[]}"#, "application/json"))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());

        assert_eq!(
            EmailProvider::locate_message(&client, "old-id", Some("<m-1@example.com>"))
                .await
                .expect("locate"),
            None
        );
    }

    #[tokio::test]
    async fn locate_message_finds_a_message_moved_to_the_archive_folder() {
        // Graph's `archive` well-known folder. EmailOps has no archive mailbox
        // of its own, so an archived message is filed under inbox — the same
        // place Gmail's archived mail already lands.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        for empty in [
            "/me/mailFolders/inbox/messages",
            "/me/mailFolders/deleteditems/messages",
        ] {
            Mock::given(method("GET"))
                .and(path(empty))
                .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"value":[]}"#, "application/json"))
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/me/mailFolders/archive/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"value":[{"id":"archived-id","conversationId":"c-1"}]}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let client = OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri());
        let located = EmailProvider::locate_message(&client, "old-id", Some("<m-1@example.com>"))
            .await
            .expect("locate")
            .expect("found");

        assert_eq!(located.id, "archived-id");
        assert_eq!(located.mailbox, "inbox");
    }

    // ── Attachments that do not fit in one request ────────────────────────

    use crate::sync::outlook_upload::{INLINE_ATTACHMENT_LIMIT, MAX_ATTACHMENT_SIZE, UPLOAD_CHUNK_SIZE};
    use crate::sync::provider::EmailAttachment;
    use base64::Engine as _;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    /// Two ranges: one full chunk and a rest.
    const LARGE: usize = UPLOAD_CHUNK_SIZE as usize + 500_000;
    // Large enough to need an upload session.
    const _: () = assert!(LARGE as u64 >= INLINE_ATTACHMENT_LIMIT);

    fn content(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn file(name: &str, len: usize) -> EmailAttachment {
        EmailAttachment {
            filename: name.to_string(),
            mime_type: "application/octet-stream".to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(content(len)),
            content_id: None,
            is_inline: false,
        }
    }

    fn range_header(start: usize, end: usize, total: usize) -> String {
        format!("bytes {}-{}/{}", start, end - 1, total)
    }

    fn client_for(server: &MockServer) -> OutlookClient {
        OutlookClient::new("tok".into(), None, None, None).with_base_url(server.uri())
    }

    async fn mount_json(server: &MockServer, verb: &str, at: &str, status: u16, body: serde_json::Value) {
        Mock::given(method(verb))
            .and(path(at))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(server)
            .await;
    }

    /// A draft `d1`, an upload session for it at `/upload/d1`, and the two
    /// ranges of a [`LARGE`] file accepted.
    async fn mount_large_upload(server: &MockServer) {
        mount_json(
            server,
            "POST",
            "/me/messages/d1/attachments/createUploadSession",
            201,
            serde_json::json!({
                "uploadUrl": format!("{}/upload/d1", server.uri()),
                "expirationDateTime": "2026-09-30T20:00:00Z",
                "nextExpectedRanges": ["0-"],
            }),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path("/upload/d1"))
            .and(header(
                "content-range",
                range_header(0, UPLOAD_CHUNK_SIZE as usize, LARGE).as_str(),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "nextExpectedRanges": [UPLOAD_CHUNK_SIZE.to_string()] })),
            )
            .mount(server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/upload/d1"))
            .and(header(
                "content-range",
                range_header(UPLOAD_CHUNK_SIZE as usize, LARGE, LARGE).as_str(),
            ))
            .respond_with(ResponseTemplate::new(201))
            .mount(server)
            .await;
    }

    /// Run a scenario that uploads, holding the log seam and returning what it
    /// logged: an upload reports to the output panel through the process
    /// logger, whose events other tests count. A sync test on its own
    /// runtime, because the guard must not be held across an await.
    fn with_log_seam<F: std::future::Future>(scenario: F) -> Vec<AppLogEvent> {
        let _seam = crate::services::events::seam_test_lock();
        let logger = crate::services::logger::install_for_testing();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(scenario);
        crate::services::logger::install(std::sync::Arc::new(crate::services::logger::NoopLogger));
        logger.events()
    }

    /// `METHOD /path` of every request the server saw, in order.
    async fn requests(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| format!("{} {}", r.method, r.url.path()))
            .collect()
    }

    async fn requests_to(server: &MockServer, verb: &str, at: &str) -> Vec<Request> {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method.as_str() == verb && r.url.path() == at)
            .collect()
    }

    async fn send_new(client: &OutlookClient, attachments: &[EmailAttachment]) -> Result<provider::SentMessageMeta> {
        client
            .send_new_email(
                "me@example.com",
                &["them@example.com".to_string()],
                &[],
                "Files",
                &EmailBody::plain("see attached"),
                attachments,
            )
            .await
    }

    #[tokio::test]
    async fn a_small_attachment_still_travels_in_the_send_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/sendMail"))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;
        let small = file("notes.txt", 2_000);

        send_new(&client_for(&server), std::slice::from_ref(&small))
            .await
            .expect("sent");

        assert_eq!(requests(&server).await, vec!["POST /me/sendMail"]);
        let body: serde_json::Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["message"]["attachments"][0]["contentBytes"], small.data);
    }

    #[test]
    fn a_large_attachment_is_uploaded_in_ranges_to_a_draft_that_is_then_sent() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            mount_large_upload(&server).await;
            Mock::given(method("POST"))
                .and(path("/me/messages/d1/send"))
                .respond_with(ResponseTemplate::new(202))
                .mount(&server)
                .await;

            send_new(&client_for(&server), &[file("video.bin", LARGE)])
                .await
                .expect("sent");

            assert_eq!(
                requests(&server).await,
                vec![
                    "POST /me/messages",
                    "POST /me/messages/d1/attachments/createUploadSession",
                    "PUT /upload/d1",
                    "PUT /upload/d1",
                    "POST /me/messages/d1/send",
                ]
            );
            // The draft carries the message, not the file.
            let draft: serde_json::Value =
                serde_json::from_slice(&requests_to(&server, "POST", "/me/messages").await[0].body).unwrap();
            assert_eq!(draft["subject"], "Files");
            assert!(draft.get("attachments").is_none(), "{draft}");
            // The session describes the file.
            let session: serde_json::Value = serde_json::from_slice(
                &requests_to(&server, "POST", "/me/messages/d1/attachments/createUploadSession").await[0].body,
            )
            .unwrap();
            assert_eq!(session["AttachmentItem"]["attachmentType"], "file");
            assert_eq!(session["AttachmentItem"]["name"], "video.bin");
            assert_eq!(session["AttachmentItem"]["size"], LARGE);
            // The ranges are the file's bytes, in order, without the bearer token.
            let puts = requests_to(&server, "PUT", "/upload/d1").await;
            let uploaded: Vec<u8> = puts.iter().flat_map(|r| r.body.clone()).collect();
            assert_eq!(uploaded, content(LARGE));
            for put in &puts {
                assert!(
                    put.headers.get("authorization").is_none(),
                    "the upload URL is pre-authenticated"
                );
                assert_eq!(put.headers.get("content-type").unwrap(), "application/octet-stream");
                assert_eq!(
                    put.headers.get("content-length").unwrap().to_str().unwrap(),
                    put.body.len().to_string()
                );
            }
        });
    }

    #[test]
    fn a_failed_range_is_sent_again() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            // The first range is refused once with a retryable status.
            Mock::given(method("PUT"))
                .and(path("/upload/d1"))
                .respond_with(ResponseTemplate::new(503))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            mount_large_upload(&server).await;
            Mock::given(method("POST"))
                .and(path("/me/messages/d1/send"))
                .respond_with(ResponseTemplate::new(202))
                .mount(&server)
                .await;

            send_new(&client_for(&server), &[file("video.bin", LARGE)])
                .await
                .expect("sent");

            let ranges: Vec<String> = requests_to(&server, "PUT", "/upload/d1")
                .await
                .iter()
                .map(|r| r.headers.get("content-range").unwrap().to_str().unwrap().to_string())
                .collect();
            let first = range_header(0, UPLOAD_CHUNK_SIZE as usize, LARGE);
            assert_eq!(
                ranges,
                vec![
                    first.clone(),
                    first,
                    range_header(UPLOAD_CHUNK_SIZE as usize, LARGE, LARGE)
                ]
            );
            assert_eq!(requests_to(&server, "POST", "/me/messages/d1/send").await.len(), 1);
        });
    }

    #[test]
    fn the_upload_continues_where_graph_says() {
        with_log_seam(async {
            // Graph kept less of the first range than was sent.
            let server = MockServer::start().await;
            let kept = 1_000_000usize;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            mount_json(
                &server,
                "POST",
                "/me/messages/d1/attachments/createUploadSession",
                201,
                serde_json::json!({ "uploadUrl": format!("{}/upload/d1", server.uri()), "nextExpectedRanges": ["0-"] }),
            )
            .await;
            Mock::given(method("PUT"))
                .and(path("/upload/d1"))
                .and(header(
                    "content-range",
                    range_header(0, UPLOAD_CHUNK_SIZE as usize, LARGE).as_str(),
                ))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({ "NextExpectedRanges": [format!("{kept}-")] })),
                )
                .mount(&server)
                .await;
            Mock::given(method("PUT"))
                .and(path("/upload/d1"))
                .and(header("content-range", range_header(kept, LARGE, LARGE).as_str()))
                .respond_with(ResponseTemplate::new(201))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/me/messages/d1/send"))
                .respond_with(ResponseTemplate::new(202))
                .mount(&server)
                .await;

            send_new(&client_for(&server), &[file("video.bin", LARGE)])
                .await
                .expect("sent");

            let puts = requests_to(&server, "PUT", "/upload/d1").await;
            assert_eq!(puts.len(), 2);
            assert_eq!(puts[1].body, content(LARGE)[kept..]);
        });
    }

    #[test]
    fn a_refused_range_cancels_the_session_and_removes_the_draft() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            mount_json(
                &server,
                "POST",
                "/me/messages/d1/attachments/createUploadSession",
                201,
                serde_json::json!({ "uploadUrl": format!("{}/upload/d1", server.uri()), "nextExpectedRanges": ["0-"] }),
            )
            .await;
            mount_json(
                &server,
                "PUT",
                "/upload/d1",
                400,
                serde_json::json!({ "error": { "code": "ErrorInvalidRange", "message": "bad range" } }),
            )
            .await;
            for at in ["/upload/d1", "/me/messages/d1"] {
                Mock::given(method("DELETE"))
                    .and(path(at))
                    .respond_with(ResponseTemplate::new(204))
                    .mount(&server)
                    .await;
            }

            let err = send_new(&client_for(&server), &[file("video.bin", LARGE)])
                .await
                .expect_err("the send fails");

            assert!(err.to_string().contains("video.bin"), "{err}");
            assert!(err.to_string().contains("bad range"), "{err}");
            assert_eq!(
                requests(&server).await,
                vec![
                    "POST /me/messages",
                    "POST /me/messages/d1/attachments/createUploadSession",
                    "PUT /upload/d1",
                    "DELETE /upload/d1",
                    "DELETE /me/messages/d1",
                ],
                "nothing is sent, and no draft is left behind"
            );
            let cancel = &requests_to(&server, "DELETE", "/upload/d1").await[0];
            assert!(cancel.headers.get("authorization").is_none());
        });
    }

    #[tokio::test]
    async fn an_attachment_over_the_maximum_is_refused_before_any_request() {
        let server = MockServer::start().await;
        // All-zero content: the size is what matters.
        let symbols = (MAX_ATTACHMENT_SIZE as usize + 3) / 3 * 4;
        let too_large = EmailAttachment {
            data: "A".repeat(symbols),
            ..file("archive.zip", 0)
        };

        let client = client_for(&server);
        let sent = send_new(&client, std::slice::from_ref(&too_large)).await;
        let drafted = client
            .create_draft(
                &[],
                &[],
                "Files",
                &EmailBody::plain("x"),
                std::slice::from_ref(&too_large),
            )
            .await;

        for err in [sent.expect_err("refused"), drafted.expect_err("refused")] {
            assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
            assert!(err.to_string().contains("archive.zip"), "{err}");
        }
        assert!(requests(&server).await.is_empty());
    }

    #[tokio::test]
    async fn small_attachments_too_big_together_are_added_to_the_draft_one_by_one() {
        let server = MockServer::start().await;
        mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
        mount_json(
            &server,
            "POST",
            "/me/messages/d1/attachments",
            201,
            serde_json::json!({ "id": "a1" }),
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/me/messages/d1/send"))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;
        let half = INLINE_ATTACHMENT_LIMIT as usize / 2;
        let files = [file("one.bin", half), file("two.bin", half)];

        send_new(&client_for(&server), &files).await.expect("sent");

        assert_eq!(
            requests(&server).await,
            vec![
                "POST /me/messages",
                "POST /me/messages/d1/attachments",
                "POST /me/messages/d1/attachments",
                "POST /me/messages/d1/send",
            ]
        );
        let added = requests_to(&server, "POST", "/me/messages/d1/attachments").await;
        let names: Vec<serde_json::Value> = added
            .iter()
            .map(|r| serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["name"].clone())
            .collect();
        assert_eq!(names, vec!["one.bin", "two.bin"]);
        let first: serde_json::Value = serde_json::from_slice(&added[0].body).unwrap();
        assert_eq!(first["@odata.type"], "#microsoft.graph.fileAttachment");
        assert_eq!(first["contentBytes"], files[0].data);
    }

    #[test]
    fn a_small_file_next_to_a_large_one_is_posted_and_the_large_one_uploaded() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            mount_json(
                &server,
                "POST",
                "/me/messages/d1/attachments",
                201,
                serde_json::json!({ "id": "a1" }),
            )
            .await;
            mount_large_upload(&server).await;
            Mock::given(method("POST"))
                .and(path("/me/messages/d1/send"))
                .respond_with(ResponseTemplate::new(202))
                .mount(&server)
                .await;

            send_new(
                &client_for(&server),
                &[file("notes.txt", 2_000), file("video.bin", LARGE)],
            )
            .await
            .expect("sent");

            assert_eq!(
                requests(&server).await,
                vec![
                    "POST /me/messages",
                    "POST /me/messages/d1/attachments",
                    "POST /me/messages/d1/attachments/createUploadSession",
                    "PUT /upload/d1",
                    "PUT /upload/d1",
                    "POST /me/messages/d1/send",
                ]
            );
        });
    }

    #[test]
    fn a_reply_with_a_large_attachment_goes_through_a_reply_draft() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(
                &server,
                "POST",
                "/me/messages/orig-1/createReply",
                201,
                serde_json::json!({ "id": "d1" }),
            )
            .await;
            mount_large_upload(&server).await;
            Mock::given(method("POST"))
                .and(path("/me/messages/d1/send"))
                .respond_with(ResponseTemplate::new(202))
                .mount(&server)
                .await;

            client_for(&server)
                .send_reply(
                    "me@example.com",
                    &["them@example.com".to_string()],
                    &[],
                    "orig-1",
                    "Re: Files",
                    &EmailBody::plain("here it is"),
                    &[file("video.bin", LARGE)],
                )
                .await
                .expect("sent");

            assert_eq!(
                requests(&server).await,
                vec![
                    "POST /me/messages/orig-1/createReply",
                    "POST /me/messages/d1/attachments/createUploadSession",
                    "PUT /upload/d1",
                    "PUT /upload/d1",
                    "POST /me/messages/d1/send",
                ]
            );
            let reply: serde_json::Value =
                serde_json::from_slice(&requests_to(&server, "POST", "/me/messages/orig-1/createReply").await[0].body)
                    .unwrap();
            assert!(reply["comment"].as_str().unwrap().starts_with("here it is"), "{reply}");
            assert_eq!(
                reply["message"]["toRecipients"][0]["emailAddress"]["address"],
                "them@example.com"
            );
        });
    }

    #[test]
    fn a_send_that_fails_after_the_upload_keeps_the_draft_and_says_so() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            mount_large_upload(&server).await;
            mount_json(
                &server,
                "POST",
                "/me/messages/d1/send",
                500,
                serde_json::json!({ "error": { "code": "InternalServerError", "message": "try later" } }),
            )
            .await;

            let err = send_new(&client_for(&server), &[file("video.bin", LARGE)])
                .await
                .expect_err("the send fails");

            assert!(err.to_string().contains("try later"), "{err}");
            assert!(err.to_string().contains("Drafts"), "{err}");
            assert_eq!(
                requests_to(&server, "POST", "/me/messages/d1/send").await.len(),
                1,
                "a send is never repeated"
            );
            assert!(
                requests_to(&server, "DELETE", "/me/messages/d1").await.is_empty(),
                "it may have been sent: the draft is not removed"
            );
        });
    }

    #[test]
    fn a_new_draft_with_a_large_attachment_is_created_and_then_filled() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            mount_large_upload(&server).await;

            let id = client_for(&server)
                .create_draft(
                    &["them@example.com".to_string()],
                    &[],
                    "Files",
                    &EmailBody::plain("draft").without_footer(),
                    &[file("video.bin", LARGE)],
                )
                .await
                .expect("created");

            assert_eq!(id, "d1");
            assert_eq!(
                requests(&server).await,
                vec![
                    "POST /me/messages",
                    "POST /me/messages/d1/attachments/createUploadSession",
                    "PUT /upload/d1",
                    "PUT /upload/d1",
                ]
            );
        });
    }

    #[test]
    fn a_new_draft_whose_attachment_cannot_be_added_is_removed_again() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            mount_json(
                &server,
                "POST",
                "/me/messages/d1/attachments/createUploadSession",
                400,
                serde_json::json!({ "error": { "code": "ErrorInvalidRequest", "message": "no session" } }),
            )
            .await;
            Mock::given(method("DELETE"))
                .and(path("/me/messages/d1"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&server)
                .await;

            let result = client_for(&server)
                .create_draft(
                    &[],
                    &[],
                    "Files",
                    &EmailBody::plain("draft"),
                    &[file("video.bin", LARGE)],
                )
                .await;

            assert!(result.is_err());
            assert_eq!(requests_to(&server, "DELETE", "/me/messages/d1").await.len(), 1);
        });
    }

    #[test]
    fn updating_a_draft_with_a_large_attachment_replaces_its_attachments() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(
                &server,
                "PATCH",
                "/me/messages/d1",
                200,
                serde_json::json!({ "id": "d1" }),
            )
            .await;
            mount_json(
                &server,
                "GET",
                "/me/messages/d1/attachments",
                200,
                serde_json::json!({ "value": [{ "id": "old-1" }, { "id": "old-2" }] }),
            )
            .await;
            for old in ["old-1", "old-2"] {
                Mock::given(method("DELETE"))
                    .and(path(format!("/me/messages/d1/attachments/{old}")))
                    .respond_with(ResponseTemplate::new(204))
                    .mount(&server)
                    .await;
            }
            mount_large_upload(&server).await;

            let id = client_for(&server)
                .update_draft(
                    "d1",
                    &["them@example.com".to_string()],
                    &[],
                    "Files",
                    &EmailBody::plain("draft").without_footer(),
                    &[file("video.bin", LARGE)],
                )
                .await
                .expect("updated");

            assert_eq!(id, "d1");
            assert_eq!(
                requests(&server).await,
                vec![
                    "PATCH /me/messages/d1",
                    "GET /me/messages/d1/attachments",
                    "DELETE /me/messages/d1/attachments/old-1",
                    "DELETE /me/messages/d1/attachments/old-2",
                    "POST /me/messages/d1/attachments/createUploadSession",
                    "PUT /upload/d1",
                    "PUT /upload/d1",
                ]
            );
            let patch: serde_json::Value =
                serde_json::from_slice(&requests_to(&server, "PATCH", "/me/messages/d1").await[0].body).unwrap();
            assert!(patch.get("attachments").is_none(), "{patch}");
        });
    }

    #[test]
    fn an_upload_url_on_another_plain_http_host_is_refused() {
        with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            mount_json(
                &server,
                "POST",
                "/me/messages/d1/attachments/createUploadSession",
                201,
                serde_json::json!({ "uploadUrl": "http://uploads.example.com/session/1", "nextExpectedRanges": ["0-"] }),
            )
            .await;
            Mock::given(method("DELETE"))
                .and(path("/me/messages/d1"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&server)
                .await;

            let err = send_new(&client_for(&server), &[file("video.bin", LARGE)])
                .await
                .expect_err("refused");

            assert!(err.to_string().contains("upload URL"), "{err}");
            assert!(requests_to(&server, "PUT", "/upload/d1").await.is_empty());
        });
    }

    #[test]
    fn an_upload_is_reported_to_the_output_panel() {
        let events = with_log_seam(async {
            let server = MockServer::start().await;
            mount_json(&server, "POST", "/me/messages", 201, serde_json::json!({ "id": "d1" })).await;
            mount_large_upload(&server).await;
            Mock::given(method("POST"))
                .and(path("/me/messages/d1/send"))
                .respond_with(ResponseTemplate::new(202))
                .mount(&server)
                .await;
            send_new(&client_for(&server), &[file("panel-video.bin", LARGE)])
                .await
                .expect("sent");
        });

        let about_file: Vec<_> = events
            .iter()
            .filter(|e| e.source == "sync" && e.message.contains("panel-video.bin"))
            .collect();
        assert!(about_file.iter().any(|e| e.level == "info"), "{events:?}");
        assert!(about_file.iter().any(|e| e.level == "success"), "{events:?}");
    }
}
