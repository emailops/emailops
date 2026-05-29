use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::db::Database;
use crate::models::error::AppError;
use crate::models::{Account, AppLogEvent};
use crate::services::task_queue::TaskQueue;

// In-memory per-account dedup: stores the last error message we emitted to
// the output panel. Suppresses repeated identical "auto-sync failed" logs
// every 60 s when the underlying state cannot change without user action
// (e.g. NeedsReauth — keyring entry missing). Cleared on the first
// successful sync after recovery, or explicitly after re-authentication.
static LAST_SYNC_ERROR: std::sync::LazyLock<RwLock<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// Drop the dedup state for an account so the next sync attempt logs/emits
/// regardless of prior error history. Call after re-authentication so the
/// user sees a fresh log line if a different problem now surfaces.
pub fn clear_sync_error_dedup(account_id: &str) {
    let mut guard = LAST_SYNC_ERROR.write().unwrap_or_else(PoisonError::into_inner);
    guard.remove(account_id);
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// Boxed async function that runs a single sync cycle.
pub(crate) type SyncFn = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

// ── Public API ────────────────────────────────────────────────────────────────

/// Background sync scheduler.
///
/// - Gmail: polls every 60 s
/// - IMAP: maintains IDLE connection, syncs on server push; exponential reconnect backoff
pub struct SyncScheduler {
    handles: Vec<tauri::async_runtime::JoinHandle<()>>,
    /// Stop flags passed to IMAP blocking threads so they exit without waiting
    /// for the full IDLE timeout.
    stop_flags: Vec<Arc<AtomicBool>>,
}

impl SyncScheduler {
    /// No-op instance for tests: no background tasks, `stop()` is safe to call.
    pub fn stub() -> Self {
        Self {
            handles: Vec::new(),
            stop_flags: Vec::new(),
        }
    }

    /// Start per-account background watchers for all enabled accounts.
    ///
    /// `online_flag` is the cached connectivity probe result from
    /// `ConnectivityMonitor`. Both the Gmail poll loop and the IMAP IDLE
    /// watcher consult it before initiating any network work; while offline
    /// they simply wait, so we don't emit HTTP-failure noise that duplicates
    /// what the offline banner already tells the user.
    pub fn start(
        db: Arc<Database>,
        app_data_dir: PathBuf,
        app: AppHandle,
        ai_background: TaskQueue,
        sync_abort_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
        sync_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
        online_flag: Arc<AtomicBool>,
    ) -> Self {
        let accounts = db.list_accounts().unwrap_or_default();
        let mut handles = Vec::new();
        let mut stop_flags = Vec::new();
        let enabled: Vec<Account> = enabled_accounts(accounts);

        // Recover from prior-process crashes: if a previous run of the app was
        // killed or panicked mid-sync, sync_state is still "syncing" on disk.
        // `gmail_poll_loop` skips every tick while status == "syncing", so
        // without this reset the account would never sync until the user
        // manually triggered it. Reset to "idle" for all enabled accounts at
        // startup — no sync is actually in progress in this fresh process.
        for account in &enabled {
            if let Ok(status) = db.get_sync_status(&account.id) {
                if status.status == "syncing" {
                    if let Err(e) = db.upsert_sync_status(
                        &account.id,
                        "idle",
                        status.last_sync_at,
                        Some("Previous session ended unexpectedly"),
                    ) {
                        crate::services::logger::log(
                            "error",
                            "sync",
                            format!("failed to reset stuck sync_status on startup: {e}"),
                        );
                    }
                }
            }
        }

        for account in &enabled {
            let flag = Arc::new(AtomicBool::new(false));
            stop_flags.push(flag.clone());
            handles.push(spawn_watcher(
                account.clone(),
                db.clone(),
                app_data_dir.clone(),
                app.clone(),
                ai_background.clone(),
                flag,
                sync_abort_flags.clone(),
                sync_locks.clone(),
                online_flag.clone(),
            ));
        }
        // One consolidation ticker per account. Cheap: it no-ops quickly when
        // the memory subsystem is disabled or nothing needs consolidating.
        for account in &enabled {
            let flag = Arc::new(AtomicBool::new(false));
            stop_flags.push(flag.clone());
            handles.push(tauri::async_runtime::spawn(memory_consolidation_loop(
                db.clone(),
                app.clone(),
                account.id.clone(),
                flag,
            )));
        }

        // Single global VACUUM / WAL-truncate ticker. Runs every 30 minutes
        // and is best-effort: contention is swallowed inside the DB helpers
        // so a busy writer never blocks startup or shutdown.
        {
            let flag = Arc::new(AtomicBool::new(false));
            stop_flags.push(flag.clone());
            handles.push(tauri::async_runtime::spawn(vacuum_loop(db.clone(), app.clone(), flag)));
        }

        Self { handles, stop_flags }
    }

    /// Signal all background tasks to stop and abort their async handles.
    /// IMAP blocking threads will exit within one IDLE timeout cycle (≤ 30 s).
    pub fn stop(&self) {
        for flag in &self.stop_flags {
            flag.store(true, Ordering::Relaxed);
        }
        for h in &self.handles {
            h.abort();
        }
    }

    pub fn task_count(&self) -> usize {
        self.handles.len()
    }
}

// ── Watcher spawning ──────────────────────────────────────────────────────────

fn spawn_watcher(
    account: Account,
    db: Arc<Database>,
    app_data_dir: PathBuf,
    app: AppHandle,
    ai_background: TaskQueue,
    stop_flag: Arc<AtomicBool>,
    sync_abort_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    sync_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    online_flag: Arc<AtomicBool>,
) -> tauri::async_runtime::JoinHandle<()> {
    let sync_fn = make_sync_fn(
        db.clone(),
        account.id.clone(),
        app_data_dir,
        app.clone(),
        ai_background,
        sync_abort_flags,
        sync_locks,
    );

    match plan_watcher_kind(&account) {
        WatcherKind::ImapIdle => {
            let creds = crate::services::accounts::get_imap_credentials(&account.id);
            let email = account.email.clone();
            tauri::async_runtime::spawn(imap_idle_watcher(creds, sync_fn, app, email, stop_flag, online_flag))
        }
        WatcherKind::GmailPoll => tauri::async_runtime::spawn(gmail_poll_loop(
            db,
            account.id,
            sync_fn,
            Duration::from_secs(60),
            online_flag,
        )),
    }
}

fn make_sync_fn(
    db: Arc<Database>,
    account_id: String,
    app_data_dir: PathBuf,
    app: AppHandle,
    ai_background: TaskQueue,
    sync_abort_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    sync_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
) -> SyncFn {
    Arc::new(move || {
        let db = db.clone();
        let account_id = account_id.clone();
        let app_data_dir = app_data_dir.clone();
        let app = app.clone();
        let ai_background = ai_background.clone();
        let sync_abort_flags = sync_abort_flags.clone();
        let sync_locks = sync_locks.clone();
        Box::pin(async move {
            match crate::services::emails::sync_account(
                &db,
                &account_id,
                &app_data_dir,
                app.clone(),
                ai_background,
                sync_abort_flags,
                sync_locks,
            )
            .await
            {
                Ok(()) => {
                    // Sync recovered — clear any dedup state so the next
                    // failure (with a possibly different message) is logged
                    // again even if it happens to match a stale entry.
                    clear_sync_error_dedup(&account_id);
                }
                Err(e) => {
                    let message = e.to_string();
                    let is_needs_reauth = matches!(e, AppError::NeedsReauth { .. });

                    // Dedup: when the same error repeats every poll cycle (the
                    // canonical case is NeedsReauth — the keychain entry stays
                    // missing until the user re-authenticates), avoid spamming
                    // the output panel and frontend with identical events.
                    let already_reported = {
                        let guard = LAST_SYNC_ERROR.read().unwrap_or_else(PoisonError::into_inner);
                        guard.get(&account_id).is_some_and(|prev| prev == &message)
                    };

                    if !already_reported {
                        emit_log(&app, &format!("Auto-sync failed for {account_id}: {message}"));

                        // Persist the real error message so the UI shows the
                        // actionable text (e.g. "Authentication required —
                        // please sign in again") instead of the generic
                        // guard fallback ("Sync interrupted…").
                        if let Err(status_error) = db.upsert_sync_status(&account_id, "error", None, Some(&message)) {
                            emit_log(
                                &app,
                                &format!("Failed to persist sync error state for {account_id}: {status_error}"),
                            );
                        }

                        // Emit the same `sync-progress` error event the manual
                        // sync command emits, so the frontend ErrorBanner /
                        // re-authenticate prompt appears for auto-sync auth
                        // failures too. NeedsReauth's Display already contains
                        // "authentication" so the banner's substring match
                        // shows the "Sign in again" button.
                        emit_sync_error(&app, &account_id, &message);

                        // Remember this error for dedup on subsequent ticks.
                        // Only retain across ticks for stable user-actionable
                        // failures (NeedsReauth); transient errors should be
                        // logged each occurrence so the user can spot patterns.
                        if is_needs_reauth {
                            let mut guard = LAST_SYNC_ERROR.write().unwrap_or_else(PoisonError::into_inner);
                            guard.insert(account_id.clone(), message);
                        }
                    }
                }
            }
        })
    })
}

// ── Gmail polling ─────────────────────────────────────────────────────────────

/// Poll Gmail every `interval`. Skips a tick if a sync is already in progress
/// or if the connectivity monitor reports we're offline.
pub(crate) async fn gmail_poll_loop(
    db: Arc<Database>,
    account_id: String,
    sync_fn: SyncFn,
    interval: Duration,
    online_flag: Arc<AtomicBool>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await; // discard the immediate first tick — sync on demand at startup

    loop {
        ticker.tick().await;

        // Offline: the network call would just fail with a transport error
        // that the user can't act on, so skip silently. The probe runs every
        // 15 s and will resume polling automatically on the next reconnect.
        if !online_flag.load(Ordering::Relaxed) {
            continue;
        }

        let already_syncing = db
            .get_sync_status(&account_id)
            .map(|s| s.status == "syncing")
            .unwrap_or(false);

        if already_syncing {
            continue;
        }

        sync_fn().await;
    }
}

// ── IMAP IDLE watcher ─────────────────────────────────────────────────────────

/// Manages an IMAP IDLE connection for one account.
/// The blocking IDLE loop runs on a thread-pool thread and signals via a channel.
/// Reconnects with exponential backoff when the connection drops.
pub(crate) async fn imap_idle_watcher(
    creds: crate::models::error::Result<crate::sync::imap::ImapCredentials>,
    sync_fn: SyncFn,
    app: AppHandle,
    email: String,
    stop_flag: Arc<AtomicBool>,
    online_flag: Arc<AtomicBool>,
) {
    let creds = match creds {
        Ok(c) => c,
        Err(e) => {
            emit_log(&app, &format!("IDLE: cannot load IMAP credentials for {email}: {e}"));
            return;
        }
    };

    let mut backoff = Duration::from_secs(INITIAL_BACKOFF_SECS);

    loop {
        if stop_flag.load(Ordering::Relaxed) {
            return;
        }

        // Don't attempt the IDLE handshake while we know we're offline — it
        // would just fail and trip the reconnect-backoff path on every probe
        // cycle. Poll the flag at a fixed cadence; the connectivity monitor
        // ticks every 15 s so a 5 s wait here is cheap and responsive.
        if !online_flag.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<bool>(4);
        let creds_clone = creds.clone();
        let flag_clone = stop_flag.clone();

        // Blocking IDLE loop — runs on the tokio thread pool
        tokio::task::spawn_blocking(move || {
            crate::sync::imap::ImapClient::run_imap_idle_blocking(creds_clone, tx, flag_clone);
        });

        // Drain notifications until the blocking thread exits (channel closes)
        imap_idle_drain(rx, &sync_fn).await;

        // Connection dropped — reconnect after backoff (unless stopping)
        if stop_flag.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = next_backoff(backoff, MAX_BACKOFF_SECS);
    }
}

/// Drains the IDLE notification channel, calling `sync_fn` on new-mail signals.
/// Returns when the channel closes (blocking IDLE thread exited).
pub(crate) async fn imap_idle_drain(mut rx: tokio::sync::mpsc::Receiver<bool>, sync_fn: &SyncFn) {
    while let Some(new_mail) = rx.recv().await {
        if new_mail {
            sync_fn().await;
        }
        // false = server-side keepalive timeout; re-enter IDLE without syncing
    }
}

// ── Memory consolidation ticker ───────────────────────────────────────────────

/// Per-account periodic consolidation loop. Reads `MemoryConfig` on every tick
/// so interval changes from the settings UI take effect without restarting the
/// scheduler. No-ops (with a short sleep) when memory is disabled or the
/// interval is set to 0.
async fn memory_consolidation_loop(db: Arc<Database>, app: AppHandle, account_id: String, stop_flag: Arc<AtomicBool>) {
    // Short idle delay used when the feature is disabled or interval==0 so we
    // keep checking for config changes without busy-looping.
    const IDLE_POLL_SECS: u64 = 60;

    loop {
        if stop_flag.load(Ordering::Relaxed) {
            return;
        }

        let (enabled, interval_mins) = match crate::services::memory::config::get_config(&db) {
            Ok(c) => (c.enabled, c.consolidation_interval_minutes),
            Err(_) => (false, 0),
        };

        let sleep_secs = if enabled && interval_mins > 0 {
            (interval_mins as u64).saturating_mul(60)
        } else {
            IDLE_POLL_SECS
        };

        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;

        if stop_flag.load(Ordering::Relaxed) {
            return;
        }

        if !enabled || interval_mins == 0 {
            continue;
        }

        if let Err(e) = crate::services::memory::consolidation::run_consolidation(&db, Some(&app), &account_id) {
            let _ = app.emit(
                "app-log",
                AppLogEvent {
                    level: "warn".to_string(),
                    source: "memory".to_string(),
                    message: format!("Periodic consolidation failed for {account_id}: {e}"),
                },
            );
        }
    }
}

// ── VACUUM hygiene ────────────────────────────────────────────────────────────

/// Periodic disk-reclamation loop. Every `VACUUM_INTERVAL` runs:
///   1. `PRAGMA incremental_vacuum(N)` — releases up to `VACUUM_PAGES_PER_TICK`
///      free pages back to the OS. No-op on databases still in
///      `auto_vacuum=NONE` mode (legacy installs).
///   2. `PRAGMA wal_checkpoint(TRUNCATE)` — shrinks the WAL file even when
///      sync batches haven't triggered the per-sync truncate path.
///
/// Both calls are best-effort: contention is swallowed inside the DB helpers
/// so a busy writer never breaks the loop. Errors that escape are logged at
/// `debug` level — this is a background hygiene task, not user-visible work.
const VACUUM_INTERVAL: Duration = Duration::from_secs(30 * 60);
const VACUUM_PAGES_PER_TICK: u32 = 1_000;

async fn vacuum_loop(db: Arc<Database>, app: AppHandle, stop_flag: Arc<AtomicBool>) {
    // Stagger the first run a bit so it doesn't pile on top of startup work.
    tokio::time::sleep(Duration::from_secs(60)).await;

    loop {
        if stop_flag.load(Ordering::Relaxed) {
            return;
        }

        if let Err(e) = db.incremental_vacuum_pages(VACUUM_PAGES_PER_TICK) {
            let _ = app.emit(
                "app-log",
                AppLogEvent {
                    level: "debug".to_string(),
                    source: "system".to_string(),
                    message: format!("incremental_vacuum failed: {e}"),
                },
            );
        }
        if let Err(e) = db.checkpoint_wal_truncate() {
            let _ = app.emit(
                "app-log",
                AppLogEvent {
                    level: "debug".to_string(),
                    source: "system".to_string(),
                    message: format!("wal_checkpoint(TRUNCATE) failed: {e}"),
                },
            );
        }

        tokio::time::sleep(VACUUM_INTERVAL).await;
    }
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

const INITIAL_BACKOFF_SECS: u64 = 5;
const MAX_BACKOFF_SECS: u64 = 300;

/// Double the reconnect backoff, capped at `max_secs` seconds.
pub(crate) fn next_backoff(current: Duration, max_secs: u64) -> Duration {
    Duration::from_secs(current.as_secs().saturating_mul(2).min(max_secs))
}

/// Return only enabled accounts.
pub(crate) fn enabled_accounts(accounts: Vec<Account>) -> Vec<Account> {
    accounts.into_iter().filter(|a| a.enabled).collect()
}

// ── Pure planner ──────────────────────────────────────────────────────────────

/// The kind of background sync watcher an account should run.
///
/// This is a pure, I/O-free decision based solely on the account's provider
/// type. Extract the *decision* here so tests can assert on it without
/// spawning real threads or network connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatcherKind {
    /// IMAP IDLE: maintains a long-lived TLS connection; syncs on server push.
    ImapIdle,
    /// Gmail poll: checks for new messages every `poll_interval`.
    GmailPoll,
}

/// Pure function: decide which watcher strategy to use for an account.
///
/// Call sites should call this first, then route to the appropriate async
/// executor (`imap_idle_watcher` vs `gmail_poll_loop`) based on the result.
/// This lets tests verify routing decisions without spawning any I/O.
pub fn plan_watcher_kind(account: &Account) -> WatcherKind {
    if account.provider == "imap" {
        WatcherKind::ImapIdle
    } else {
        WatcherKind::GmailPoll
    }
}

fn emit_log(_app: &AppHandle, message: &str) {
    crate::services::logger::log("error", "sync", message);
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncProgressEvent {
    account_id: String,
    status: String,
    current: u32,
    total: u32,
    message: String,
}

fn emit_sync_error(app: &AppHandle, account_id: &str, message: &str) {
    let _ = app.emit(
        "sync-progress",
        SyncProgressEvent {
            account_id: account_id.to_string(),
            status: "error".to_string(),
            current: 0,
            total: 0,
            message: message.to_string(),
        },
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn make_account(id: &str, provider: &str, enabled: bool) -> Account {
        Account {
            id: id.to_string(),
            provider: provider.to_string(),
            email: format!("{id}@test.com"),
            name: id.to_string(),
            created_at: 0,
            sort_order: 0,
            enabled,
            sync_from_timestamp: None,
        }
    }

    fn counter_sync_fn(counter: Arc<AtomicU32>) -> SyncFn {
        Arc::new(move || {
            let c = counter.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        })
    }

    // ── next_backoff ──────────────────────────────────────────────────────────

    #[test]
    fn backoff_doubles_up_to_max() {
        assert_eq!(next_backoff(Duration::from_secs(5), 300), Duration::from_secs(10));
        assert_eq!(next_backoff(Duration::from_secs(10), 300), Duration::from_secs(20));
        assert_eq!(next_backoff(Duration::from_secs(128), 300), Duration::from_secs(256));
        assert_eq!(next_backoff(Duration::from_secs(200), 300), Duration::from_secs(300));
        assert_eq!(next_backoff(Duration::from_secs(300), 300), Duration::from_secs(300));
    }

    // ── enabled_accounts ─────────────────────────────────────────────────────

    #[test]
    fn enabled_accounts_excludes_disabled() {
        let accounts = vec![
            make_account("a1", "gmail", true),
            make_account("a2", "imap", true),
            make_account("a3", "gmail", false),
            make_account("a4", "imap", false),
        ];
        let result = enabled_accounts(accounts);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|a| a.enabled));
    }

    #[test]
    fn enabled_accounts_empty_input_returns_empty() {
        assert!(enabled_accounts(vec![]).is_empty());
    }

    #[test]
    fn enabled_accounts_all_disabled_returns_empty() {
        let accounts = vec![make_account("a1", "gmail", false), make_account("a2", "imap", false)];
        assert!(enabled_accounts(accounts).is_empty());
    }

    // ── imap_idle_drain ───────────────────────────────────────────────────────

    /// A `true` signal triggers sync once.
    #[tokio::test]
    async fn imap_idle_drain_triggers_sync_on_new_mail() {
        let count = Arc::new(AtomicU32::new(0));
        let (tx, rx) = tokio::sync::mpsc::channel::<bool>(4);
        tx.send(true).await.unwrap();
        drop(tx);

        imap_idle_drain(rx, &counter_sync_fn(count.clone())).await;

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    /// `false` signals (keepalive timeouts) must NOT trigger sync.
    #[tokio::test]
    async fn imap_idle_drain_ignores_keepalive_signals() {
        let count = Arc::new(AtomicU32::new(0));
        let (tx, rx) = tokio::sync::mpsc::channel::<bool>(4);
        tx.send(false).await.unwrap();
        tx.send(false).await.unwrap();
        drop(tx);

        imap_idle_drain(rx, &counter_sync_fn(count.clone())).await;

        assert_eq!(count.load(Ordering::SeqCst), 0, "keepalive must not trigger sync");
    }

    /// Each `true` signal fires one sync; `false` signals between them are ignored.
    #[tokio::test]
    async fn imap_idle_drain_counts_each_new_mail_signal() {
        let count = Arc::new(AtomicU32::new(0));
        let (tx, rx) = tokio::sync::mpsc::channel::<bool>(4);
        tx.send(true).await.unwrap();
        tx.send(false).await.unwrap();
        tx.send(true).await.unwrap();
        drop(tx);

        imap_idle_drain(rx, &counter_sync_fn(count.clone())).await;

        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    // ── gmail_poll_loop ───────────────────────────────────────────────────────

    /// Sync is called once per interval tick after the initial skip.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn gmail_poll_loop_calls_sync_on_each_interval() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        let count = Arc::new(AtomicU32::new(0));

        let handle = tokio::spawn(gmail_poll_loop(
            db,
            "acc-gmail".to_string(),
            counter_sync_fn(count.clone()),
            Duration::from_secs(60),
            Arc::new(AtomicBool::new(true)),
        ));

        tokio::task::yield_now().await; // let the task start and reach ticker.tick()
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "should sync once after first interval");

        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "should sync again after second interval"
        );

        handle.abort();
    }

    /// Sync must be skipped when status is already "syncing".
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn gmail_poll_loop_skips_sync_when_already_syncing() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc-busy");
        db.upsert_sync_status("acc-busy", "syncing", None, None).unwrap();

        let count = Arc::new(AtomicU32::new(0));

        let handle = tokio::spawn(gmail_poll_loop(
            db,
            "acc-busy".to_string(),
            counter_sync_fn(count.clone()),
            Duration::from_secs(60),
            Arc::new(AtomicBool::new(true)),
        ));

        tokio::task::yield_now().await; // let the task start and reach ticker.tick()
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 0, "must not sync while already syncing");

        handle.abort();
    }

    /// When status is idle the sync runs normally.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn gmail_poll_loop_runs_sync_when_idle() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc-idle");
        db.upsert_sync_status("acc-idle", "idle", None, None).unwrap();

        let count = Arc::new(AtomicU32::new(0));

        let handle = tokio::spawn(gmail_poll_loop(
            db,
            "acc-idle".to_string(),
            counter_sync_fn(count.clone()),
            Duration::from_secs(60),
            Arc::new(AtomicBool::new(true)),
        ));

        tokio::task::yield_now().await; // let the task start and reach ticker.tick()
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 1);

        handle.abort();
    }

    /// Sync must be skipped while the connectivity probe reports offline, and
    /// must resume on the next tick once the flag flips back to online.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn gmail_poll_loop_skips_sync_when_offline() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc-offline");
        db.upsert_sync_status("acc-offline", "idle", None, None).unwrap();

        let count = Arc::new(AtomicU32::new(0));
        let online = Arc::new(AtomicBool::new(false));

        let handle = tokio::spawn(gmail_poll_loop(
            db,
            "acc-offline".to_string(),
            counter_sync_fn(count.clone()),
            Duration::from_secs(60),
            online.clone(),
        ));

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 0, "must not sync while offline");

        // Flip online and advance one more tick — sync should fire.
        online.store(true, Ordering::Relaxed);
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "must sync after reconnect");

        handle.abort();
    }

    // ── plan_watcher_kind ─────────────────────────────────────────────────────

    #[test]
    fn imap_account_gets_imap_idle_watcher() {
        let acct = make_account("a1", "imap", true);
        assert_eq!(plan_watcher_kind(&acct), WatcherKind::ImapIdle);
    }

    #[test]
    fn gmail_account_gets_poll_watcher() {
        let acct = make_account("a1", "gmail", true);
        assert_eq!(plan_watcher_kind(&acct), WatcherKind::GmailPoll);
    }

    #[test]
    fn unknown_provider_falls_back_to_poll() {
        // Any non-"imap" provider (e.g. future "outlook") falls back to the
        // generic poll strategy rather than crashing.
        let acct = make_account("a1", "outlook", true);
        assert_eq!(plan_watcher_kind(&acct), WatcherKind::GmailPoll);
    }
}
