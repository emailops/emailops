//! One bounded sync pass, for iOS `BGAppRefreshTask`.
//!
//! iOS suspends the process the moment the app leaves the foreground, so the
//! scheduler's poll loops (`services::sync_scheduler`) simply stop. Background
//! refresh is the only mechanism a serverless client has to be *less* stale
//! than "whatever was there when you last closed it": the system wakes the app
//! when it feels like it — learned from usage, minutes to hours apart, never on
//! demand — and gives it a few seconds to do something useful.
//!
//! This is deliberately **not** new-mail delivery. Timely notification needs a
//! server holding the connection and pushing through APNs, which the
//! architecture does not have (see `docs/DECISIONS.md`, "iOS v1 syncs only
//! while the app is open"). What this buys is that opening the app after lunch
//! shows a current inbox instead of a spinner.
//!
//! Two hard constraints shape everything here:
//!
//! * **The window is small and hard.** iOS allows roughly 30 seconds and then
//!   calls the expiration handler; overrunning is a terminated process and a
//!   worse scheduling reputation. So the pass takes a deadline and checks it
//!   between accounts rather than trusting a sync to be quick.
//! * **The wake may be a cold launch.** If the app was terminated, iOS starts
//!   it in the background and the whole Tauri setup runs first. The FFI entry
//!   point therefore waits (briefly) for the context to be installed instead of
//!   assuming it already is.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
// Only the executor's context holds shared handles; the planner is plain data,
// so the headless (`--no-default-features`) build must not import this.
#[cfg(feature = "desktop")]
use std::sync::Arc;

/// What the planner needs to know about one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshCandidate {
    pub account_id: String,
    pub enabled: bool,
    /// Unix seconds of the last successful sync; `None` if it has never synced.
    pub last_sync_at: Option<i64>,
}

/// Which accounts to sync in this window, most-stale first.
///
/// Staleness order, rather than the account list's own order, is what makes a
/// short window fair: a 20-second budget that only ever reached the first two
/// accounts would leave the third permanently unsynced, and the user would
/// reasonably call that a bug in the third account.
///
/// Never-synced accounts sort first — they have the most to gain and are the
/// most likely to look broken.
pub fn plan_background_refresh(candidates: &[RefreshCandidate], max_accounts: usize) -> Vec<String> {
    let mut eligible: Vec<&RefreshCandidate> = candidates.iter().filter(|c| c.enabled).collect();
    // `i64::MIN` for never-synced puts them ahead of everything with a
    // timestamp. Ties break on account id so the order is deterministic —
    // a test that depended on HashMap iteration order would be flaky.
    eligible.sort_by(|a, b| {
        a.last_sync_at
            .unwrap_or(i64::MIN)
            .cmp(&b.last_sync_at.unwrap_or(i64::MIN))
            .then_with(|| a.account_id.cmp(&b.account_id))
    });
    eligible
        .into_iter()
        .take(max_accounts)
        .map(|c| c.account_id.clone())
        .collect()
}

/// Whether another account may be started, given the time already spent.
///
/// Starting a sync with two seconds left wastes the request and risks being
/// killed mid-write, so the check is "is there room for a *whole* account",
/// not "is there any time left".
pub fn has_room_for_another(elapsed: Duration, budget: Duration, per_account: Duration) -> bool {
    elapsed + per_account <= budget
}

/// How much of the window to spend before refusing to start another account.
/// A first sync on a fresh account can take far longer than this; it is not
/// aborted mid-flight, it simply does not get a successor.
pub const PER_ACCOUNT_RESERVE: Duration = Duration::from_secs(6);

/// At most this many accounts per wake. Beyond it the window is spent on
/// connection setup rather than mail.
pub const MAX_ACCOUNTS_PER_REFRESH: usize = 3;

/// Set when iOS calls the expiration handler. Checked between accounts so a
/// pass that is out of time stops cleanly instead of being terminated.
static EXPIRED: AtomicBool = AtomicBool::new(false);

/// Called from the `BGTask` expiration handler. Cheap and signal-safe: it only
/// flips a flag, because the handler itself has milliseconds to return.
pub fn signal_expired() {
    EXPIRED.store(true, Ordering::Relaxed);
}

/// Read and clear. Only the executor calls it, but the tests pin the
/// clear-on-read behaviour, so it exists in a headless test build too.
#[cfg(any(feature = "desktop", test))]
fn take_expired() -> bool {
    EXPIRED.swap(false, Ordering::Relaxed)
}

