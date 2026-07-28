use async_trait::async_trait;
use mailparse::{parse_mail, MailHeaderMap};
use sha2::{Digest, Sha256};

use crate::models::error::{AppError, Result};
use crate::models::Email;
use crate::sync::folder_plan::{self, resolve_role_folder, ListedFolder, WellKnownFolder};
use crate::sync::provider::{
    self, AttachmentInfo, EmailAttachment, EmailBody, EmailCategory, EmailProvider, MessageRef,
};

/// Prefix embedded in message IDs for emails fetched from the Sent folder.
/// INBOX emails keep the plain `{account_id}::{uid}` format for backward compat.
const SENT_ID_PREFIX: &str = "SENT::";
/// Prefix for emails fetched from the Spam / Junk folder.
const SPAM_ID_PREFIX: &str = "SPAM::";
/// Prefix for emails fetched from the Trash / Deleted Items folder.
const TRASH_ID_PREFIX: &str = "TRASH::";
/// Prefix for emails fetched from a custom (user-created) folder. The full id
/// shape is `{account_id}::FOLDER::{b64url(server_path)}::{uid}` — the
/// base64url segment shields folder paths that contain the `::` separator.
const FOLDER_ID_PREFIX: &str = "FOLDER::";

/// Pick the mailbox to APPEND a sent copy into, given the server's `LIST`
/// response. Delegates to the shared detection ladder (SPECIAL-USE attribute
/// first, then localized name candidates — the same logic the sync passes
/// use), preserving the server's exact casing since IMAP mailbox names are
/// case-sensitive on the wire. Falls back to `"Sent"` when the server reports
/// no recognizable Sent folder.
fn select_sent_folder(entries: &[ListedFolder]) -> String {
    resolve_role_folder(WellKnownFolder::Sent, entries).unwrap_or_else(|| "Sent".to_string())
}

/// Encode a folder server path for embedding in a message id (base64url, no
/// padding — the alphabet contains neither `:` nor path-hostile characters).
fn encode_folder_path(server_path: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD.encode(server_path.as_bytes())
}

/// Reverse of [`encode_folder_path`]. `None` for malformed segments so id
/// parsing can fall back to Inbox instead of panicking.
fn decode_folder_path(segment: &str) -> Option<String> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let bytes = URL_SAFE_NO_PAD.decode(segment).ok()?;
    String::from_utf8(bytes).ok()
}

/// Message-id prefix shared by every email of one custom folder:
/// `{account_id}::FOLDER::{b64url(server_path)}::`. The folder-management
/// service re-prefixes stored ids with this on folder rename, so it must stay
/// in lockstep with [`ImapClient::make_folder_email_id`].
pub(crate) fn folder_email_id_prefix(account_id: &str, server_path: &str) -> String {
    let encoded = encode_folder_path(server_path);
    if account_id.is_empty() {
        format!("{FOLDER_ID_PREFIX}{encoded}::")
    } else {
        format!("{account_id}::{FOLDER_ID_PREFIX}{encoded}::")
    }
}

/// Credentials for an IMAP account (stored in keychain as JSON).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImapCredentials {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub smtp_host: String,
    pub smtp_port: u16,
}

pub struct ImapClient {
    pub credentials: ImapCredentials,
    pub account_id: String,
    pub email: String,
    pub display_name: String,
}

/// Which IMAP mailbox a given message UID belongs to. Inbox is the default for
/// historic IDs that predate multi-mailbox support, so the parser falls back to
/// Inbox whenever no `*::` sub-prefix is present. `Custom` carries the exact
/// server path (wire format) of a user-created folder.
#[derive(Debug, Clone, PartialEq)]
enum ImapFolder {
    Inbox,
    Sent,
    Spam,
    Trash,
    Custom(String),
}

impl ImapFolder {
    /// The well-known role backing this folder, when it has one.
    fn well_known(&self) -> Option<WellKnownFolder> {
        match self {
            Self::Sent => Some(WellKnownFolder::Sent),
            Self::Spam => Some(WellKnownFolder::Spam),
            Self::Trash => Some(WellKnownFolder::Trash),
            Self::Inbox | Self::Custom(_) => None,
        }
    }

    /// Legacy SELECT-candidate names, used only as fallback when `LIST` fails.
    /// Non-ASCII localized candidates are excluded by construction here — they
    /// only ever match via the LIST + decode path.
    fn legacy_candidates(&self) -> &'static [&'static str] {
        match self {
            Self::Inbox => &["INBOX"],
            Self::Sent => folder_plan::SENT_FOLDER_CANDIDATES,
            Self::Spam => folder_plan::SPAM_FOLDER_CANDIDATES,
            Self::Trash => folder_plan::TRASH_FOLDER_CANDIDATES,
            Self::Custom(_) => &[],
        }
    }

    /// The `mailbox` column value for a message fetched from this folder.
    /// `parse_message` defaults every message to `"inbox"`, so `get_message`
    /// must apply this so Sent/Spam/Trash/custom-folder messages are not
    /// mis-filed as inbox.
    fn mailbox_value(&self) -> String {
        match self {
            Self::Inbox => "inbox".to_string(),
            Self::Sent => "sent".to_string(),
            Self::Spam => "spam".to_string(),
            Self::Trash => "trash".to_string(),
            Self::Custom(path) => format!("folder:{path}"),
        }
    }
}

impl ImapClient {
    pub fn new(credentials: ImapCredentials, email: String, display_name: String, account_id: String) -> Self {
        Self {
            credentials,
            account_id,
            email,
            display_name,
        }
    }

    /// Build the stable email ID for an IMAP INBOX message.
    /// When account_id is set, IDs are prefixed to prevent collisions across accounts.
    fn make_email_id(&self, uid: u32) -> String {
        if self.account_id.is_empty() {
            uid.to_string()
        } else {
            format!("{}::{}", self.account_id, uid)
        }
    }

    /// Build the stable email ID for an email fetched from the Sent folder.
    /// Uses a `SENT::` sub-prefix so `get_message` can tell which mailbox to SELECT.
    fn make_sent_email_id(&self, uid: u32) -> String {
        self.make_prefixed_email_id(SENT_ID_PREFIX, uid)
    }

    /// Build the stable email ID for an email fetched from a secondary mailbox.
    /// `sub_prefix` must be one of the `*_ID_PREFIX` constants so the lookup
    /// path can tell which folder to SELECT later.
    fn make_prefixed_email_id(&self, sub_prefix: &str, uid: u32) -> String {
        if self.account_id.is_empty() {
            format!("{}{}", sub_prefix, uid)
        } else {
            format!("{}::{}{}", self.account_id, sub_prefix, uid)
        }
    }

