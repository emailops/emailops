//! App release update check.
//!
//! Periodically fetches the latest GitHub release and, when it is newer than
//! the running build, emits an `app-update-available` event the frontend
//! renders as a toast with a Download action. Split per the repo convention:
//! pure planners (`parse_version`, `should_check`, `plan_notification`), a
//! testable tick executor (`update_check_tick` — no `AppHandle`), and the
//! thin loop + HTTP fetch wired by the sync scheduler.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::db::Database;
use crate::models::error::AppError;

/// GitHub's `/releases/latest` endpoint excludes drafts and prereleases
/// server-side; `plan_notification` re-checks the flags anyway.
pub const GITHUB_LATEST_RELEASE_URL: &str = "https://api.github.com/repos/emailops/emailops/releases/latest";

/// Unix seconds of the last *successful* fetch. Failed fetches leave it
/// untouched so they retry on the next tick.
pub const PREF_UPDATE_LAST_CHECK_AT: &str = "app_update_last_check_at";

/// Normalized ("0.7.0", no `v` prefix) version we last emitted an event for.
/// Written at emit time so the same release never re-notifies.
pub const PREF_UPDATE_NOTIFIED_VERSION: &str = "app_update_notified_version";

/// Normalized version of the latest stable release seen on GitHub, written on
/// every successful fetch (newer or not). Backs the persistent sidebar link,
/// which — unlike the once-per-version toast — must survive app restarts.
pub const PREF_UPDATE_LATEST_VERSION: &str = "app_update_latest_version";

/// Release page URL (`html_url`) paired with [`PREF_UPDATE_LATEST_VERSION`].
pub const PREF_UPDATE_LATEST_URL: &str = "app_update_latest_url";

/// Minimum spacing between successful checks, enforced across restarts via
/// [`PREF_UPDATE_LAST_CHECK_AT`].
const CHECK_MIN_INTERVAL_SECS: i64 = 86_400;

/// The loop wakes hourly; most ticks no-op via [`should_check`]. Hourly (not
/// daily) so a failed fetch retries soon and a wall-clock day boundary is
/// crossed promptly in long-running sessions.
const TICK_INTERVAL: Duration = Duration::from_secs(3_600);

/// Stagger the first check past startup work, mirroring `vacuum_loop`.
const STARTUP_DELAY: Duration = Duration::from_secs(60);

/// The subset of a GitHub release object this feature reads.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
}

/// Payload of the `app-update-available` event — the frontend shows a toast
/// whose Download action opens `url` in the external browser.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAvailableEvent {
    /// Normalized dotted version, e.g. "0.7.0" (no `v` prefix).
    pub version: String,
    /// GitHub release page URL (`html_url`).
    pub url: String,
}

// ── Pure planners ─────────────────────────────────────────────────────────────

/// Parse a release tag or version string into `(major, minor, patch)`.
///
/// Accepts `0.7.0`, `v0.7.0`, `V0.7.0`, and short forms (`0.7` → `(0,7,0)`).
/// Anything else — prerelease suffixes, 4-part versions, empty or non-numeric
/// input — returns `None`, and `None` never notifies.
pub fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix(['v', 'V']).unwrap_or(trimmed);
    if stripped.is_empty() {
        return None;
    }
    let mut parts = [0u64; 3];
    for (i, segment) in stripped.split('.').enumerate() {
        if i >= 3 || segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        parts[i] = segment.parse().ok()?;
    }
    Some((parts[0], parts[1], parts[2]))
}

/// Whether enough time has passed since the last successful check.
pub fn should_check(now_secs: i64, last_check_at: Option<i64>) -> bool {
    match last_check_at {
        None => true,
        // A last-check in the future means the wall clock rolled back; treat
        // it as due so a bad timestamp can't disable checks forever.
        Some(last) => last > now_secs || now_secs - last >= CHECK_MIN_INTERVAL_SECS,
    }
}

