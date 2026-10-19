//! Recover from a crashed WebKit web process instead of leaving a dead window.
//!
//! On Linux the UI runs in WebKitGTK's separate web process. When that
//! process dies, the window goes blank while the main process keeps running
//! and keeps the single-instance lock, so the user cannot even relaunch the
//! app. We reload the page instead. A crash that repeats right away (a font
//! that aborts the renderer on every paint, say) would turn that into an
//! endless reload loop, so after a few crashes in a short window the app tells
//! the user and exits, which releases the lock.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Crashes tolerated inside [`CRASH_WINDOW`] before giving up.
pub const MAX_CRASHES: usize = 3;

/// The window [`MAX_CRASHES`] is counted over.
pub const CRASH_WINDOW: Duration = Duration::from_secs(60);

/// What to do about one web-process crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashAction {
    /// Reload the page in a fresh web process.
    Reload,
    /// The page keeps crashing: tell the user and exit.
    GiveUp,
}

/// Counts recent crashes to decide between reloading and giving up.
#[derive(Debug)]
pub struct CrashBudget {
    max: usize,
    window: Duration,
    recent: VecDeque<Instant>,
}

impl Default for CrashBudget {
    fn default() -> Self {
        Self::new(MAX_CRASHES, CRASH_WINDOW)
    }
}

impl CrashBudget {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            recent: VecDeque::new(),
        }
    }

    /// Record a crash at `now` and say what to do about it.
    pub fn record(&mut self, now: Instant) -> CrashAction {
        while self
            .recent
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= self.window)
        {
            self.recent.pop_front();
        }
        self.recent.push_back(now);
        if self.recent.len() >= self.max {
            CrashAction::GiveUp
        } else {
            CrashAction::Reload
        }
    }
}

/// Message shown when the app gives up.
pub const GIVE_UP_MESSAGE: &str = "EmailOps closed because its window kept crashing. \
This usually comes from a font or graphics driver the built-in web view cannot handle. \
Open EmailOps again; if it keeps happening, please report it with the details in the log.";

/// Reload the webview after a web-process crash, or exit after repeated ones.
#[cfg(all(target_os = "linux", feature = "desktop"))]
pub fn watch(window: &tauri::WebviewWindow) {
    use std::sync::{Arc, Mutex};
    use webkit2gtk::{WebProcessTerminationReason, WebViewExt};

    let budget = Arc::new(Mutex::new(CrashBudget::default()));
    let result = window.with_webview(move |platform| {
        let budget = budget.clone();
        platform.inner().connect_web_process_terminated(move |webview, reason| {
            // An API-requested termination is ours, not a crash.
            if reason == WebProcessTerminationReason::TerminatedByApi {
                return;
            }
            let action = budget
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .record(Instant::now());
            let detail = format!("web view process terminated ({reason:?})");
            match action {
                CrashAction::Reload => {
                    eprintln!("[webview] {detail}; reloading");
                    crate::services::logger::log("error", "system", format!("{detail}; the window was reloaded"));
                    webview.reload();
                }
                CrashAction::GiveUp => {
                    eprintln!("[webview] {detail}; {MAX_CRASHES} crashes within {CRASH_WINDOW:?}, exiting");
                    crate::util::startup_dialog::show(None, &format!("{GIVE_UP_MESSAGE}\n\n{detail}"));
                    crate::services::ai::shutdown_and_exit(1);
                }
            }
        });
    });
    if let Err(e) = result {
        eprintln!("[webview] Warning: could not watch for web process crashes: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_crash_reloads() {
        let mut budget = CrashBudget::new(3, Duration::from_secs(60));
        assert_eq!(budget.record(Instant::now()), CrashAction::Reload);
    }

    #[test]
    fn third_crash_within_the_window_gives_up() {
        let mut budget = CrashBudget::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        assert_eq!(budget.record(t0), CrashAction::Reload);
        assert_eq!(budget.record(t0 + Duration::from_secs(10)), CrashAction::Reload);
        assert_eq!(budget.record(t0 + Duration::from_secs(20)), CrashAction::GiveUp);
    }

    #[test]
    fn crashes_older_than_the_window_are_forgotten() {
        let mut budget = CrashBudget::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        budget.record(t0);
        budget.record(t0 + Duration::from_secs(10));
        assert_eq!(budget.record(t0 + Duration::from_secs(75)), CrashAction::Reload);
    }
}
