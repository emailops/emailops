//! Connectivity probe.
//!
//! Periodically checks whether the device has actual internet connectivity by
//! issuing a cheap HTTP HEAD request to a neutral, high-availability endpoint.
//! The result is published two ways:
//!   * cached in an atomic flag readable via the `is_online` command (used by
//!     the frontend on startup and as a defensive fallback if it misses an
//!     event), and
//!   * emitted as an `app-connectivity-changed` Tauri event after every probe.
//!     Every probe, not just the transitions: the front-end store has no other
//!     way back from a spurious browser `offline` event (see `publish`).
//!
//! Why a probe instead of relying on `navigator.onLine`: the browser flag only
//! reflects "the OS thinks we have a link layer", which is wrong on captive
//! portals, VPN drops, and Wi-Fi with no internet upstream. A periodic 1-line
//! HEAD request is cheap and authoritative.
//!
//! Some networks block the public endpoint while the mail servers stay
//! reachable. When the HEAD request fails, the round falls back to a TCP
//! connect to the hosts of the configured accounts (no third party, no
//! payload), at most once a minute, and the app reports offline only after
//! two consecutive rounds found no route. The decision lives in the pure
//! [`ProbeState`] / [`mail_probe_targets`] planners; [`probe_round`] is the
//! thin executor.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::db::Database;
use crate::models::Account;
use crate::services::app_handle::AppHandle;
use serde::Serialize;
#[cfg(feature = "desktop")]
use tauri::Emitter;

/// Endpoint to probe. Picked because it is globally anycasted, returns small
/// responses, and is unaffected by individual provider outages (e.g. Gmail
/// down does not imply we're offline). HTTPS even though the probe carries no
/// data: the app sends nothing over a plaintext protocol (CASA/DASA 1.1.1),
/// and a successful handshake also rules out a captive portal answering for it.
const PROBE_URL: &str = "https://1.1.1.1/";
const PROBE_INTERVAL: Duration = Duration::from_secs(15);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// While the public probe keeps failing, the mail hosts are tried on every
/// 4th round (60 s at `PROBE_INTERVAL`), so a mail server does not take the
/// app for a scanner.
const MAIL_CHECK_EVERY_ROUNDS: u32 = 4;
/// Consecutive rounds with no route before the app reports offline: one
/// failed round is a blip, and a false offline leaves the user with no mail.
const OFFLINE_AFTER_ROUNDS: u32 = 2;

/// Hosts behind `sync::gmail::GMAIL_API_BASE` and `sync::outlook::GRAPH_API_BASE`.
const GMAIL_PROBE_HOST: (&str, u16) = ("gmail.googleapis.com", 443);
const GRAPH_PROBE_HOST: (&str, u16) = ("graph.microsoft.com", 443);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityEvent {
    pub online: bool,
}

#[derive(Clone)]
pub struct ConnectivityMonitor {
    /// Latest probe result. Optimistic default (true) so the UI doesn't show
    /// an offline banner before the first probe completes.
    online: Arc<AtomicBool>,
}

impl ConnectivityMonitor {
    /// No-op instance for tests: always reports online, no probe loop.
    pub fn stub() -> Self {
        Self {
            online: Arc::new(AtomicBool::new(true)),
        }
    }

    /// `db` supplies the account mail hosts the probe falls back to when the
    /// public endpoint is unreachable.
    pub fn start(app: AppHandle, db: Arc<Database>) -> Self {
        let online = Arc::new(AtomicBool::new(true));
        let monitor = Self { online: online.clone() };

        let client = match reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() {
            Ok(c) => c,
            Err(e) => {
                crate::services::logger::log(
                    "error",
                    "system",
                    format!("connectivity: failed to build HTTP client (probe disabled): {e}"),
                );
                return monitor;
            }
        };

        crate::runtime::spawn::spawn(async move {
            // Run one probe immediately so the cached state reflects reality
            // before the first PROBE_INTERVAL elapses.
            let mut state = probe_once(&client, &db, ProbeState::default(), &online, &app).await;
            let mut ticker = tokio::time::interval(PROBE_INTERVAL);
            // Skip the immediate tick that `interval` fires by default — we
            // already ran one probe above.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                state = probe_once(&client, &db, state, &online, &app).await;
            }
        });