/// Decide whether `release` warrants notifying the user.
pub fn plan_notification(
    release: &ReleaseInfo,
    current_version: &str,
    notified_version: Option<&str>,
) -> Option<UpdateAvailableEvent> {
    if release.draft || release.prerelease {
        return None;
    }
    let latest = parse_version(&release.tag_name)?;
    let current = parse_version(current_version)?;
    if latest <= current {
        return None;
    }
    if notified_version.and_then(parse_version) == Some(latest) {
        return None;
    }
    let (major, minor, patch) = latest;
    Some(UpdateAvailableEvent {
        version: format!("{major}.{minor}.{patch}"),
        url: release.html_url.clone(),
    })
}

/// Decide whether the stored latest-release prefs describe an update newer
/// than the running build. Unlike [`plan_notification`] this ignores the
/// notified marker — the sidebar link stays visible until the user actually
/// upgrades, while the toast fires only once per version.
pub fn plan_available_update(
    latest_version: Option<&str>,
    latest_url: Option<&str>,
    current_version: &str,
) -> Option<UpdateAvailableEvent> {
    let url = latest_url?;
    let latest = parse_version(latest_version?)?;
    let current = parse_version(current_version)?;
    if latest <= current {
        return None;
    }
    let (major, minor, patch) = latest;
    Some(UpdateAvailableEvent {
        version: format!("{major}.{minor}.{patch}"),
        url: url.to_string(),
    })
}

/// Read the persisted latest-release prefs and plan against them. Backs the
/// `get_available_update` command the frontend calls at startup.
pub fn available_update(db: &Database, current_version: &str) -> Option<UpdateAvailableEvent> {
    let read_pref = |key: &str| match db.get_preference(key) {
        Ok(value) => value,
        Err(e) => {
            crate::services::logger::log(
                "error",
                "system",
                format!("update check: failed to read {key} pref: {e}"),
            );
            None
        }
    };
    let latest_version = read_pref(PREF_UPDATE_LATEST_VERSION);
    let latest_url = read_pref(PREF_UPDATE_LATEST_URL);
    plan_available_update(latest_version.as_deref(), latest_url.as_deref(), current_version)
}

// ── Tick executor ─────────────────────────────────────────────────────────────

/// Boxed async function that fetches the latest release. Function-injection
/// seam (mirrors `SyncFn`) so tests never touch the network.
pub(crate) type FetchLatestFn =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<ReleaseInfo, String>> + Send>> + Send + Sync>;

/// One check attempt: gate on [`should_check`], fetch, persist prefs, and
/// return the event the caller should emit (`None` = nothing to do).
pub(crate) async fn update_check_tick(
    db: &Database,
    current_version: &str,
    fetch: &FetchLatestFn,
) -> Option<UpdateAvailableEvent> {
    let now = crate::services::clock::now_secs();
    let last_check_at = match db.get_preference(PREF_UPDATE_LAST_CHECK_AT) {
        Ok(value) => value.and_then(|s| s.parse::<i64>().ok()),
        Err(e) => {
            crate::services::logger::log(
                "error",
                "system",
                format!("update check: failed to read last-check pref: {e}"),
            );
            None
        }
    };
    if !should_check(now, last_check_at) {
        return None;
    }

    let release = match fetch().await {
        Ok(release) => release,
        // Debug level: transient network/API noise the user can't act on;
        // the next tick retries because last_check_at was not advanced.
        Err(e) => {
            crate::services::logger::log("debug", "system", format!("update check failed: {e}"));
            return None;
        }
    };

    if let Err(e) = db.set_preference(PREF_UPDATE_LAST_CHECK_AT, &now.to_string()) {
        crate::services::logger::log(
            "error",
            "system",
            format!("update check: failed to persist last-check pref: {e}"),
        );
    }

    // Persist the latest stable release (newer or not) so the persistent
    // sidebar link can be re-derived after a restart via `available_update`.
    if !release.draft && !release.prerelease {
        if let Some((major, minor, patch)) = parse_version(&release.tag_name) {
            let normalized = format!("{major}.{minor}.{patch}");
            if let Err(e) = db.set_preference(PREF_UPDATE_LATEST_VERSION, &normalized) {
                crate::services::logger::log(
                    "error",
                    "system",
                    format!("update check: failed to persist latest-version pref: {e}"),
                );
            }
            if let Err(e) = db.set_preference(PREF_UPDATE_LATEST_URL, &release.html_url) {
                crate::services::logger::log(
                    "error",
                    "system",
                    format!("update check: failed to persist latest-url pref: {e}"),
                );
            }
        }
    }

    let notified_version = match db.get_preference(PREF_UPDATE_NOTIFIED_VERSION) {
        Ok(value) => value,
        Err(e) => {
            crate::services::logger::log(
                "error",
                "system",
                format!("update check: failed to read notified pref: {e}"),
            );
            None
        }
    };
    let event = plan_notification(&release, current_version, notified_version.as_deref())?;

    // Written at emit-decision time so the same release never re-notifies,
    // even across restarts.
    if let Err(e) = db.set_preference(PREF_UPDATE_NOTIFIED_VERSION, &event.version) {
        crate::services::logger::log(
            "error",
            "system",
            format!("update check: failed to persist notified pref: {e}"),
        );
    }
    Some(event)
}

