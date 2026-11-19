//! Behaviour every `EmailProvider` must share, written once and run against
//! each implementation: the `FakeEmailProvider` the service tests rely on, an
//! `ImapClient` over the in-memory [`FakeImapServer`], and an `ImapClient`
//! against a real server (`make imap-e2e`). A check that passes on the fakes
//! and fails on the real server means a fake has drifted from reality.
//!
//! Each check takes a [`ContractMailbox`] — the provider plus a way to put a
//! message in its inbox from the server side — and returns `Err` with what
//! went wrong. `tests/provider_contract.rs` and `tests/imap_live.rs` turn each
//! one into a test per implementation.
//!
//! Test infrastructure only: compiled into test and debug builds.

use std::sync::Arc;

use async_trait::async_trait;

use crate::models::Email;
use crate::sync::imap::{ImapClient, ImapCredentials};
use crate::sync::imap_fake::{synthetic_message, FakeImapServer};
use crate::sync::imap_transport::{MailTransport, NativeTlsTransport};
use crate::sync::provider::{EmailCategory, EmailProvider, FakeEmailProvider, MoveTarget};

/// A provider under test plus server-side access to its inbox.
#[async_trait]
pub trait ContractMailbox: Send + Sync {
    fn provider(&self) -> &dyn EmailProvider;

    /// Deliver a synthetic message with `message_id` to the inbox, the way
    /// mail arrives — not through the provider. Returns the provider's id for it.
    async fn seed_inbox(&self, message_id: &str, subject: &str) -> Result<String, String>;
}

/// Arrival time of seeded messages.
pub const SEED_TIME: i64 = 1_793_000_000;

const SENDER: &str = "sender@example.test";
const OWNER: &str = "owner@example.test";

fn check(condition: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(what())
    }
}

async fn inbox_ids(mailbox: &dyn ContractMailbox) -> Result<Vec<String>, String> {
    let (refs, _) = mailbox
        .provider()
        .list_messages(500, None, None, None, None)
        .await
        .map_err(|e| format!("list_messages: {e}"))?;
    Ok(refs.into_iter().map(|r| r.id).collect())
}

/// A delivered message is listed in the inbox.
pub async fn a_delivered_message_is_listed_in_the_inbox(mailbox: &dyn ContractMailbox) -> Result<(), String> {
    let id = mailbox.seed_inbox("<listed@example.test>", "Listed").await?;
    let ids = inbox_ids(mailbox).await?;
    check(ids.contains(&id), || format!("{id} missing from inbox listing {ids:?}"))
}

/// `get_message` returns the subject, sender and Message-ID, unread and unstarred.
pub async fn a_fetched_message_carries_its_headers(mailbox: &dyn ContractMailbox) -> Result<(), String> {
    let id = mailbox
        .seed_inbox("<headers@example.test>", "Quarterly numbers")
        .await?;
    let (email, _, _) = mailbox
        .provider()
        .get_message(&id)
        .await
        .map_err(|e| format!("get_message: {e}"))?;
    check(email.subject == "Quarterly numbers", || {
        format!("subject {:?}", email.subject)
    })?;
    check(email.sender_email == SENDER, || {
        format!("sender {:?}", email.sender_email)
    })?;
    check(email.message_id.as_deref() == Some("<headers@example.test>"), || {
        format!("message_id {:?}", email.message_id)
    })?;
    check(!email.is_read, || "a new message came back read".to_string())?;
    check(!email.is_starred, || "a new message came back starred".to_string())
}

/// Starring and unstarring are what the next fetch reports.
pub async fn the_star_round_trips(mailbox: &dyn ContractMailbox) -> Result<(), String> {
    let id = mailbox.seed_inbox("<star@example.test>", "Star me").await?;
    let provider = mailbox.provider();
    for starred in [true, false] {
        provider
            .set_starred(&id, starred)
            .await
            .map_err(|e| format!("set_starred({starred}): {e}"))?;
        let (email, _, _) = provider
            .get_message(&id)
            .await
            .map_err(|e| format!("get_message: {e}"))?;
        check(email.is_starred == starred, || {
            format!(
                "after set_starred({starred}) the message reads is_starred={}",
                email.is_starred
            )
        })?;
    }
    Ok(())
}