pub fn expired() -> bool {
    EXPIRED.load(Ordering::Relaxed)
}

/// Everything a refresh pass needs, captured once at startup.
///
/// Held globally rather than threaded through because the caller is a C
/// function pointer invoked by iOS — there is no `State` to inject into it.
#[cfg(feature = "desktop")]
pub struct RefreshContext {
    pub db: Arc<crate::db::Database>,
    pub app_data_dir: std::path::PathBuf,
    pub app: crate::services::app_handle::AppHandle,
    pub ai_background: crate::services::task_queue::TaskQueue,
    pub sync_abort_flags: Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>>,
    pub sync_locks: Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

#[cfg(feature = "desktop")]
static CONTEXT: std::sync::OnceLock<RefreshContext> = std::sync::OnceLock::new();

/// Publish the context. Called once from the Tauri `setup` hook; a second call
/// is ignored rather than treated as an error, since losing the race is
/// harmless (both values describe the same app).
#[cfg(feature = "desktop")]
pub fn install(context: RefreshContext) {
    let _ = CONTEXT.set(context);
}

/// Run one bounded pass. Returns whether anything was synced successfully —
/// iOS uses the `setTaskCompleted(success:)` answer to decide how generous to
/// be with future windows, so "nothing to do" reports success and only a real
/// failure reports otherwise.
#[cfg(feature = "desktop")]
pub async fn run_refresh(budget: Duration) -> bool {
    let Some(ctx) = CONTEXT.get() else {
        crate::services::logger::log(
            "error",
            "sync",
            "background refresh woke before the app finished starting up".to_string(),
        );
        return false;
    };

    let started = std::time::Instant::now();
    take_expired(); // clear any flag left by a previous window

    let accounts = match ctx.db.list_accounts() {
        Ok(accounts) => accounts,
        Err(e) => {
            crate::services::logger::log("error", "sync", format!("background refresh: {e}"));
            return false;
        }
    };
    let candidates: Vec<RefreshCandidate> = accounts
        .iter()
        .map(|a| RefreshCandidate {
            account_id: a.id.clone(),
            enabled: a.enabled,
            last_sync_at: ctx.db.get_sync_status(&a.id).ok().and_then(|s| s.last_sync_at),
        })
        .collect();

    let planned = plan_background_refresh(&candidates, MAX_ACCOUNTS_PER_REFRESH);
    if planned.is_empty() {
        return true;
    }

    let mut synced = 0usize;
    let mut failed = 0usize;
    for account_id in planned {
        if expired() || !has_room_for_another(started.elapsed(), budget, PER_ACCOUNT_RESERVE) {
            break;
        }
        match crate::services::emails::sync_account(
            &ctx.db,
            &account_id,
            &ctx.app_data_dir,
            Some(ctx.app.clone()),
            ctx.ai_background.clone(),
            ctx.sync_abort_flags.clone(),
            ctx.sync_locks.clone(),
        )
        .await
        {
            Ok(()) => synced += 1,
            Err(e) => {
                failed += 1;
                crate::services::logger::log(
                    "error",
                    "sync",
                    format!("background refresh failed for {account_id}: {e}"),
                );
            }
        }
    }

    crate::services::logger::log(
        "debug",
        "sync",
        format!(
            "background refresh: {synced} synced, {failed} failed, {:.1}s used",
            started.elapsed().as_secs_f64()
        ),
    );
    failed == 0
}

// ── iOS FFI ──────────────────────────────────────────────────────────────────
//
// Called by `EmailOpsBackgroundRefresh.m` (source of truth in `src-tauri/ios/`,
// copied into the generated Xcode project by `scripts/ios_patch_project.sh`).
// Two entry points, both `extern "C"` because the caller is a `BGTask` handler
// block — there is no Rust on that side of the boundary.

/// How long a pass may run. iOS allows roughly 30 seconds from the moment the
/// handler is invoked; the rest is margin for a cold launch that has to boot
/// the whole app first, and for reporting completion afterwards.
#[cfg(target_os = "ios")]
const IOS_REFRESH_BUDGET: Duration = Duration::from_secs(20);

/// How long to wait for the app to finish starting up when iOS wakes a
/// *terminated* app: the BG handler is registered during launch and can fire
/// before Tauri's `setup` hook has published the context.
#[cfg(target_os = "ios")]
const CONTEXT_WAIT: Duration = Duration::from_secs(5);

/// Run one refresh pass, blocking until it finishes. Returns whether it
/// succeeded, for `setTaskCompleted(success:)`.
///
/// # Safety / threading
///
/// Must be called from a background dispatch queue — never the main thread and
/// never from inside the Tokio runtime. `EmailOpsBackgroundRefresh.m` does
/// exactly that; blocking there is what lets the Objective-C side stay a plain
/// synchronous call instead of threading a completion callback through the FFI.
#[cfg(all(target_os = "ios", feature = "desktop"))]
#[no_mangle]
pub extern "C" fn emailops_ios_background_refresh() -> bool {
    tauri::async_runtime::block_on(async {
        let deadline = std::time::Instant::now() + CONTEXT_WAIT;
        while CONTEXT.get().is_none() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        run_refresh(IOS_REFRESH_BUDGET).await
    })
}

/// Called from the `BGTask` expiration handler when iOS is out of patience.
/// Returns immediately — the handler has milliseconds, so all it does is set
/// the flag `run_refresh` checks between accounts.
#[cfg(all(target_os = "ios", feature = "desktop"))]
#[no_mangle]
pub extern "C" fn emailops_ios_expire_background_refresh() {
    signal_expired();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, enabled: bool, last_sync_at: Option<i64>) -> RefreshCandidate {
        RefreshCandidate {
            account_id: id.to_string(),
            enabled,
            last_sync_at,
        }
    }