// ── Executors (loop + HTTP) ───────────────────────────────────────────────────

/// Build the reqwest-backed fetcher once at spawn time.
pub fn make_github_fetch() -> Result<FetchLatestFn, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        // GitHub rejects UA-less requests with 403.
        .user_agent(concat!("EmailOps/", env!("CARGO_PKG_VERSION")))
        .build()?;
    Ok(Arc::new(move || {
        let client = client.clone();
        Box::pin(async move {
            let response = client
                .get(GITHUB_LATEST_RELEASE_URL)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err(format!("GitHub API returned {}", response.status()));
            }
            response.json::<ReleaseInfo>().await.map_err(|e| e.to_string())
        })
    }))
}

/// Long-lived update-check loop, spawned by the sync scheduler. Offline ticks
/// are skipped silently, mirroring the other poll loops.
pub(crate) async fn update_check_loop(
    db: Arc<Database>,
    app: AppHandle,
    online_flag: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    fetch: FetchLatestFn,
) {
    tokio::time::sleep(STARTUP_DELAY).await;

    let mut ticker = tokio::time::interval(TICK_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        if stop_flag.load(Ordering::Relaxed) {
            return;
        }
        if !online_flag.load(Ordering::Relaxed) {
            continue;
        }
        if let Some(event) = update_check_tick(&db, env!("CARGO_PKG_VERSION"), &fetch).await {
            crate::services::logger::log(
                "info",
                "system",
                format!("Update available: EmailOps {} — {}", event.version, event.url),
            );
            let _ = app.emit("app-update-available", event);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    use crate::services::events::seam_test_lock as lock;
    use crate::services::{clock, logger};

    fn release(tag: &str) -> ReleaseInfo {
        ReleaseInfo {
            tag_name: tag.to_string(),
            html_url: format!("https://github.com/emailops/emailops/releases/tag/{tag}"),
            prerelease: false,
            draft: false,
        }
    }

    fn fake_fetch(calls: Arc<AtomicU32>, result: Result<ReleaseInfo, String>) -> FetchLatestFn {
        Arc::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            let result = result.clone();
            Box::pin(async move { result })
        })
    }

    fn restore_seams() {
        clock::install(Arc::new(clock::SystemClock));
        logger::install(Arc::new(logger::NoopLogger));
    }

    /// Drive one tick to completion on a throwaway single-thread runtime.
    /// The tick tests hold the global seam lock for their whole body; keeping
    /// them as sync `#[test]`s (instead of `#[tokio::test]`) means no
    /// `MutexGuard` is ever held across an await point
    /// (`clippy::await_holding_lock`). The fake fetch resolves immediately,
    /// so blocking here cannot stall.
    fn run_tick(db: &Database, current_version: &str, fetch: &FetchLatestFn) -> Option<UpdateAvailableEvent> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build test runtime")
            .block_on(update_check_tick(db, current_version, fetch))
    }

    // ── parse_version ─────────────────────────────────────────────────────────

    #[test]
    fn parse_version_accepts_plain_and_v_prefixed_tags() {
        assert_eq!(parse_version("0.7.0"), Some((0, 7, 0)));
        assert_eq!(parse_version("v0.7.0"), Some((0, 7, 0)));
        assert_eq!(parse_version("V0.7.0"), Some((0, 7, 0)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
    }

    #[test]
    fn parse_version_pads_short_forms_with_zeros() {
        assert_eq!(parse_version("0.7"), Some((0, 7, 0)));
        assert_eq!(parse_version("v2"), Some((2, 0, 0)));
    }

    #[test]
    fn parse_version_rejects_malformed_input() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("v"), None);
        assert_eq!(parse_version("0.7.0-beta.1"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("abc"), None);
        assert_eq!(parse_version("1..2"), None);
    }

    // ── plan_notification ─────────────────────────────────────────────────────

    #[test]
    fn newer_release_notifies_when_never_notified() {
        for tag in ["v0.6.3", "v0.7.0", "v1.0.0"] {
            let event = plan_notification(&release(tag), "0.6.2", None);
            assert!(event.is_some(), "{tag} is newer than 0.6.2 — must notify");
        }
    }

    #[test]
    fn version_comparison_is_numeric_not_lexicographic() {
        assert!(plan_notification(&release("v0.10.0"), "0.9.9", None).is_some());
        assert!(plan_notification(&release("v0.9.9"), "0.10.0", None).is_none());
    }

    #[test]
    fn equal_or_older_release_does_not_notify() {
        assert!(plan_notification(&release("v0.6.2"), "0.6.2", None).is_none());
        assert!(plan_notification(&release("v0.6.1"), "0.6.2", None).is_none());
    }

    #[test]
    fn already_notified_version_does_not_renotify() {
        // Stored normalized ("0.7.0") and defensively with a `v` prefix.
        assert!(plan_notification(&release("v0.7.0"), "0.6.2", Some("0.7.0")).is_none());
        assert!(plan_notification(&release("v0.7.0"), "0.6.2", Some("v0.7.0")).is_none());
    }

    #[test]
    fn newer_release_notifies_even_if_an_older_one_was_notified() {
        let event = plan_notification(&release("v0.8.0"), "0.6.2", Some("0.7.0"));
        assert!(
            event.is_some(),
            "a release newer than the notified one must notify again"
        );
    }

    #[test]
    fn prerelease_draft_and_unparseable_tags_never_notify() {
        let mut pre = release("v0.9.0");
        pre.prerelease = true;
        assert!(plan_notification(&pre, "0.6.2", None).is_none());

        let mut draft = release("v0.9.0");
        draft.draft = true;
        assert!(plan_notification(&draft, "0.6.2", None).is_none());

        assert!(plan_notification(&release("nightly"), "0.6.2", None).is_none());
    }

    #[test]
    fn event_carries_normalized_version_and_release_url() {
        let event = plan_notification(&release("v0.7.0"), "0.6.2", None);
        assert_eq!(
            event,
            Some(UpdateAvailableEvent {
                version: "0.7.0".to_string(),
                url: "https://github.com/emailops/emailops/releases/tag/v0.7.0".to_string(),
            })
        );
    }

    // ── should_check ──────────────────────────────────────────────────────────

    #[test]
    fn should_check_when_never_checked() {
        assert!(should_check(1_000_000, None));
    }

    #[test]
    fn should_not_check_within_24h_of_last_check() {
        let now = 1_000_000;
        assert!(!should_check(now, Some(now - 23 * 3_600)));
    }

    #[test]
    fn should_check_after_24h() {
        let now = 1_000_000;
        assert!(should_check(now, Some(now - 25 * 3_600)));
        assert!(should_check(now, Some(now - CHECK_MIN_INTERVAL_SECS)));
    }

    #[test]
    fn future_last_check_self_heals() {
        // Clock rollback (manual clock change) must not disable checks forever.
        assert!(should_check(1_000_000, Some(2_000_000)));
    }

    // ── ReleaseInfo deserialization ───────────────────────────────────────────

    #[test]
    fn release_info_deserializes_from_github_payload() {
        // Trimmed shape of a real /releases/latest response — extra fields
        // must be ignored, absent prerelease/draft default to false.
        let json = r#"{
            "url": "https://api.github.com/repos/emailops/emailops/releases/216836044",
            "html_url": "https://github.com/emailops/emailops/releases/tag/v0.7.0",
            "tag_name": "v0.7.0",
            "name": "EmailOps 0.7.0",
            "target_commitish": "main",
            "assets": [
                { "name": "EmailOps-macos.dmg", "size": 123 },
                { "name": "EmailOps-linux.AppImage", "size": 123 },
                { "name": "EmailOps-linux.deb", "size": 123 },
                { "name": "EmailOps-windows.msi", "size": 123 }
            ],
            "body": "Release notes"
        }"#;
        let info: ReleaseInfo = serde_json::from_str(json).expect("must deserialize");
        assert_eq!(info.tag_name, "v0.7.0");
        assert_eq!(
            info.html_url,
            "https://github.com/emailops/emailops/releases/tag/v0.7.0"
        );
        assert!(!info.prerelease);
        assert!(!info.draft);
    }

    #[test]
    fn release_info_honors_explicit_prerelease_and_draft_flags() {
        let json = r#"{ "tag_name": "v0.7.0-rc.1", "html_url": "https://x", "prerelease": true, "draft": true }"#;
        let info: ReleaseInfo = serde_json::from_str(json).expect("must deserialize");
        assert!(info.prerelease);
        assert!(info.draft);
    }

    // ── plan_available_update ─────────────────────────────────────────────────

    #[test]
    fn available_when_stored_latest_is_newer_than_current() {
        let event = plan_available_update(
            Some("0.7.0"),
            Some("https://github.com/emailops/emailops/releases/tag/v0.7.0"),
            "0.6.2",
        );
        assert_eq!(
            event,
            Some(UpdateAvailableEvent {
                version: "0.7.0".to_string(),
                url: "https://github.com/emailops/emailops/releases/tag/v0.7.0".to_string(),
            })
        );
    }

    #[test]
    fn not_available_when_stored_latest_is_equal_or_older() {
        assert!(plan_available_update(Some("0.6.2"), Some("https://x"), "0.6.2").is_none());
        assert!(plan_available_update(Some("0.6.1"), Some("https://x"), "0.6.2").is_none());
    }

    #[test]
    fn not_available_when_prefs_are_missing_or_unparseable() {
        assert!(plan_available_update(None, Some("https://x"), "0.6.2").is_none());
        assert!(plan_available_update(Some("0.7.0"), None, "0.6.2").is_none());
        assert!(plan_available_update(Some("nightly"), Some("https://x"), "0.6.2").is_none());
    }

    // ── available_update (DB-backed) ──────────────────────────────────────────

    #[test]
    fn available_update_reads_persisted_prefs() {
        let db = Database::new_for_testing().unwrap();
        assert!(available_update(&db, "0.6.2").is_none(), "no prefs → no update");

        db.set_preference(PREF_UPDATE_LATEST_VERSION, "0.7.0").unwrap();
        db.set_preference(
            PREF_UPDATE_LATEST_URL,
            "https://github.com/emailops/emailops/releases/tag/v0.7.0",
        )
        .unwrap();
        let event = available_update(&db, "0.6.2");
        assert_eq!(
            event,
            Some(UpdateAvailableEvent {
                version: "0.7.0".to_string(),
                url: "https://github.com/emailops/emailops/releases/tag/v0.7.0".to_string(),
            })
        );

        // Self-heals after the user upgrades: stored latest == new current.
        assert!(available_update(&db, "0.7.0").is_none());
    }

    // ── update_check_tick ─────────────────────────────────────────────────────

    #[test]
    fn tick_fetches_persists_prefs_and_returns_event_once() {
        let _g = lock();
        let clock = clock::install_for_testing(1_000_000);
        let _logger = logger::install_for_testing();
        let db = Database::new_for_testing().unwrap();
        let calls = Arc::new(AtomicU32::new(0));
        let fetch = fake_fetch(calls.clone(), Ok(release("v0.7.0")));

        // First tick: fetches, records the check, notifies.
        let event = run_tick(&db, "0.6.2", &fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            event,
            Some(UpdateAvailableEvent {
                version: "0.7.0".to_string(),
                url: "https://github.com/emailops/emailops/releases/tag/v0.7.0".to_string(),
            })
        );
        assert_eq!(
            db.get_preference(PREF_UPDATE_LAST_CHECK_AT).unwrap(),
            Some("1000000".to_string())
        );
        assert_eq!(
            db.get_preference(PREF_UPDATE_NOTIFIED_VERSION).unwrap(),
            Some("0.7.0".to_string())
        );
        // Latest-release prefs back the persistent sidebar link.
        assert_eq!(
            db.get_preference(PREF_UPDATE_LATEST_VERSION).unwrap(),
            Some("0.7.0".to_string())
        );
        assert_eq!(
            db.get_preference(PREF_UPDATE_LATEST_URL).unwrap(),
            Some("https://github.com/emailops/emailops/releases/tag/v0.7.0".to_string())
        );

        // Second tick within 24h: gated — no fetch, no event.
        let event = run_tick(&db, "0.6.2", &fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "must not fetch again within 24h");
        assert!(event.is_none());

        // 24h later, same latest release: fetches but stays silent.
        clock.advance_secs(CHECK_MIN_INTERVAL_SECS);
        let event = run_tick(&db, "0.6.2", &fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 2, "must fetch again after 24h");
        assert!(event.is_none(), "already-notified release must not re-notify");

        restore_seams();
    }

    #[test]
    fn tick_fetch_error_retries_next_time_and_logs_debug() {
        let _g = lock();
        clock::install_for_testing(1_000_000);
        let logger = logger::install_for_testing();
        let db = Database::new_for_testing().unwrap();
        let calls = Arc::new(AtomicU32::new(0));
        let fetch = fake_fetch(calls.clone(), Err("GitHub API returned 503".to_string()));

        let event = run_tick(&db, "0.6.2", &fetch);
        assert!(event.is_none());
        assert_eq!(
            db.get_preference(PREF_UPDATE_LAST_CHECK_AT).unwrap(),
            None,
            "failed fetch must not advance last_check_at"
        );
        assert_eq!(logger.count_by_level("debug"), 1, "failure is logged at debug level");

        // The gate still sees "never checked", so the next tick retries.
        let _ = run_tick(&db, "0.6.2", &fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        restore_seams();
    }

    #[test]
    fn tick_up_to_date_release_records_check_but_stays_silent() {
        let _g = lock();
        clock::install_for_testing(1_000_000);
        let _logger = logger::install_for_testing();
        let db = Database::new_for_testing().unwrap();
        let calls = Arc::new(AtomicU32::new(0));
        let fetch = fake_fetch(calls.clone(), Ok(release("v0.6.2")));

        let event = run_tick(&db, "0.6.2", &fetch);
        assert!(event.is_none());
        assert_eq!(
            db.get_preference(PREF_UPDATE_LAST_CHECK_AT).unwrap(),
            Some("1000000".to_string()),
            "successful fetch records the check even when nothing is newer"
        );
        assert_eq!(
            db.get_preference(PREF_UPDATE_NOTIFIED_VERSION).unwrap(),
            None,
            "no notification means no notified marker"
        );
        assert_eq!(
            db.get_preference(PREF_UPDATE_LATEST_VERSION).unwrap(),
            Some("0.6.2".to_string()),
            "latest-release info is persisted even when nothing is newer"
        );
        assert!(
            available_update(&db, "0.6.2").is_none(),
            "up-to-date build must not report an available update"
        );

        restore_seams();
    }
}