/// Marking read and unread is what the next fetch reports.
pub async fn the_read_state_round_trips(mailbox: &dyn ContractMailbox) -> Result<(), String> {
    let id = mailbox.seed_inbox("<read@example.test>", "Read me").await?;
    let provider = mailbox.provider();
    for read in [true, false] {
        provider
            .set_read_state(&id, read)
            .await
            .map_err(|e| format!("set_read_state({read}): {e}"))?;
        let (email, _, _) = provider
            .get_message(&id)
            .await
            .map_err(|e| format!("get_message: {e}"))?;
        check(email.is_read == read, || {
            format!(
                "after set_read_state({read}) the message reads is_read={}",
                email.is_read
            )
        })?;
    }
    Ok(())
}

/// Archiving takes the message out of the inbox; it can be found at the
/// location returned, and moved back.
pub async fn archive_leaves_the_inbox_and_can_be_undone(mailbox: &dyn ContractMailbox) -> Result<(), String> {
    let header = "<archive@example.test>";
    let id = mailbox.seed_inbox(header, "Archive me").await?;
    let provider = mailbox.provider();

    let location = provider
        .archive_message(&id, Some(header))
        .await
        .map_err(|e| format!("archive_message: {e}"))?;
    let ids = inbox_ids(mailbox).await?;
    check(!ids.contains(&id), || {
        format!("{id} still in the inbox after archiving")
    })?;
    check(location.mailbox != "inbox", || {
        "archive reported the inbox as its location".to_string()
    })?;

    let found = provider
        .locate_message(&location.id, Some(header))
        .await
        .map_err(|e| format!("locate_message: {e}"))?;
    check(found.is_some(), || {
        format!("nothing at the archived location {}", location.id)
    })?;

    provider
        .move_message(&location.id, Some(header), &MoveTarget::Inbox)
        .await
        .map_err(|e| format!("move_message(Inbox): {e}"))?;
    let back = inbox_messages_with_header(mailbox, header).await?;
    check(back == 1, || format!("{back} copies in the inbox after moving it back"))
}

/// Moving to spam takes the message out of the inbox into the `spam` mailbox.
pub async fn spam_leaves_the_inbox(mailbox: &dyn ContractMailbox) -> Result<(), String> {
    let header = "<spam@example.test>";
    let id = mailbox.seed_inbox(header, "Cheap watches").await?;
    let location = mailbox
        .provider()
        .move_to_spam(&id, Some(header))
        .await
        .map_err(|e| format!("move_to_spam: {e}"))?;
    check(location.mailbox == "spam", || {
        format!("spam location {:?}", location.mailbox)
    })?;
    let left = inbox_messages_with_header(mailbox, header).await?;
    check(left == 0, || format!("{left} copies still in the inbox"))
}

/// Trashing takes the message out of the inbox.
pub async fn trash_leaves_the_inbox(mailbox: &dyn ContractMailbox) -> Result<(), String> {
    let header = "<trash@example.test>";
    let id = mailbox.seed_inbox(header, "Delete me").await?;
    mailbox
        .provider()
        .trash_message(&id, Some(header))
        .await
        .map_err(|e| format!("trash_message: {e}"))?;
    let left = inbox_messages_with_header(mailbox, header).await?;
    check(left == 0, || format!("{left} copies still in the inbox"))
}

/// A message that has not moved is located under its own id.
pub async fn a_message_in_place_is_located_under_its_id(mailbox: &dyn ContractMailbox) -> Result<(), String> {
    let header = "<inplace@example.test>";
    let id = mailbox.seed_inbox(header, "Stay").await?;
    let found = mailbox
        .provider()
        .locate_message(&id, Some(header))
        .await
        .map_err(|e| format!("locate_message: {e}"))?;
    check(found.as_ref().map(|l| l.id.as_str()) == Some(id.as_str()), || {
        format!("located {found:?}, expected {id}")
    })
}