    #[test]
    fn the_stalest_account_is_synced_first() {
        let candidates = [
            candidate("fresh", true, Some(2_000)),
            candidate("stale", true, Some(1_000)),
        ];
        assert_eq!(plan_background_refresh(&candidates, 3), vec!["stale", "fresh"]);
    }

    #[test]
    fn a_never_synced_account_goes_ahead_of_everything() {
        let candidates = [candidate("known", true, Some(1_000)), candidate("new", true, None)];
        assert_eq!(plan_background_refresh(&candidates, 3), vec!["new", "known"]);
    }

    #[test]
    fn disabled_accounts_are_skipped() {
        let candidates = [candidate("off", false, None), candidate("on", true, Some(5))];
        assert_eq!(plan_background_refresh(&candidates, 3), vec!["on"]);
    }

    #[test]
    fn the_window_is_capped_at_the_stalest_few() {
        let candidates = [
            candidate("a", true, Some(4)),
            candidate("b", true, Some(3)),
            candidate("c", true, Some(2)),
            candidate("d", true, Some(1)),
        ];
        // Cap applies to the *front* of the staleness order, so the accounts
        // that most need a sync are the ones that get it.
        assert_eq!(plan_background_refresh(&candidates, 2), vec!["d", "c"]);
    }

    #[test]
    fn the_order_is_deterministic_when_timestamps_tie() {
        let candidates = [candidate("b", true, Some(1)), candidate("a", true, Some(1))];
        assert_eq!(plan_background_refresh(&candidates, 3), vec!["a", "b"]);
    }

    #[test]
    fn nothing_to_do_plans_nothing() {
        assert!(plan_background_refresh(&[], 3).is_empty());
        assert!(plan_background_refresh(&[candidate("off", false, None)], 3).is_empty());
    }

    #[test]
    fn another_account_starts_only_with_room_to_finish() {
        let budget = Duration::from_secs(20);
        assert!(has_room_for_another(
            Duration::from_secs(0),
            budget,
            PER_ACCOUNT_RESERVE
        ));
        assert!(has_room_for_another(
            Duration::from_secs(14),
            budget,
            PER_ACCOUNT_RESERVE
        ));
        // 15s spent, 6s reserve: starting now would overrun the window and
        // risk being killed mid-write.
        assert!(!has_room_for_another(
            Duration::from_secs(15),
            budget,
            PER_ACCOUNT_RESERVE
        ));
    }

    #[test]
    fn expiry_is_sticky_until_read_and_then_clears() {
        assert!(!expired());
        signal_expired();
        assert!(expired());
        assert!(take_expired());
        // Cleared, so the next window starts without inheriting this one's
        // expiry — otherwise one overrun would poison every later refresh.
        assert!(!expired());
    }
}
