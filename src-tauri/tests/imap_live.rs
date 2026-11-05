// `ImapClient` against a real IMAP/SMTP server: the local Dovecot + Mailpit +
// Toxiproxy of `scripts/imap_test.sh`. Every test returns early (and passes)
// when `EMAILOPS_IMAP_LIVE_HOST` is unset, so plain `cargo test` skips them.
//
// Run: make imap-live   (starts the servers, exports the env, runs this file)
//
// Each test logs in as a fresh user, so it starts from an empty mailbox.

use std::time::{Duration, Instant};

use emailops_lib::sync::imap_fake::synthetic_message;
use emailops_lib::sync::provider::{EmailBody, EmailProvider};
use emailops_lib::sync::provider_contract::{self as contract, ContractMailbox, LiveImapConfig, LiveImapMailbox};

fn live() -> Option<LiveImapConfig> {
    let config = LiveImapConfig::from_env();
    if config.is_none() {
        eprintln!("EMAILOPS_IMAP_LIVE_HOST unset: skipping live IMAP test (run `make imap-live`)");
    }
    config
}

// ── The shared provider contract ────────────────────────────────────────────

macro_rules! live_contract {
    ($($check:ident),* $(,)?) => {
        mod contract_on_live_server {
            use super::*;
            $(
                #[tokio::test(flavor = "multi_thread")]
                async fn $check() {
                    let Some(config) = live() else { return };
                    let mailbox = LiveImapMailbox::new(&config, stringify!($check));
                    if let Err(e) = contract::$check(&mailbox as &dyn ContractMailbox).await {
                        panic!("{e}");
                    }
                }
            )*
        }
    };
}

live_contract!(
    a_delivered_message_is_listed_in_the_inbox,
    a_fetched_message_carries_its_headers,
    the_star_round_trips,
    the_read_state_round_trips,
    archive_leaves_the_inbox_and_can_be_undone,
    spam_leaves_the_inbox,
    trash_leaves_the_inbox,
    a_message_in_place_is_located_under_its_id,
);

