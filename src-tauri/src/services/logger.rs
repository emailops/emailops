//! Logger trait seam.
//!
//! Centralises emission of `app-log` events so:
//!   1. Tests can assert on log output via `VecLogger` instead of needing a
//!      live Tauri runtime.
//!   2. Services don't need to thread `AppHandle` everywhere just to log.
//!   3. There is a single place to add/replace transport (e.g. piping to a
//!      file in the future).
//!
//! ## Production
//!
//! `lib.rs::run()` constructs a `TauriLogger` with the real `AppHandle` and
//! calls [`install`] during the Tauri `setup()` hook. From that point onward
//! any code calling [`log()`] (or the convenience `info!`/`error!`-style
//! helpers) emits an `app-log` event to the frontend.
//!
//! ## Tests
//!
//! Tests call [`install_for_testing()`], which swaps in a `VecLogger` and
//! returns an `Arc<VecLogger>` so the test can inspect recorded events.
//!
//! ## Migration note
//!
//! 20 modules currently define a local `emit_log(app: &AppHandle, ...)` helper.
//! These have been refactored to delegate to this seam — they ignore the
//! passed AppHandle. The AppHandle parameter remains in helper signatures
//! for now to avoid touching ~78 call sites; future cleanup will remove it.

use std::sync::{Arc, PoisonError, RwLock};

use crate::models::AppLogEvent;

/// Sink for `app-log` events.
pub trait Logger: Send + Sync {
    /// Emit a single log event. Implementations must not block — the call
    /// happens on hot paths (sync loops, embedding batches, etc.) and a slow
    /// logger would back-pressure the whole pipeline.
    fn log(&self, event: AppLogEvent);
}

/// Production logger: forwards events to the frontend via `AppHandle::emit`.
pub struct TauriLogger {
    app: tauri::AppHandle,
}

impl TauriLogger {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl Logger for TauriLogger {
    fn log(&self, event: AppLogEvent) {
        use tauri::Emitter;
        // Ignore emit failures: a missing webview must not crash the
        // background pipeline. The frontend will catch up via DB-backed
        // status the next time it polls.
        let _ = self.app.emit("app-log", event);
    }
}

/// No-op logger. Active before any real logger is installed (e.g. during
/// process bootstrap) and the default for `Database::new_for_testing()` until
/// a test explicitly opts in via `install_for_testing`.
pub struct NoopLogger;

impl Logger for NoopLogger {
    fn log(&self, _event: AppLogEvent) {}
}

/// Test logger that records every event it receives. Tests assert on
/// `events()` after the code under test runs.
#[derive(Default)]
pub struct VecLogger {
    events: RwLock<Vec<AppLogEvent>>,
}

impl VecLogger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every event recorded so far. Returned by value so tests
    /// can inspect without holding the lock.
    pub fn events(&self) -> Vec<AppLogEvent> {
        self.events.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Count events by `level` (e.g. "info", "error"). Convenience for
    /// tests that only care about totals.
    pub fn count_by_level(&self, level: &str) -> usize {
        self.events
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|e| e.level == level)
            .count()
    }
}

impl Logger for VecLogger {
    fn log(&self, event: AppLogEvent) {
        self.events.write().unwrap_or_else(PoisonError::into_inner).push(event);
    }
}

// ── Global registry ──────────────────────────────────────────────────────────

static LOGGER: std::sync::LazyLock<RwLock<Arc<dyn Logger>>> =
    std::sync::LazyLock::new(|| RwLock::new(Arc::new(NoopLogger)));

/// Get a handle to the active logger backend.
pub fn current() -> Arc<dyn Logger> {
    LOGGER.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Install a new active logger. Called at app startup (with `TauriLogger`)
/// and by tests (with `VecLogger`).
pub fn install(backend: Arc<dyn Logger>) {
    *LOGGER.write().unwrap_or_else(PoisonError::into_inner) = backend;
}

/// Emit a log event through the active backend.
pub fn log(level: &str, source: &str, message: impl Into<String>) {
    current().log(AppLogEvent {
        level: level.to_string(),
        source: source.to_string(),
        message: message.into(),
    });
}

pub fn install_for_testing() -> Arc<VecLogger> {
    let logger = Arc::new(VecLogger::new());
    install(logger.clone() as Arc<dyn Logger>);
    logger
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serialize tests in this module: they all share the global LOGGER and
    // would race otherwise.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[test]
    fn vec_logger_records_events() {
        let _g = lock();
        let logger = install_for_testing();
        log("info", "test", "first");
        log("error", "test", "second");
        let events = logger.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].level, "info");
        assert_eq!(events[1].level, "error");
        assert_eq!(events[0].message, "first");
        // Reset for other tests
        install(Arc::new(NoopLogger));
    }

    #[test]
    fn count_by_level_filters() {
        let _g = lock();
        let logger = install_for_testing();
        log("info", "s", "a");
        log("info", "s", "b");
        log("error", "s", "c");
        assert_eq!(logger.count_by_level("info"), 2);
        assert_eq!(logger.count_by_level("error"), 1);
        assert_eq!(logger.count_by_level("debug"), 0);
        install(Arc::new(NoopLogger));
    }
}