    /// Build the stable email ID for an email fetched from a custom folder:
    /// `{account_id}::FOLDER::{b64url(server_path)}::{uid}`.
    fn make_folder_email_id(&self, server_path: &str, uid: u32) -> String {
        let encoded = encode_folder_path(server_path);
        if self.account_id.is_empty() {
            format!("{FOLDER_ID_PREFIX}{encoded}::{uid}")
        } else {
            format!("{}::{FOLDER_ID_PREFIX}{encoded}::{uid}", self.account_id)
        }
    }

    /// Parse a message_id and return `(folder, uid_str)` where folder is the
    /// mailbox the UID lives in. Strips the account prefix, then checks for
    /// any of the `*::` sub-prefixes. Malformed `FOLDER::` segments fall back
    /// to Inbox (same policy as unknown prefixes) rather than erroring.
    fn parse_message_ref<'a>(&self, message_id: &'a str) -> (ImapFolder, &'a str) {
        let after_account = if self.account_id.is_empty() {
            message_id
        } else {
            let prefix = format!("{}::", self.account_id);
            message_id.strip_prefix(&prefix).unwrap_or(message_id)
        };
        if let Some(uid_str) = after_account.strip_prefix(SENT_ID_PREFIX) {
            (ImapFolder::Sent, uid_str)
        } else if let Some(uid_str) = after_account.strip_prefix(SPAM_ID_PREFIX) {
            (ImapFolder::Spam, uid_str)
        } else if let Some(uid_str) = after_account.strip_prefix(TRASH_ID_PREFIX) {
            (ImapFolder::Trash, uid_str)
        } else if let Some(rest) = after_account.strip_prefix(FOLDER_ID_PREFIX) {
            match rest
                .split_once("::")
                .and_then(|(encoded, uid_str)| Some((decode_folder_path(encoded)?, uid_str)))
            {
                Some((path, uid_str)) => (ImapFolder::Custom(path), uid_str),
                None => (ImapFolder::Inbox, after_account),
            }
        } else {
            (ImapFolder::Inbox, after_account)
        }
    }

    /// Run `LIST "" "*"` on an open session and adapt the typed response into
    /// the pure planner's [`ListedFolder`] shape (attributes rendered back to
    /// their wire spelling, SPECIAL-USE attrs arrive via
    /// `NameAttribute::Custom`).
    fn list_entries_blocking(
        session: &mut imap::Session<native_tls::TlsStream<std::net::TcpStream>>,
    ) -> std::result::Result<Vec<ListedFolder>, imap::Error> {
        let names = session.list(None, Some("*"))?;
        Ok(names
            .iter()
            .map(|name| ListedFolder {
                raw_name: name.name().to_string(),
                delimiter: name.delimiter().map(str::to_string),
                attributes: name
                    .attributes()
                    .iter()
                    .map(|attr| match attr {
                        imap::types::NameAttribute::NoInferiors => "\\Noinferiors".to_string(),
                        imap::types::NameAttribute::NoSelect => "\\Noselect".to_string(),
                        imap::types::NameAttribute::Marked => "\\Marked".to_string(),
                        imap::types::NameAttribute::Unmarked => "\\Unmarked".to_string(),
                        imap::types::NameAttribute::Custom(s) => s.to_string(),
                    })
                    .collect(),
            })
            .collect())
    }

    /// Full folder discovery on an open session: plain `LIST` first; when the
    /// server advertises SPECIAL-USE but volunteered no role attributes, retry
    /// with an explicit `RETURN (SPECIAL-USE)` and parse the raw response.
    fn list_folders_blocking(
        session: &mut imap::Session<native_tls::TlsStream<std::net::TcpStream>>,
    ) -> std::result::Result<Vec<ListedFolder>, imap::Error> {
        let entries = Self::list_entries_blocking(session)?;
        let has_role_attr = entries.iter().any(|e| {
            e.attributes.iter().any(|a| {
                a.eq_ignore_ascii_case("\\Sent")
                    || a.eq_ignore_ascii_case("\\Junk")
                    || a.eq_ignore_ascii_case("\\Trash")
            })
        });
        if has_role_attr {
            return Ok(entries);
        }
        let advertises_special_use = session
            .capabilities()
            .map(|caps| caps.iter().any(|c| format!("{c:?}").contains("SPECIAL-USE")))
            .unwrap_or(false);
        if !advertises_special_use {
            return Ok(entries);
        }
        match session.run_command_and_read_response("LIST \"\" \"*\" RETURN (SPECIAL-USE)") {
            Ok(bytes) => {
                let parsed = folder_plan::parse_list_response(&String::from_utf8_lossy(&bytes));
                if parsed.is_empty() {
                    Ok(entries)
                } else {
                    Ok(parsed)
                }
            }
            // The extended LIST is best-effort; the localized-name ladder
            // still covers detection when it fails.
            Err(_) => Ok(entries),
        }
    }

    /// SELECT the server folder backing `folder`. Well-known roles resolve via
    /// the shared LIST + detection ladder, with the legacy hardcoded candidate
    /// loop as fallback when LIST itself fails. Returns `false` when no
    /// matching folder exists on the server.
    fn select_folder_blocking(
        session: &mut imap::Session<native_tls::TlsStream<std::net::TcpStream>>,
        folder: &ImapFolder,
    ) -> bool {
        match folder {
            ImapFolder::Inbox => session.select("INBOX").is_ok(),
            ImapFolder::Custom(path) => session.select(path).is_ok(),
            role_folder => {
                if let Ok(entries) = Self::list_entries_blocking(session) {
                    // `well_known()` is Some for Sent/Spam/Trash — the only
                    // variants that can reach this arm.
                    if let Some(role) = role_folder.well_known() {
                        if let Some(resolved) = resolve_role_folder(role, &entries) {
                            if session.select(&resolved).is_ok() {
                                return true;
                            }
                        }
                    }
                }
                for &candidate in role_folder.legacy_candidates() {
                    if session.select(candidate).is_ok() {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Connect and login synchronously — intended for use inside `spawn_blocking`.
    ///
    /// Enforces TCP connect / read / write timeouts so a wrong host or an
    /// unresponsive server can't hang the blocking task forever. Without these,
    /// `imap::connect` will block until the OS socket eventually times out
    /// (often several minutes) which makes the "Test Connection" UI appear
    /// stuck.
    pub(crate) fn connect_sync(
        creds: &ImapCredentials,
    ) -> std::result::Result<imap::Session<native_tls::TlsStream<std::net::TcpStream>>, imap::Error> {
        use std::io;
        use std::net::{TcpStream, ToSocketAddrs};
        use std::time::Duration;

        const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
        const IO_TIMEOUT: Duration = Duration::from_secs(15);

        let tls = native_tls::TlsConnector::new().map_err(|e| {
            // Wrap as a MissingMessageData error since there's no generic imap::Error variant for TLS
            imap::Error::Bad(format!("TLS init failed: {e}"))
        })?;

        // Resolve the first address and connect with a timeout.
        let addr = (creds.host.as_str(), creds.port)
            .to_socket_addrs()
            .map_err(imap::Error::Io)?
            .next()
            .ok_or_else(|| {
                imap::Error::Io(io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    format!("No addresses resolved for {}:{}", creds.host, creds.port),
                ))
            })?;

        let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(imap::Error::Io)?;
        tcp.set_read_timeout(Some(IO_TIMEOUT)).map_err(imap::Error::Io)?;
        tcp.set_write_timeout(Some(IO_TIMEOUT)).map_err(imap::Error::Io)?;

        let tls_stream = tls
            .connect(creds.host.as_str(), tcp)
            .map_err(|e| imap::Error::Bad(format!("TLS handshake failed: {e}")))?;

        let client = imap::Client::new(tls_stream);
        let session = client.login(&creds.username, &creds.password).map_err(|(e, _)| e)?;
        Ok(session)
    }

    /// Test IMAP + SMTP connectivity. Returns separate error messages for each protocol.
    /// `Ok(())` means both succeeded.
    ///
    /// Wrapped in a hard timeout so that the frontend never sees a permanently
    /// stuck "Testing…" button even if the network layer hangs unexpectedly.
    pub async fn test_connection(&self) -> Result<()> {
        use std::time::Duration;
        const OVERALL_TIMEOUT: Duration = Duration::from_secs(25);

        tokio::time::timeout(OVERALL_TIMEOUT, async {
            // Test IMAP
            self.login_raw().await?;
            // Test SMTP
            self.test_smtp().await?;
            Ok::<(), AppError>(())
        })
        .await
        .map_err(|_| {
            AppError::SyncError(format!(
                "Connection test timed out after {}s",
                OVERALL_TIMEOUT.as_secs()
            ))
        })??;
        Ok(())
    }

    /// Test SMTP connectivity by connecting and issuing EHLO.
    pub async fn test_smtp(&self) -> Result<()> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, Tokio1Executor};

        let creds = Credentials::new(self.credentials.username.clone(), self.credentials.password.clone());
        // Port 465 = SMTPS (TLS on connect); anything else (e.g. 587) = STARTTLS.
        // Using relay() on a STARTTLS port causes an immediate TLS handshake against
        // a server that's still in plain-text mode, which hangs until timeout.
        let builder = if self.credentials.smtp_port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&self.credentials.smtp_host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.credentials.smtp_host)
        }
        .map_err(|e| AppError::SyncError(format!("SMTP relay init failed: {e}")))?;

        let mailer: AsyncSmtpTransport<Tokio1Executor> =
            builder.credentials(creds).port(self.credentials.smtp_port).build();

        mailer
            .test_connection()
            .await
            .map_err(|e| AppError::SyncError(format!("SMTP connection test failed: {e}")))?;
        Ok(())
    }

    /// Verify credentials by opening a session and immediately logging out.
    pub async fn login_raw(&self) -> Result<()> {
        let creds = self.credentials.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut s =
                Self::connect_sync(&creds).map_err(|e| AppError::AuthError(format!("IMAP login failed: {e}")))?;
            let _ = s.logout();
            Ok(())
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking join error: {e}")))??;
        Ok(())
    }

    // ── Parse raw RFC 5322 bytes ──────────────────────────────────────────────

    fn parse_message(uid: u32, raw: &[u8]) -> Result<(Email, Vec<AttachmentInfo>)> {
        let parsed = parse_mail(raw).map_err(|e| AppError::SyncError(format!("Failed to parse IMAP message: {e}")))?;

        let hdrs = &parsed.headers;

        let subject = hdrs
            .get_first_value("Subject")
            .unwrap_or_else(|| "(no subject)".to_string());

        let (sender_name, sender_email_addr) = parse_from_header(&hdrs.get_first_value("From").unwrap_or_default());

        let recipients = parse_address_list(&hdrs.get_first_value("To").unwrap_or_default());

        let cc = parse_address_list(&hdrs.get_first_value("Cc").unwrap_or_default());

        let timestamp = hdrs
            .get_first_value("Date")
            .and_then(|d| mailparse::dateparse(&d).ok())
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        let message_id = hdrs.get_first_value("Message-ID").map(|s| s.trim().to_string());

        let references = hdrs.get_first_value("References");
        let in_reply_to = hdrs.get_first_value("In-Reply-To");
        let thread_id = derive_thread_id(
            message_id.as_deref(),
            references.as_deref(),
            in_reply_to.as_deref(),
            timestamp,
            &subject,
        );

        let (body, snippet) = extract_body(&parsed);
        let attachments = extract_attachments(&parsed);

        let email = Email {
            id: uid.to_string(),
            account_id: String::new(),
            thread_id,
            message_id,
            subject,
            sender: sender_name,
            sender_email: sender_email_addr,
            recipients,
            cc,
            body,
            snippet,
            timestamp,
            is_read: false,
            triage_status: None,
            category: "primary".to_string(),
            // Overridden by the caller per-mailbox (INBOX/Sent/Trash/Junk).
            mailbox: "inbox".to_string(),
            // IMAP has no per-message sent label — a message is sent iff it
            // came from the Sent mailbox, which the insert derives from the
            // caller's `mailbox` value.
            is_sent: false,
        };

        Ok((email, attachments))
    }
}