/// How many inbox messages carry `header` as their Message-ID.
async fn inbox_messages_with_header(mailbox: &dyn ContractMailbox, header: &str) -> Result<usize, String> {
    let mut count = 0;
    for id in inbox_ids(mailbox).await? {
        let (email, _, _) = mailbox
            .provider()
            .get_message(&id)
            .await
            .map_err(|e| format!("get_message({id}): {e}"))?;
        if email.message_id.as_deref() == Some(header) {
            count += 1;
        }
    }
    Ok(count)
}

// ── Implementations ─────────────────────────────────────────────────────────

/// `FakeEmailProvider`, seeded through `add_message`.
pub struct FakeProviderMailbox {
    pub provider: FakeEmailProvider,
}

impl FakeProviderMailbox {
    pub fn new() -> Self {
        Self {
            provider: FakeEmailProvider::new(OWNER, "Owner"),
        }
    }
}

impl Default for FakeProviderMailbox {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContractMailbox for FakeProviderMailbox {
    fn provider(&self) -> &dyn EmailProvider {
        &self.provider
    }

    async fn seed_inbox(&self, message_id: &str, subject: &str) -> Result<String, String> {
        let id = format!("fake-{}", message_id.trim_matches(['<', '>']));
        self.provider.add_message(
            Email {
                id: id.clone(),
                account_id: "acct".to_string(),
                thread_id: format!("thread-{id}"),
                message_id: Some(message_id.to_string()),
                references: None,
                subject: subject.to_string(),
                sender: "Sender".to_string(),
                sender_email: SENDER.to_string(),
                recipients: vec![OWNER.to_string()],
                cc: vec![],
                body: format!("Body of {subject}."),
                snippet: format!("Body of {subject}."),
                timestamp: SEED_TIME,
                is_read: false,
                triage_status: None,
                category: "primary".to_string(),
                mailbox: "inbox".to_string(),
                is_sent: false,
                is_starred: false,
                headers: None,
            },
            EmailCategory::Primary,
            vec![],
        );
        Ok(id)
    }
}

/// An `ImapClient` over the in-memory [`FakeImapServer`].
pub struct FakeImapMailbox {
    pub server: FakeImapServer,
    pub client: ImapClient,
}

impl FakeImapMailbox {
    pub fn new(server: FakeImapServer) -> Self {
        let client = ImapClient::new(
            ImapCredentials {
                host: "imap.example.test".to_string(),
                port: 993,
                username: OWNER.to_string(),
                password: "secret".to_string(),
                smtp_host: "smtp.example.test".to_string(),
                smtp_port: 465,
            },
            OWNER.to_string(),
            "Owner".to_string(),
            "acct".to_string(),
        )
        .with_transport(server.transport());
        Self { server, client }
    }
}

#[async_trait]
impl ContractMailbox for FakeImapMailbox {
    fn provider(&self) -> &dyn EmailProvider {
        &self.client
    }

    async fn seed_inbox(&self, message_id: &str, subject: &str) -> Result<String, String> {
        let raw = synthetic_message(message_id, subject, SENDER, OWNER, SEED_TIME);
        let uid = self.server.deliver("INBOX", &raw, &[], SEED_TIME);
        Ok(format!("acct::{uid}"))
    }
}

/// Where the live IMAP server of `make imap-e2e` listens, from the env vars
/// `scripts/imap_test.sh env` prints.
#[derive(Debug, Clone)]
pub struct LiveImapConfig {
    pub host: String,
    pub port: u16,
    pub smtp_port: u16,
    pub password: String,
}

impl LiveImapConfig {
    /// `None` when `EMAILOPS_IMAP_LIVE_HOST` is unset: no live server.
    pub fn from_env() -> Option<Self> {
        let host = std::env::var("EMAILOPS_IMAP_LIVE_HOST").ok()?;
        let port = |name: &str, default: u16| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
        Some(Self {
            host,
            port: port("EMAILOPS_IMAP_LIVE_PORT", 31143),
            smtp_port: port("EMAILOPS_IMAP_LIVE_SMTP_PORT", 31587),
            password: std::env::var("EMAILOPS_IMAP_LIVE_PASSWORD").unwrap_or_else(|_| "emailops-test".to_string()),
        })
    }