// ── Live-only behaviour ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_connection_passes_imap_and_smtp() {
    let Some(config) = live() else { return };
    let mailbox = LiveImapMailbox::new(&config, "probe");
    mailbox.client.test_connection().await.expect("IMAP + SMTP reachable");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_password_is_an_auth_failure() {
    let Some(mut config) = live() else { return };
    config.password = "not-the-password".to_string();
    let mailbox = LiveImapMailbox::new(&config, "wrongpw");
    let result = mailbox.client.login_raw().await;
    // login_raw files every connect failure under AuthError, so check that
    // the server itself refused the credentials.
    assert!(
        matches!(&result, Err(emailops_lib::models::error::AppError::AuthError(m)) if m.contains("AUTHENTICATIONFAILED")),
        "{result:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sending_reaches_the_relay_and_files_a_copy_in_sent() {
    let Some(config) = live() else { return };
    let mailbox = LiveImapMailbox::new(&config, "send");
    let subject = format!("Live send {}", uuid::Uuid::new_v4().simple());

    mailbox
        .client
        .send_new_email(
            &mailbox.credentials.username,
            Some("Owner"),
            &["friend@example.test".to_string()],
            &[],
            &subject,
            &EmailBody::plain("Hello from the live test."),
            &[],
        )
        .await
        .expect("send");

    let mailpit = std::env::var("EMAILOPS_IMAP_LIVE_MAILPIT").unwrap_or_else(|_| "http://127.0.0.1:8025".to_string());
    let found = poll(Duration::from_secs(10), || async {
        let url = format!(
            "{mailpit}/api/v1/search?query=subject:%22{}%22",
            subject.replace(' ', "%20")
        );
        let body: serde_json::Value = reqwest::get(&url).await.ok()?.json().await.ok()?;
        (body["messages_count"].as_u64()? > 0).then_some(())
    })
    .await;
    assert!(found.is_some(), "Mailpit never received {subject:?}");

    let sent = mailbox.client.list_mailbox_identities("sent").await.expect("list sent");
    assert_eq!(sent.len(), 1, "one copy filed in Sent: {sent:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_mailbox_lists_and_downloads_completely() {
    let Some(config) = live() else { return };
    let mailbox = LiveImapMailbox::new(&config, "bulk");
    let count = 120;
    let credentials = mailbox.credentials.clone();
    tokio::task::spawn_blocking(move || {
        let seeder = LiveImapMailbox::with_credentials(credentials);
        for n in 0..count {
            let message_id = format!("<bulk-{n}@example.test>");
            let raw = synthetic_message(
                &message_id,
                &format!("Bulk {n}"),
                "a@example.test",
                "b@example.test",
                1_793_000_000 + n,
            );
            seeder.append("INBOX", &raw, &message_id).expect("append");
        }
    })
    .await
    .expect("seed");

    let mut ids = Vec::new();
    let mut page = None;
    loop {
        let (refs, next) = mailbox
            .client
            .list_messages(50, page.as_deref(), None, None, None)
            .await
            .expect("list");
        ids.extend(refs.into_iter().map(|r| r.id));
        match next {
            Some(token) => page = Some(token),
            None => break,
        }
    }
    let inbox: Vec<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| !id.contains("::SENT::"))
        .collect();
    assert_eq!(inbox.len(), count as usize);
    let fetched = mailbox.client.batch_get_messages(&inbox[..20]).await.expect("batch");
    assert!(fetched.iter().all(Result::is_ok), "{fetched:?}");
}

// ── Faults through Toxiproxy ────────────────────────────────────────────────

struct Toxiproxy {
    api: String,
}

impl Toxiproxy {
    fn from_env() -> Option<(Self, u16)> {
        let api = std::env::var("EMAILOPS_IMAP_LIVE_TOXIPROXY").ok()?;
        let port = std::env::var("EMAILOPS_IMAP_LIVE_PROXY_PORT").ok()?.parse().ok()?;
        Some((Self { api }, port))
    }

    async fn set_enabled(&self, enabled: bool) {
        reqwest::Client::new()
            .post(format!("{}/proxies/imap", self.api))
            .json(&serde_json::json!({ "enabled": enabled }))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .expect("toxiproxy: toggle proxy");
    }

    async fn add_toxic(&self, name: &str, kind: &str, stream: &str, attributes: serde_json::Value) {
        reqwest::Client::new()
            .post(format!("{}/proxies/imap/toxics", self.api))
            .json(&serde_json::json!({ "name": name, "type": kind, "stream": stream, "attributes": attributes }))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .expect("toxiproxy: add toxic");
    }

    async fn remove_toxic(&self, name: &str) {
        let _ = reqwest::Client::new()
            .delete(format!("{}/proxies/imap/toxics/{name}", self.api))
            .send()
            .await;
    }
}

/// The proxy is shared, so fault tests must not overlap.
static PROXY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn proxied(config: &LiveImapConfig, label: &str, port: u16) -> LiveImapMailbox {
    let mut mailbox = LiveImapMailbox::new(config, label);
    mailbox.credentials.port = port;
    LiveImapMailbox::with_credentials(mailbox.credentials)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_comes_back_within_the_retry_window_is_reached() {
    let Some(config) = live() else { return };
    let Some((proxy, port)) = Toxiproxy::from_env() else {
        return;
    };
    let _guard = PROXY_LOCK.lock().await;
    let mailbox = proxied(&config, "retry", port);

    proxy.set_enabled(false).await;
    let reenable = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        proxy.set_enabled(true).await;
    });
    let started = Instant::now();
    let folders = mailbox.client.list_folders().await;
    reenable.await.expect("re-enable");

    assert!(folders.is_ok(), "{folders:?}");
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "the first attempt cannot have succeeded"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_link_still_completes() {
    let Some(config) = live() else { return };
    let Some((proxy, port)) = Toxiproxy::from_env() else {
        return;
    };
    let _guard = PROXY_LOCK.lock().await;
    let mailbox = proxied(&config, "slow", port);
    let id = mailbox.seed_inbox("<slow@example.test>", "Slow").await.expect("seed");

    proxy
        .add_toxic(
            "latency",
            "latency",
            "downstream",
            serde_json::json!({ "latency": 700, "jitter": 200 }),
        )
        .await;
    let result = mailbox.client.get_message(&id).await;
    proxy.remove_toxic("latency").await;

    assert!(result.is_ok(), "{result:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_reset_mid_operation_fails_it_and_the_next_one_reconnects() {
    let Some(config) = live() else { return };
    let Some((proxy, port)) = Toxiproxy::from_env() else {
        return;
    };
    let _guard = PROXY_LOCK.lock().await;
    let mailbox = proxied(&config, "reset", port);
    let id = mailbox.seed_inbox("<reset@example.test>", "Reset").await.expect("seed");
    // Opens the session the client keeps.
    mailbox.client.get_message(&id).await.expect("warm up");

    proxy
        .add_toxic("reset", "reset_peer", "downstream", serde_json::json!({ "timeout": 0 }))
        .await;
    let during = mailbox.client.set_starred(&id, true).await;
    proxy.remove_toxic("reset").await;
    assert!(
        during.is_err(),
        "the reset must surface as an error, not silently succeed"
    );

    mailbox
        .client
        .set_starred(&id, true)
        .await
        .expect("reconnects after the reset");
    let (email, _, _) = mailbox.client.get_message(&id).await.expect("get");
    assert!(email.is_starred);
}

async fn poll<F, Fut, T>(within: Duration, mut probe: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if let Some(value) = probe().await {
            return Some(value);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    None
}