        monitor
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    /// Shared handle to the cached online flag, for components that need to
    /// poll it from inside a hot loop without going through the monitor's
    /// `is_online()` method (e.g. the sync scheduler's poll loops, which want
    /// to skip a tick atomically without holding an Arc<ConnectivityMonitor>).
    pub fn online_flag(&self) -> Arc<AtomicBool> {
        self.online.clone()
    }
}

async fn probe_once(
    client: &reqwest::Client,
    db: &Database,
    state: ProbeState,
    online: &Arc<AtomicBool>,
    app: &AppHandle,
) -> ProbeState {
    let public_ok = match client.head(PROBE_URL).send().await {
        Ok(resp) => resp.status().is_success() || resp.status().is_redirection(),
        Err(_) => false,
    };
    let state = probe_round(state, public_ok, || load_mail_targets(db)).await;
    publish(online, state.online(), app);
    state
}

/// Cache the probe result and publish it to the front end.
///
/// Publishes on **every** probe, not only on transitions. The front-end
/// connectivity store treats this event as the authoritative signal, so it
/// needs a heartbeat it can re-converge on: a browser `offline` event that
/// never gets a matching `online` would otherwise pin the offline banner
/// forever while this side stayed happily online and never emitted again.
/// One event per `PROBE_INTERVAL` is not a meaningful cost.
fn publish(online: &AtomicBool, now_online: bool, app: &AppHandle) {
    online.store(now_online, Ordering::Relaxed);
    if let Err(e) = app.emit("app-connectivity-changed", ConnectivityEvent { online: now_online }) {
        crate::services::logger::log("error", "system", format!("connectivity: failed to emit event: {e}"));
    }
}

/// A mail host the fallback probe opens a TCP connection to: `(host, port)`.
pub(crate) type MailHost = (String, u16);

/// Pure planner for the probe rounds.
///
/// Each round tries the public endpoint first. When it fails, the round may
/// also try the account mail hosts, but only on the first failed round and
/// then every `MAIL_CHECK_EVERY_ROUNDS` rounds; between two checks the latest
/// mail-host result stands. The app reports offline once
/// `OFFLINE_AFTER_ROUNDS` consecutive rounds found no route.
///
/// Default = before the first round, which reports online (optimistic start).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ProbeState {
    /// Failed rounds left before the mail hosts are tried again; 0 means the
    /// next failed round tries them.
    rounds_until_mail_check: u32,
    /// Latest mail-host result in the current failure streak.
    mail_reachable: bool,
    /// Consecutive rounds with no route at all.
    no_route_rounds: u32,
}

impl ProbeState {
    /// Whether this round, whose public probe returned `public_ok`, must also
    /// try the mail hosts.
    pub(crate) fn should_check_mail_hosts(&self, public_ok: bool) -> bool {
        !public_ok && self.rounds_until_mail_check == 0
    }

    /// The state after a round. `mail_ok` is `None` when the round did not
    /// try the mail hosts.
    pub(crate) fn after_round(self, public_ok: bool, mail_ok: Option<bool>) -> Self {
        if public_ok {
            return Self::default();
        }
        let mail_reachable = mail_ok.unwrap_or(self.mail_reachable);
        Self {
            rounds_until_mail_check: match self.rounds_until_mail_check {
                0 => MAIL_CHECK_EVERY_ROUNDS - 1,
                n => n - 1,
            },
            mail_reachable,
            no_route_rounds: if mail_reachable {
                0
            } else {
                self.no_route_rounds.saturating_add(1)
            },
        }
    }

    pub(crate) fn online(&self) -> bool {
        self.no_route_rounds < OFFLINE_AFTER_ROUNDS
    }
}