// ── Thread ID derivation ──────────────────────────────────────────────────────

fn derive_thread_id(
    message_id: Option<&str>,
    references: Option<&str>,
    in_reply_to: Option<&str>,
    timestamp: i64,
    subject: &str,
) -> String {
    // Root is the first entry in References, falling back to In-Reply-To, then Message-ID
    let root = references
        .and_then(|r| r.split_whitespace().next())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            in_reply_to
                .and_then(|s| s.split_whitespace().next())
                .filter(|s| !s.is_empty())
        })
        .or(message_id)
        .unwrap_or("");

    let input = if root.is_empty() {
        format!("{timestamp}{subject}")
    } else {
        root.to_string()
    };

    let hash = Sha256::digest(input.as_bytes());
    hex::encode(hash)[..16].to_string()
}

// ── Address parsing ───────────────────────────────────────────────────────────

/// Parse "Display Name <email@host>" or "email@host" → (name, email).
fn parse_from_header(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if let Some(lt) = raw.rfind('<') {
        let email = raw[lt + 1..].trim_end_matches('>').trim().to_string();
        let name = raw[..lt].trim().trim_matches('"').to_string();
        (name, email)
    } else {
        (String::new(), raw.to_string())
    }
}

/// Parse a comma-separated list of addresses → Vec<email string>.
fn parse_address_list(raw: &str) -> Vec<String> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    raw.split(',')
        .map(|s| {
            let s = s.trim();
            if let Some(lt) = s.rfind('<') {
                s[lt + 1..].trim_end_matches('>').trim().to_string()
            } else {
                s.to_string()
            }
        })
        .filter(|s| !s.is_empty())
        .collect()
}