    /// Credentials for `username`, over `port` (the proxy port for fault tests).
    pub fn credentials(&self, username: &str, port: u16) -> ImapCredentials {
        ImapCredentials {
            host: self.host.clone(),
            port,
            username: username.to_string(),
            password: self.password.clone(),
            smtp_host: self.host.clone(),
            smtp_port: self.smtp_port,
        }
    }
}

/// A username nobody has used: the live server gives every new login an
/// empty mailbox, so each test starts clean.
pub fn fresh_username(label: &str) -> String {
    format!("{label}-{}@example.test", uuid::Uuid::new_v4().simple())
}

/// An `ImapClient` against the live test server, as a fresh user.
pub struct LiveImapMailbox {
    pub client: ImapClient,
    pub credentials: ImapCredentials,
    transport: Arc<dyn MailTransport>,
}

impl LiveImapMailbox {
    pub fn new(config: &LiveImapConfig, label: &str) -> Self {
        Self::with_credentials(config.credentials(&fresh_username(label), config.port))
    }

    /// A client for an existing login, e.g. the same user through another port.
    pub fn with_credentials(credentials: ImapCredentials) -> Self {
        let client = ImapClient::new(
            credentials.clone(),
            credentials.username.clone(),
            "Owner".to_string(),
            "acct".to_string(),
        );
        Self {
            client,
            credentials,
            transport: Arc::new(NativeTlsTransport::from_env()),
        }
    }

    /// `APPEND` `raw` to `folder` over a separate session; returns its UID.
    /// Blocking: call from `spawn_blocking` in async code.
    pub fn append(&self, folder: &str, raw: &[u8], message_id: &str) -> Result<u32, String> {
        append_blocking(self.transport.as_ref(), &self.credentials, folder, raw, message_id)
    }
}

fn append_blocking(
    transport: &dyn MailTransport,
    credentials: &ImapCredentials,
    folder: &str,
    raw: &[u8],
    message_id: &str,
) -> Result<u32, String> {
    let mut session = ImapClient::connect_sync(transport, credentials).map_err(|e| format!("connect: {e}"))?;
    session
        .append(folder, raw)
        .map_err(|e| format!("APPEND to {folder}: {e}"))?;
    crate::sync::imap_search::select(&mut session, folder).map_err(|e| format!("SELECT {folder}: {e}"))?;
    let uids = crate::sync::imap_search::uid_search(&mut session, &format!("HEADER Message-ID \"{message_id}\""))
        .map_err(|e| format!("SEARCH: {e}"))?;
    let _ = session.logout();
    uids.into_iter()
        .max()
        .ok_or_else(|| format!("{message_id} not found after APPEND"))
}

#[async_trait]
impl ContractMailbox for LiveImapMailbox {
    fn provider(&self) -> &dyn EmailProvider {
        &self.client
    }

    async fn seed_inbox(&self, message_id: &str, subject: &str) -> Result<String, String> {
        let raw = synthetic_message(message_id, subject, SENDER, OWNER, SEED_TIME);
        let message_id = message_id.to_string();
        let transport = Arc::clone(&self.transport);
        let credentials = self.credentials.clone();
        let uid = tokio::task::spawn_blocking(move || {
            append_blocking(transport.as_ref(), &credentials, "INBOX", &raw, &message_id)
        })
        .await
        .map_err(|e| format!("seed task: {e}"))??;
        Ok(format!("acct::{uid}"))
    }
}