/// Pure planner: the mail hosts a failed public probe falls back to, one per
/// enabled account (deduplicated, in account order). `imap_host` returns the
/// stored IMAP `(host, port)` of an account, `None` when it has none.
pub(crate) fn mail_probe_targets(accounts: &[Account], imap_host: &dyn Fn(&str) -> Option<MailHost>) -> Vec<MailHost> {
    let mut targets: Vec<MailHost> = Vec::new();
    for account in accounts.iter().filter(|a| a.enabled) {
        let target = match account.provider.as_str() {
            "imap" => imap_host(&account.id),
            "gmail" => Some((GMAIL_PROBE_HOST.0.to_string(), GMAIL_PROBE_HOST.1)),
            "outlook" => Some((GRAPH_PROBE_HOST.0.to_string(), GRAPH_PROBE_HOST.1)),
            _ => None,
        };
        if let Some(target) = target {
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
    }
    targets
}

/// The mail hosts of the stored accounts. A DB error is logged and leaves
/// the round with fewer targets, which at worst reads as no route.
fn load_mail_targets(db: &Database) -> Vec<MailHost> {
    let accounts = match db.list_accounts() {
        Ok(accounts) => accounts,
        Err(e) => {
            crate::services::logger::log(
                "error",
                "system",
                format!("connectivity: cannot list accounts for the mail-host probe: {e}"),
            );
            return Vec::new();
        }
    };
    mail_probe_targets(&accounts, &|account_id| match db.get_imap_settings(account_id) {
        Ok(settings) => settings.map(|(host, port, ..)| (host, port)),
        Err(e) => {
            crate::services::logger::log(
                "error",
                "system",
                format!("connectivity: cannot read IMAP settings for the mail-host probe: {e}"),
            );
            None
        }
    })
}

/// One probe round once the public probe has answered `public_ok`.
/// `load_targets` runs only when the round tries the mail hosts, so a normal
/// network costs no DB read and no extra connection.
async fn probe_round(state: ProbeState, public_ok: bool, load_targets: impl FnOnce() -> Vec<MailHost>) -> ProbeState {
    let mail_ok = if state.should_check_mail_hosts(public_ok) {
        Some(any_reachable(&load_targets()).await)
    } else {
        None
    };
    state.after_round(public_ok, mail_ok)
}

/// Whether any of `targets` accepts a TCP connection. Connects to all of them
/// at once and stops at the first success; each connection is closed without
/// sending a byte.
async fn any_reachable(targets: &[MailHost]) -> bool {
    // `select_ok` panics on an empty iterator.
    if targets.is_empty() {
        return false;
    }
    let attempts = targets.iter().map(|(host, port)| {
        Box::pin(async move {
            match tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect((host.as_str(), *port))).await {
                Ok(Ok(_stream)) => Ok(()),
                _ => Err(()),
            }
        })
    });
    futures::future::select_ok(attempts).await.is_ok()
}

#[cfg(test)]
mod planner_tests {
    use super::*;

    /// Runs `rounds` rounds whose public probe fails and whose mail check, when
    /// due, returns `mail_ok`. Returns the final state and the rounds (0-based)
    /// that tried the mail hosts.
    fn failed_rounds(mut state: ProbeState, rounds: usize, mail_ok: bool) -> (ProbeState, Vec<usize>) {
        let mut checked = Vec::new();
        for round in 0..rounds {
            let mail = if state.should_check_mail_hosts(false) {
                checked.push(round);
                Some(mail_ok)
            } else {
                None
            };
            state = state.after_round(false, mail);
        }
        (state, checked)
    }

    #[test]
    fn a_normal_network_never_touches_the_mail_hosts() {
        let mut state = ProbeState::default();
        for _ in 0..10 {
            assert!(
                !state.should_check_mail_hosts(true),
                "public probe answered: no mail-host connection"
            );
            state = state.after_round(true, None);
            assert!(state.online());
        }
    }

    #[test]
    fn the_first_failed_round_checks_the_mail_hosts() {
        assert!(ProbeState::default().should_check_mail_hosts(false));
    }

    #[test]
    fn mail_hosts_are_checked_every_fourth_failed_round() {
        let (_, checked) = failed_rounds(ProbeState::default(), 9, true);
        assert_eq!(checked, vec![0, 4, 8], "60 s between mail-host checks at a 15 s tick");
    }

    #[test]
    fn a_reachable_mail_host_keeps_the_app_online_between_checks() {
        let (state, _) = failed_rounds(ProbeState::default(), 7, true);
        assert!(state.online(), "the last mail-host success carries forward");
    }

    #[test]
    fn one_round_with_no_route_keeps_the_app_online() {
        let (state, _) = failed_rounds(ProbeState::default(), 1, false);
        assert!(state.online());
    }

    #[test]
    fn two_consecutive_rounds_with_no_route_report_offline() {
        let (state, _) = failed_rounds(ProbeState::default(), 2, false);
        assert!(!state.online());
    }