// ── Body extraction ───────────────────────────────────────────────────────────

fn extract_body(msg: &mailparse::ParsedMail) -> (String, String) {
    let (html, plain) = collect_body_parts(msg);

    let body = if let Some(h) = html {
        h
    } else if let Some(t) = plain.as_ref() {
        let escaped = t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        format!("<pre style=\"white-space:pre-wrap\">{}</pre>", escaped)
    } else {
        String::new()
    };

    let stripped;
    let snippet_src: &str = if let Some(ref p) = plain {
        p.as_str()
    } else {
        stripped = strip_html_tags_owned(&body);
        &stripped
    };
    let snippet: String = snippet_src.chars().take(200).collect();
    (body, snippet)
}

fn collect_body_parts(msg: &mailparse::ParsedMail) -> (Option<String>, Option<String>) {
    let ct = &msg.ctype;
    let mime = ct.mimetype.as_str();

    if mime == "text/html" {
        let body = msg.get_body().unwrap_or_default();
        return (Some(body), None);
    }
    if mime == "text/plain" {
        let body = msg.get_body().unwrap_or_default();
        return (None, Some(body));
    }

    let mut html: Option<String> = None;
    let mut plain: Option<String> = None;

    for sub in &msg.subparts {
        let (sh, sp) = collect_body_parts(sub);
        if html.is_none() {
            html = sh;
        }
        if plain.is_none() {
            plain = sp;
        }
    }

    (html, plain)
}