    #[test]
    fn a_mail_host_success_breaks_the_no_route_streak() {
        let state = ProbeState::default().after_round(false, Some(false));
        let (state, _) = failed_rounds(state, 4, true);
        assert!(
            state.online(),
            "round 4 found the mail host, so the streak never reached two"
        );
    }

    #[test]
    fn the_public_probe_answering_again_reports_online_in_that_round() {
        let (offline, _) = failed_rounds(ProbeState::default(), 3, false);
        assert!(!offline.online());

        assert!(offline.after_round(true, None).online());
    }

    #[test]
    fn a_mail_host_answering_again_reports_online_in_that_round() {
        let (offline, checked) = failed_rounds(ProbeState::default(), 4, false);
        assert!(!offline.online());
        assert_eq!(checked, vec![0]);

        assert!(offline.should_check_mail_hosts(false), "round 4 is a mail-host round");
        assert!(offline.after_round(false, Some(true)).online());
    }

    #[test]
    fn public_probe_recovery_restarts_the_mail_host_cadence() {
        let (state, _) = failed_rounds(ProbeState::default(), 2, true);
        let state = state.after_round(true, None);

        assert!(
            state.should_check_mail_hosts(false),
            "a new failure streak checks the mail hosts on its first round"
        );
    }

    fn account(id: &str, provider: &str, enabled: bool) -> Account {
        Account {
            id: id.to_string(),
            provider: provider.to_string(),
            email: format!("{id}@example.com"),
            name: id.to_string(),
            created_at: 0,
            sort_order: 0,
            enabled,
            sync_from_timestamp: None,
            color: crate::models::AccountColor::Blue,
        }
    }

    fn host(name: &str, port: u16) -> MailHost {
        (name.to_string(), port)
    }

    #[test]
    fn each_provider_maps_to_the_host_its_sync_talks_to() {
        let accounts = [
            account("a1", "imap", true),
            account("a2", "gmail", true),
            account("a3", "outlook", true),
        ];
        let imap = |id: &str| (id == "a1").then(|| host("imap.example.com", 993));

        assert_eq!(
            mail_probe_targets(&accounts, &imap),
            vec![
                host("imap.example.com", 993),
                host("gmail.googleapis.com", 443),
                host("graph.microsoft.com", 443),
            ]
        );
    }

    #[test]
    fn disabled_accounts_are_not_probed() {
        let accounts = [account("a1", "gmail", false), account("a2", "imap", false)];
        let imap = |_: &str| Some(host("imap.example.com", 993));

        assert!(mail_probe_targets(&accounts, &imap).is_empty());
    }

    #[test]
    fn a_host_shared_by_several_accounts_is_probed_once() {
        let accounts = [
            account("a1", "gmail", true),
            account("a2", "gmail", true),
            account("a3", "imap", true),
            account("a4", "imap", true),
        ];
        let imap = |_: &str| Some(host("imap.example.com", 993));

        assert_eq!(
            mail_probe_targets(&accounts, &imap),
            vec![host("gmail.googleapis.com", 443), host("imap.example.com", 993)]
        );
    }

    #[test]
    fn an_imap_account_without_stored_settings_is_skipped() {
        let accounts = [account("a1", "imap", true)];

        assert!(mail_probe_targets(&accounts, &|_: &str| None).is_empty());
    }

    #[test]
    fn no_accounts_means_no_targets() {
        assert!(mail_probe_targets(&[], &|_: &str| None).is_empty());
    }

    #[test]
    fn the_api_hosts_match_the_base_urls_sync_uses() {
        assert!(crate::sync::gmail::GMAIL_API_BASE.starts_with(&format!("https://{}/", GMAIL_PROBE_HOST.0)));
        assert!(crate::sync::outlook::GRAPH_API_BASE.starts_with(&format!("https://{}/", GRAPH_PROBE_HOST.0)));
    }
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    async fn listener() -> (TcpListener, MailHost) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        (listener, ("127.0.0.1".to_string(), port))
    }

    /// A port nothing listens on: bind one, then drop the listener.
    async fn closed_port() -> MailHost {
        let (listener, host) = listener().await;
        drop(listener);
        host
    }

    #[tokio::test]
    async fn one_accepting_host_is_enough() {
        let (_listener, open) = listener().await;
        let closed = closed_port().await;

        assert!(any_reachable(&[closed, open]).await);
    }

    #[tokio::test]
    async fn every_host_refusing_means_no_route() {
        let closed = closed_port().await;

        assert!(!any_reachable(&[closed]).await);
    }

    #[tokio::test]
    async fn no_targets_means_no_route() {
        assert!(!any_reachable(&[]).await);
    }

    #[tokio::test]
    async fn the_mail_host_receives_no_payload() {
        let (listener, open) = listener().await;

        assert!(any_reachable(&[open]).await);

        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut received = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut received))
            .await
            .expect("the probe closes the connection")
            .expect("read");
        assert!(
            received.is_empty(),
            "the probe must send nothing, got {} bytes",
            received.len()
        );
    }

    #[tokio::test]
    async fn a_round_whose_public_probe_answers_loads_no_mail_hosts() {
        let state = probe_round(ProbeState::default(), true, || {
            panic!("the mail hosts must not be loaded when the public probe answers")
        })
        .await;

        assert!(state.online());
    }

    #[tokio::test]
    async fn a_blocked_public_probe_with_a_reachable_mail_host_stays_online() {
        let (_listener, open) = listener().await;
        let mut state = ProbeState::default();

        for _ in 0..6 {
            let targets = vec![open.clone()];
            state = probe_round(state, false, move || targets).await;
            assert!(state.online());
        }
    }

    #[test]
    fn the_mail_hosts_come_from_the_stored_accounts() {
        let db = Database::new_for_testing().expect("db");
        let mut imap = crate::models::Account {
            id: "a1".to_string(),
            provider: "imap".to_string(),
            email: "a1@example.com".to_string(),
            name: "a1".to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
            color: crate::models::AccountColor::Blue,
        };
        db.insert_account(&imap).expect("insert imap");
        db.upsert_imap_settings("a1", "imap.example.com", 993, "a1", "smtp.example.com", 465)
            .expect("imap settings");
        imap.id = "a2".to_string();
        imap.provider = "gmail".to_string();
        imap.email = "a2@example.com".to_string();
        imap.sort_order = 1;
        db.insert_account(&imap).expect("insert gmail");

        assert_eq!(
            load_mail_targets(&db),
            vec![
                ("imap.example.com".to_string(), 993),
                ("gmail.googleapis.com".to_string(), 443)
            ]
        );
    }

    #[tokio::test]
    async fn with_no_accounts_a_failed_public_probe_goes_offline_after_two_rounds() {
        let state = probe_round(ProbeState::default(), false, Vec::new).await;
        assert!(state.online());

        let state = probe_round(state, false, Vec::new).await;
        assert!(!state.online());
    }
}

#[cfg(test)]
mod probe_url_tests {
    use super::PROBE_URL;

    #[test]
    fn probe_goes_over_tls() {
        // CASA/DASA 1.1.1: no application traffic may use a plaintext protocol,
        // even a payload-free reachability check.
        assert!(PROBE_URL.starts_with("https://"), "probe must use TLS, got {PROBE_URL}");
    }
}

// The stub `AppHandle` (which routes `emit` through the testable event sink)
// only exists without the `desktop` feature — same gate as `app_handle.rs`.
#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::*;
    use crate::services::events::{install_for_testing, seam_test_lock};

    #[test]
    fn publishes_every_probe_so_the_frontend_can_re_converge() {
        let _guard = seam_test_lock();
        let sink = install_for_testing();
        let online = AtomicBool::new(true);

        publish(&online, true, &AppHandle);
        publish(&online, true, &AppHandle);

        assert_eq!(
            sink.count("app-connectivity-changed"),
            2,
            "an unchanged result must still be published: it is the frontend's only heartbeat"
        );
    }

    #[test]
    fn publishes_the_probe_result_and_caches_it() {
        let _guard = seam_test_lock();
        let sink = install_for_testing();
        let online = AtomicBool::new(true);

        publish(&online, false, &AppHandle);

        assert!(
            !online.load(Ordering::Relaxed),
            "is_online must read back the latest probe"
        );
        let payloads = sink.payloads_for("app-connectivity-changed");
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0]["online"], serde_json::json!(false));
    }
}