fn strip_html_tags_owned(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

// ── Attachment extraction ─────────────────────────────────────────────────────

fn extract_attachments(msg: &mailparse::ParsedMail) -> Vec<AttachmentInfo> {
    let mut out = Vec::new();
    collect_attachments(msg, &mut out);
    out
}

fn collect_attachments(part: &mailparse::ParsedMail, out: &mut Vec<AttachmentInfo>) {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let ct = &part.ctype;
    let mime = ct.mimetype.as_str();
    let disposition = part
        .headers
        .get_first_value("Content-Disposition")
        .unwrap_or_default()
        .to_lowercase();

    // Extract filename: Content-Disposition filename= takes precedence, then Content-Type name=.
    let filename = disposition
        .split(';')
        .find(|p| p.trim().starts_with("filename"))
        .and_then(|p| p.split_once('=').map(|x| x.1.trim().trim_matches('"').to_string()))
        .or_else(|| ct.params.get("name").cloned());

    // A MIME part counts as an attachment when:
    // 1. Content-Disposition explicitly says "attachment" (RFC 2183).
    // 2. Content-Disposition says "inline" but with a filename — inline named
    //    parts (e.g. embedded images from Apple Mail / Outlook Sent copies).
    // 3. No Content-Disposition at all but the Content-Type carries a "name"
    //    parameter and the type is not text/plain, text/html, or multipart/*
    //    (common in older mail clients and IMAP Sent-folder copies).
    let is_attachment = disposition.contains("attachment")
        || (filename.is_some()
            && !mime.starts_with("text/plain")
            && !mime.starts_with("text/html")
            && !mime.starts_with("multipart/"));

    if is_attachment {
        let filename = filename.unwrap_or_else(|| "attachment".to_string());
        if let Ok(data) = part.get_body_raw() {
            out.push(AttachmentInfo {
                // Use "INLINE::<filename>" so fetch_email_attachment_bytes can look up
                // the stored inline_data without a round-trip to the IMAP server.
                attachment_id: format!("INLINE::{filename}"),
                filename,
                mime_type: ct.mimetype.clone(),
                size: data.len() as i64,
                inline_data: Some(STANDARD.encode(&data)),
            });
        }
    }

    for sub in &part.subparts {
        collect_attachments(sub, out);
    }
}

// ── EmailProvider implementation ──────────────────────────────────────────────

#[async_trait]
impl EmailProvider for ImapClient {
    async fn get_profile(&self) -> Result<(String, String)> {
        Ok((self.email.clone(), self.display_name.clone()))
    }

    async fn list_messages(
        &self,
        max_results: u32,
        _page_token: Option<&str>,
        after_timestamp: Option<i64>,
        _before_timestamp: Option<i64>,
        _label_filter: Option<&str>,
    ) -> Result<(Vec<MessageRef>, Option<String>)> {
        let creds = self.credentials.clone();
        // (uid, is_sent)
        let uids: Vec<(u32, bool)> = tokio::task::spawn_blocking(move || -> Result<Vec<(u32, bool)>> {
            let mut session =
                Self::connect_sync(&creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;

            let query = if let Some(after) = after_timestamp {
                let date = chrono::DateTime::from_timestamp(after, 0)
                    .unwrap_or_else(chrono::Utc::now)
                    .format("%d-%b-%Y")
                    .to_string();
                format!("SINCE {date}")
            } else {
                "ALL".to_string()
            };

            // Fetch INBOX UIDs with the incremental / backfill date filter.
            session
                .select("INBOX")
                .map_err(|e| AppError::SyncError(format!("IMAP SELECT INBOX failed: {e}")))?;
            let inbox_uids = session
                .uid_search(&query)
                .map_err(|e| AppError::SyncError(format!("IMAP SEARCH failed: {e}")))?;

            // Fetch Sent folder UIDs with NO date filter ("ALL").
            //
            // Sent emails can predate every email in INBOX (e.g. the first message in a
            // thread was sent by the user). Using the same incremental/backfill timestamp
            // would miss those older emails. Searching ALL and letting `emails_exist_batch`
            // deduplicate is cheap — UID scanning is a pure-index operation on the server
            // and never re-downloads already-synced messages.
            let mut sent_uids: Vec<u32> = Vec::new();
            if Self::select_folder_blocking(&mut session, &ImapFolder::Sent) {
                if let Ok(uids) = session.uid_search("ALL") {
                    sent_uids = uids.into_iter().collect();
                }
            }

            let _ = session.logout();

            let mut all: Vec<(u32, bool)> = inbox_uids.into_iter().map(|u| (u, false)).collect();
            all.extend(sent_uids.into_iter().map(|u| (u, true)));
            Ok(all)
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking error: {e}")))??;

        // INBOX and Sent UIDs live in separate UID namespaces so sorting them together
        // is meaningless. Limit each mailbox independently: most-recent INBOX first,
        // then all Sent (no cap — already-synced ones are dropped by emails_exist_batch).
        let (mut inbox_list, sent_list): (Vec<_>, Vec<_>) = uids.into_iter().partition(|(_, is_sent)| !is_sent);
        inbox_list.sort_unstable_by_key(|item| std::cmp::Reverse(item.0));
        inbox_list.truncate(max_results as usize);

        let refs = inbox_list
            .into_iter()
            .chain(sent_list)
            .map(|(uid, is_sent)| MessageRef {
                id: if is_sent {
                    self.make_sent_email_id(uid)
                } else {
                    self.make_email_id(uid)
                },
                thread_id: String::new(),
            })
            .collect();

        Ok((refs, None))
    }

    async fn get_message(&self, message_id: &str) -> Result<(Email, EmailCategory, Vec<AttachmentInfo>)> {
        let (folder, uid_str) = self.parse_message_ref(message_id);
        let uid: u32 = uid_str
            .parse()
            .map_err(|_| AppError::SyncError(format!("Invalid IMAP UID: {uid_str}")))?;

        let creds = self.credentials.clone();
        let folder_for_select = folder.clone();
        let (mut email, attachments) = tokio::task::spawn_blocking(move || -> Result<(Email, Vec<AttachmentInfo>)> {
            let mut session =
                Self::connect_sync(&creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;

            if !Self::select_folder_blocking(&mut session, &folder_for_select) {
                return Err(AppError::SyncError(format!(
                    "IMAP {:?} folder not found on server",
                    folder_for_select
                )));
            }

            let messages = session
                .uid_fetch(uid.to_string(), "RFC822")
                .map_err(|e| AppError::SyncError(format!("IMAP FETCH failed: {e}")))?;

            let raw = messages
                .iter()
                .next()
                .and_then(|f| f.body())
                .ok_or_else(|| AppError::NotFound(format!("IMAP UID {uid}: body not found")))?
                .to_vec();

            let _ = session.logout();
            Self::parse_message(uid, &raw)
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking error: {e}")))??;

        // Use the full (prefixed) message_id so the stored ID is globally unique
        email.id = message_id.to_string();
        // `parse_message` defaults every message to mailbox='inbox'. Override
        // with the folder this UID was actually fetched from, otherwise a Sent
        // (or Spam/Trash/custom-folder) message ingested by the primary merged
        // pass would be stored as 'inbox' and show up in the Inbox view too.
        email.mailbox = folder.mailbox_value();
        // Sent emails are always read.
        if folder == ImapFolder::Sent {
            email.is_read = true;
        }

        Ok((email, EmailCategory::Primary, attachments))
    }

    async fn list_mailbox_messages(
        &self,
        mailbox: provider::ExtraMailbox,
        max_results: u32,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
    ) -> Result<Vec<MessageRef>> {
        let (folder, id_prefix) = match mailbox {
            provider::ExtraMailbox::Sent => (ImapFolder::Sent, SENT_ID_PREFIX),
            provider::ExtraMailbox::Spam => (ImapFolder::Spam, SPAM_ID_PREFIX),
            provider::ExtraMailbox::Trash => (ImapFolder::Trash, TRASH_ID_PREFIX),
        };

        let creds = self.credentials.clone();
        let uids: Vec<u32> = tokio::task::spawn_blocking(move || -> Result<Vec<u32>> {
            let mut session =
                Self::connect_sync(&creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;

            // Resolve + SELECT the role folder. Folder absence is not an
            // error — some providers simply don't expose Spam or Trash; we
            // return an empty list so the caller logs and moves on.
            if !Self::select_folder_blocking(&mut session, &folder) {
                let _ = session.logout();
                return Ok(Vec::new());
            }

            // IMAP SEARCH `SINCE` / `BEFORE` are day-level. Pad SINCE by a day so
            // we don't miss messages that landed on the same day as the watermark,
            // and pad BEFORE by a day so we don't accidentally exclude the day
            // matching the upper bound.
            let mut clauses: Vec<String> = Vec::new();
            if let Some(after) = after_timestamp {
                let since = (after - 86_400).max(0);
                let date = chrono::DateTime::from_timestamp(since, 0)
                    .unwrap_or_else(chrono::Utc::now)
                    .format("%d-%b-%Y")
                    .to_string();
                clauses.push(format!("SINCE {date}"));
            }
            if let Some(before) = before_timestamp {
                let before_padded = before.saturating_add(86_400);
                let date = chrono::DateTime::from_timestamp(before_padded, 0)
                    .unwrap_or_else(chrono::Utc::now)
                    .format("%d-%b-%Y")
                    .to_string();
                clauses.push(format!("BEFORE {date}"));
            }
            let query = if clauses.is_empty() {
                "ALL".to_string()
            } else {
                clauses.join(" ")
            };

            let uids = session
                .uid_search(&query)
                .map_err(|e| AppError::SyncError(format!("IMAP SEARCH failed: {e}")))?;

            let _ = session.logout();
            Ok(uids.into_iter().collect())
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking error: {e}")))??;

        // Newest UIDs first (IMAP UIDs are monotonically increasing per folder),
        // then cap at max_results so the initial sync pulls the most recent
        // window rather than the oldest.
        let mut sorted = uids;
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        sorted.truncate(max_results as usize);

        let refs = sorted
            .into_iter()
            .map(|uid| MessageRef {
                id: self.make_prefixed_email_id(id_prefix, uid),
                thread_id: String::new(),
            })
            .collect();

        Ok(refs)
    }

    async fn list_folders(&self) -> Result<Vec<ListedFolder>> {
        let creds = self.credentials.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<ListedFolder>> {
            let mut session =
                Self::connect_sync(&creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;
            let entries = Self::list_folders_blocking(&mut session)
                .map_err(|e| AppError::SyncError(format!("IMAP LIST failed: {e}")))?;
            let _ = session.logout();
            Ok(entries)
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking error: {e}")))?
    }

    async fn list_folder_messages(
        &self,
        server_path: &str,
        max_results: u32,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
    ) -> Result<Vec<MessageRef>> {
        let creds = self.credentials.clone();
        let path = server_path.to_string();
        let uids: Vec<u32> = tokio::task::spawn_blocking(move || -> Result<Vec<u32>> {
            let mut session =
                Self::connect_sync(&creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;

            // A folder deleted server-side between LIST and this SELECT is not
            // an error — return empty so the caller logs and moves on.
            if session.select(&path).is_err() {
                let _ = session.logout();
                return Ok(Vec::new());
            }

            // Same day-level SINCE/BEFORE padding as `list_mailbox_messages`.
            let mut clauses: Vec<String> = Vec::new();
            if let Some(after) = after_timestamp {
                let since = (after - 86_400).max(0);
                let date = chrono::DateTime::from_timestamp(since, 0)
                    .unwrap_or_else(chrono::Utc::now)
                    .format("%d-%b-%Y")
                    .to_string();
                clauses.push(format!("SINCE {date}"));
            }
            if let Some(before) = before_timestamp {
                let before_padded = before.saturating_add(86_400);
                let date = chrono::DateTime::from_timestamp(before_padded, 0)
                    .unwrap_or_else(chrono::Utc::now)
                    .format("%d-%b-%Y")
                    .to_string();
                clauses.push(format!("BEFORE {date}"));
            }
            let query = if clauses.is_empty() {
                "ALL".to_string()
            } else {
                clauses.join(" ")
            };

            let uids = session
                .uid_search(&query)
                .map_err(|e| AppError::SyncError(format!("IMAP SEARCH failed: {e}")))?;

            let _ = session.logout();
            Ok(uids.into_iter().collect())
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking error: {e}")))??;

        let mut sorted = uids;
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        sorted.truncate(max_results as usize);

        Ok(sorted
            .into_iter()
            .map(|uid| MessageRef {
                id: self.make_folder_email_id(server_path, uid),
                thread_id: String::new(),
            })
            .collect())
    }

    async fn send_reply(
        &self,
        from_email: &str,
        to_emails: &[String],
        cc_emails: &[String],
        _thread_id: &str,
        original_message_id: Option<&str>,
        subject: &str,
        body: &EmailBody,
        attachments: &[EmailAttachment],
    ) -> Result<crate::sync::provider::SentMessageMeta> {
        let message = crate::sync::mime_builder::build_lettre_message(&crate::sync::mime_builder::SendMimeParams {
            from_email,
            to_emails,
            cc_emails,
            subject,
            in_reply_to: original_message_id.filter(|s| !s.trim().is_empty()),
            body,
            attachments,
        })?;
        // The appended Sent copy carries this same Message-ID, so the sync
        // reconciler can exact-match it against the optimistic local row.
        let message_id_header = crate::sync::mime_builder::extract_message_id(&message);
        let raw = message.formatted();
        self.smtp_send(message).await?;
        self.save_sent_copy(raw).await;
        Ok(crate::sync::provider::SentMessageMeta {
            message_id_header,
            ..Default::default()
        })
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
        let raw = message.formatted();
        self.smtp_send(message).await?;
        self.save_sent_copy(raw).await;
        Ok(crate::sync::provider::SentMessageMeta {
            message_id_header,
            ..Default::default()
        })
    }

    async fn fetch_attachment_bytes(&self, _message_id: &str, _attachment_id: &str) -> Result<Vec<u8>> {
        Err(AppError::SyncError(
            "IMAP attachment re-fetch not supported; data is inline in the message".to_string(),
        ))
    }

    async fn create_folder(&self, server_path: &str) -> Result<()> {
        let creds = self.credentials.clone();
        let path = server_path.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut session =
                Self::connect_sync(&creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;
            let result = session
                .create(&path)
                .map_err(|e| AppError::SyncError(format!("IMAP CREATE '{path}' failed: {e}")));
            let _ = session.logout();
            result
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking error: {e}")))?
    }

    async fn rename_folder(&self, old_server_path: &str, new_server_path: &str) -> Result<()> {
        let creds = self.credentials.clone();
        let old_path = old_server_path.to_string();
        let new_path = new_server_path.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut session =
                Self::connect_sync(&creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;
            let result = session
                .rename(&old_path, &new_path)
                .map_err(|e| AppError::SyncError(format!("IMAP RENAME '{old_path}' -> '{new_path}' failed: {e}")));
            let _ = session.logout();
            result
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking error: {e}")))?
    }

    async fn delete_folder(&self, server_path: &str) -> Result<()> {
        let creds = self.credentials.clone();
        let path = server_path.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut session =
                Self::connect_sync(&creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;
            let result = session
                .delete(&path)
                .map_err(|e| AppError::SyncError(format!("IMAP DELETE '{path}' failed: {e}")));
            let _ = session.logout();
            result
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking error: {e}")))?
    }

    async fn move_message(
        &self,
        message_id: &str,
        message_id_header: Option<&str>,
        target: &provider::MoveTarget,
    ) -> Result<Option<MessageRef>> {
        let (source, uid_str) = self.parse_message_ref(message_id);
        let uid: u32 = uid_str
            .parse()
            .map_err(|_| AppError::SyncError(format!("Invalid IMAP UID: {uid_str}")))?;
        let target_name = match target {
            provider::MoveTarget::Inbox => "INBOX".to_string(),
            provider::MoveTarget::Folder(path) => path.clone(),
        };

        let creds = self.credentials.clone();
        // Quotes inside a Message-ID would break the SEARCH syntax; such ids
        // don't occur in practice, so we just skip UID resolution for them.
        let header = message_id_header
            .map(str::trim)
            .filter(|h| !h.is_empty() && !h.contains('"'))
            .map(str::to_string);
        let target_for_select = target_name.clone();
        let new_uid: Option<u32> = tokio::task::spawn_blocking(move || -> Result<Option<u32>> {
            let mut session =
                Self::connect_sync(&creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;

            if !Self::select_folder_blocking(&mut session, &source) {
                let _ = session.logout();
                return Err(AppError::SyncError(format!(
                    "IMAP source folder {source:?} not found on server"
                )));
            }

            let uid_set = uid.to_string();
            let has_move = session
                .capabilities()
                .map(|caps| caps.iter().any(|c| format!("{c:?}").contains("MOVE")))
                .unwrap_or(false);
            if has_move {
                session
                    .uid_mv(&uid_set, &target_for_select)
                    .map_err(|e| AppError::SyncError(format!("IMAP MOVE to '{target_for_select}' failed: {e}")))?;
            } else {
                // RFC 3501 fallback: COPY + \Deleted + expunge. Prefer UID
                // EXPUNGE (UIDPLUS) so other \Deleted messages in the source
                // folder are left alone.
                session
                    .uid_copy(&uid_set, &target_for_select)
                    .map_err(|e| AppError::SyncError(format!("IMAP COPY to '{target_for_select}' failed: {e}")))?;
                session
                    .uid_store(&uid_set, "+FLAGS (\\Deleted)")
                    .map_err(|e| AppError::SyncError(format!("IMAP STORE \\Deleted failed: {e}")))?;
                let has_uidplus = session
                    .capabilities()
                    .map(|caps| caps.iter().any(|c| format!("{c:?}").contains("UIDPLUS")))
                    .unwrap_or(false);
                let expunged = if has_uidplus {
                    session
                        .run_command_and_read_response(format!("UID EXPUNGE {uid}"))
                        .map(|_| ())
                } else {
                    session.expunge().map(|_| ())
                };
                if let Err(e) = expunged {
                    // The copy landed and the original is flagged \Deleted —
                    // functionally moved. Log rather than fail the operation.
                    crate::services::logger::log(
                        "debug",
                        "sync",
                        format!("IMAP EXPUNGE after move failed (message copied + flagged): {e}"),
                    );
                }
            }

            // Resolve the message's UID in the target folder so the caller
            // can re-ingest it under its new id without a full folder resync.
            let mut new_uid = None;
            if let Some(h) = header {
                if session.select(&target_for_select).is_ok() {
                    if let Ok(found) = session.uid_search(format!("HEADER Message-ID \"{h}\"")) {
                        new_uid = found.into_iter().max();
                    }
                }
            }
            let _ = session.logout();
            Ok(new_uid)
        })
        .await
        .map_err(|e| AppError::SyncError(format!("spawn_blocking error: {e}")))??;

        Ok(new_uid.map(|u| MessageRef {
            id: match target {
                provider::MoveTarget::Inbox => self.make_email_id(u),
                provider::MoveTarget::Folder(path) => self.make_folder_email_id(path, u),
            },
            thread_id: String::new(),
        }))
    }
}

impl ImapClient {
    /// Run an IMAP IDLE loop synchronously (call from `tokio::task::spawn_blocking`).
    ///
    /// Sends `true` on the channel when the server reports new mail, `false` on a
    /// keepalive timeout (the loop re-enters IDLE without disconnecting).
    /// Returns — closing the channel — when the connection fails or the receiver drops.
    /// Run an IMAP IDLE loop synchronously (call from `tokio::task::spawn_blocking`).
    ///
    /// Sends `true` on the channel when the server reports mailbox changes (new mail),
    /// `false` on a keepalive timeout (re-enters IDLE without syncing).
    /// Closes the channel when the connection fails or the receiver is dropped.
    pub fn run_imap_idle_blocking(
        creds: ImapCredentials,
        tx: tokio::sync::mpsc::Sender<bool>,
        stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        use imap::extensions::idle::WaitOutcome;
        use std::sync::atomic::Ordering;

        // Short timeout so the loop checks `stop_flag` frequently.
        // The IMAP spec allows up to 29 minutes; 30 s is well within that.
        const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

        let mut session = match Self::connect_sync(&creds) {
            Ok(s) => s,
            Err(_) => return,
        };

        if session.select("INBOX").is_err() {
            let _ = session.logout();
            return;
        }

        loop {
            if stop_flag.load(Ordering::Relaxed) {
                let _ = session.logout();
                return;
            }

            let handle = match session.idle() {
                Ok(h) => h,
                Err(_) => return, // connection broken; outer watcher will reconnect
            };

            // wait_with_timeout sends IDLE, blocks until mailbox change or timeout.
            let new_mail = match handle.wait_with_timeout(IDLE_TIMEOUT) {
                Ok(WaitOutcome::MailboxChanged) => true,
                Ok(WaitOutcome::TimedOut) => false, // keepalive — re-enter IDLE
                Err(_) => return,                   // connection error
            };

            if tx.blocking_send(new_mail).is_err() {
                return; // receiver dropped (scheduler stopped)
            }
        }
    }

    async fn smtp_send(&self, email: lettre::Message) -> Result<()> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

        let creds = Credentials::new(self.credentials.username.clone(), self.credentials.password.clone());
        let mailer = if self.credentials.smtp_port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&self.credentials.smtp_host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.credentials.smtp_host)
        }
        .map_err(|e| AppError::SyncError(format!("SMTP relay init failed: {e}")))?
        .credentials(creds)
        .port(self.credentials.smtp_port)
        .build();

        mailer
            .send(email)
            .await
            .map_err(|e| AppError::SyncError(format!("SMTP send failed: {e}")))?;
        Ok(())
    }

    /// Save a copy of a just-sent message into the account's Sent folder.
    ///
    /// SMTP submission only delivers to recipients — it does NOT file a copy in
    /// the sender's Sent folder. Provider HTTP APIs (Gmail, Graph) do this
    /// server-side, but a plain IMAP/SMTP account (Amazon WorkMail included)
    /// must IMAP `APPEND` the copy itself, otherwise the message never appears
    /// in the Sent view.
    ///
    /// Best-effort: the message has already been delivered by the time this
    /// runs, so an APPEND failure must not fail the send. It is logged to the
    /// output panel so the user knows the Sent copy is missing.
    async fn save_sent_copy(&self, raw: Vec<u8>) {
        let creds = self.credentials.clone();
        let outcome = tokio::task::spawn_blocking(move || Self::append_to_sent_blocking(&creds, &raw))
            .await
            .unwrap_or_else(|e| Err(AppError::SyncError(format!("sent-copy task join error: {e}"))));

        if let Err(e) = outcome {
            crate::services::logger::log(
                "error",
                "sync",
                format!(
                    "[{}] Email was sent, but saving a copy to the Sent folder failed: {e}",
                    self.email
                ),
            );
        }
    }

    /// Synchronous IMAP `APPEND` of `raw` RFC 5322 bytes into the Sent folder.
    /// Intended to run inside `spawn_blocking`. Marks the appended message
    /// `\Seen` so the user's own outgoing mail does not show as unread.
    fn append_to_sent_blocking(creds: &ImapCredentials, raw: &[u8]) -> Result<()> {
        use imap::types::Flag;

        let mut session =
            Self::connect_sync(creds).map_err(|e| AppError::SyncError(format!("IMAP connect failed: {e}")))?;

        let folder = match Self::list_entries_blocking(&mut session) {
            Ok(entries) => select_sent_folder(&entries),
            // If LIST fails, fall back to the conventional default rather than
            // giving up — many servers expose "Sent" even when LIST is flaky.
            Err(_) => "Sent".to_string(),
        };

        let append_result = session.append_with_flags(&folder, raw, &[Flag::Seen]);
        let _ = session.logout();
        append_result.map_err(|e| AppError::SyncError(format!("IMAP APPEND to '{folder}' failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(names: &[&str]) -> Vec<ListedFolder> {
        names
            .iter()
            .map(|n| ListedFolder {
                raw_name: n.to_string(),
                delimiter: Some(".".to_string()),
                attributes: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn select_sent_folder_prefers_highest_priority_candidate() {
        // Server exposes several mailboxes; "Sent" outranks the others.
        let existing = entries(&["INBOX", "Sent Items", "Sent"]);
        assert_eq!(select_sent_folder(&existing), "Sent");
    }

    #[test]
    fn select_sent_folder_matches_provider_specific_name() {
        // Amazon WorkMail / Exchange use "Sent Items" and no plain "Sent".
        let existing = entries(&["INBOX", "Sent Items"]);
        assert_eq!(select_sent_folder(&existing), "Sent Items");
    }

    #[test]
    fn select_sent_folder_is_case_insensitive_and_preserves_server_casing() {
        // Match ignores case but the returned name keeps the server's exact casing,
        // since IMAP mailbox names are case-sensitive on the wire.
        let existing = entries(&["INBOX", "SENT"]);
        assert_eq!(select_sent_folder(&existing), "SENT");
    }

    #[test]
    fn select_sent_folder_matches_localized_german_name() {
        // IONOS/GMX-shaped German server: no English "Sent" at all. The sent
        // copy must APPEND into "Gesendete Objekte", not a phantom "Sent".
        let existing = entries(&["INBOX", "Gesendete Objekte", "Papierkorb"]);
        assert_eq!(select_sent_folder(&existing), "Gesendete Objekte");
    }

    #[test]
    fn select_sent_folder_prefers_special_use_attribute() {
        let existing = vec![
            ListedFolder {
                raw_name: "Postausgang".to_string(),
                delimiter: Some(".".to_string()),
                attributes: vec!["\\Sent".to_string()],
            },
            ListedFolder {
                raw_name: "Sent".to_string(),
                delimiter: Some(".".to_string()),
                attributes: Vec::new(),
            },
        ];
        assert_eq!(select_sent_folder(&existing), "Postausgang");
    }

    #[test]
    fn select_sent_folder_defaults_to_sent_when_none_match() {
        let existing = entries(&["INBOX", "Archive"]);
        assert_eq!(select_sent_folder(&existing), "Sent");
    }

    #[test]
    fn imap_folder_maps_to_mailbox_column() {
        // A message fetched from the Sent folder must be stored as mailbox='sent'
        // (not 'inbox'), otherwise it shows up in the Inbox view as well as Sent.
        assert_eq!(ImapFolder::Inbox.mailbox_value(), "inbox");
        assert_eq!(ImapFolder::Sent.mailbox_value(), "sent");
        assert_eq!(ImapFolder::Spam.mailbox_value(), "spam");
        assert_eq!(ImapFolder::Trash.mailbox_value(), "trash");
        // Custom folders carry their server path so the emails.mailbox column
        // can address each folder individually.
        assert_eq!(
            ImapFolder::Custom("INBOX.Patienten".to_string()).mailbox_value(),
            "folder:INBOX.Patienten"
        );
    }

    #[test]
    fn folder_message_id_roundtrips_through_parse() {
        let client = ImapClient::new(
            ImapCredentials {
                host: "imap.example.com".to_string(),
                port: 993,
                username: "u".to_string(),
                password: "p".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                smtp_port: 587,
            },
            "u@example.com".to_string(),
            "U".to_string(),
            "acc-1".to_string(),
        );
        // Paths containing the "::" id separator and non-ASCII wire names must
        // survive the roundtrip — the base64url segment shields them.
        for path in ["INBOX.Patienten", "Weird::Name", "Entw&APw-rfe.Alt"] {
            let id = client.make_folder_email_id(path, 42);
            let (folder, uid) = client.parse_message_ref(&id);
            assert_eq!(folder, ImapFolder::Custom(path.to_string()), "path {path}");
            assert_eq!(uid, "42");
        }
    }

    #[test]
    fn folder_email_id_prefix_matches_generated_ids() {
        let client = ImapClient::new(
            ImapCredentials {
                host: "imap.example.com".to_string(),
                port: 993,
                username: "u".to_string(),
                password: "p".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                smtp_port: 587,
            },
            "u@example.com".to_string(),
            "U".to_string(),
            "acc-1".to_string(),
        );
        let id = client.make_folder_email_id("INBOX.Kunden", 42);
        let prefix = folder_email_id_prefix("acc-1", "INBOX.Kunden");
        assert!(id.starts_with(&prefix), "{id} must start with {prefix}");
        assert_eq!(id.strip_prefix(&prefix), Some("42"));
    }

    #[test]
    fn canonical_prefixed_ids_still_parse_to_role_folders() {
        let client = ImapClient::new(
            ImapCredentials {
                host: "imap.example.com".to_string(),
                port: 993,
                username: "u".to_string(),
                password: "p".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                smtp_port: 587,
            },
            "u@example.com".to_string(),
            "U".to_string(),
            "acc-1".to_string(),
        );
        assert_eq!(client.parse_message_ref("acc-1::SENT::7"), (ImapFolder::Sent, "7"));
        assert_eq!(client.parse_message_ref("acc-1::SPAM::8"), (ImapFolder::Spam, "8"));
        assert_eq!(client.parse_message_ref("acc-1::TRASH::9"), (ImapFolder::Trash, "9"));
        assert_eq!(client.parse_message_ref("acc-1::123"), (ImapFolder::Inbox, "123"));
    }

    #[test]
    fn malformed_folder_id_falls_back_to_inbox() {
        let client = ImapClient::new(
            ImapCredentials {
                host: "imap.example.com".to_string(),
                port: 993,
                username: "u".to_string(),
                password: "p".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                smtp_port: 587,
            },
            "u@example.com".to_string(),
            "U".to_string(),
            "acc-1".to_string(),
        );
        // Garbage base64 segment or missing uid separator must not panic.
        let (folder, _) = client.parse_message_ref("acc-1::FOLDER::!!!::42");
        assert_eq!(folder, ImapFolder::Inbox);
        let (folder, _) = client.parse_message_ref("acc-1::FOLDER::noseparator");
        assert_eq!(folder, ImapFolder::Inbox);
    }
}
