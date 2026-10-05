//! `Notifier` trait seam: desktop (OS) notifications and the window-focus
//! question that decides whether one should be shown.
//!
//! Sibling of [`crate::services::events`] / [`crate::services::logger`]: a
//! process-global, installable backend so services can raise a notification
//! without threading a `tauri::AppHandle`, and tests can record what would have
//! been shown instead of hitting the OS.
//!
//! - Desktop: `TauriNotifier` (installed at startup) → `tauri-plugin-notification`.
//! - CLI / headless / before install: `NoopNotifier` (shows nothing).
//! - Tests: `VecNotifier`, passed directly to the executor under test.
//!
//! Click handling: the desktop notification plugin cannot deliver click events
//! back to the app on macOS/Windows/Linux (see the 2026-07-22 meeting
//! notification entry and the 2026-10-01 new-mail entry in `docs/DECISIONS.md`).
//! A click focuses the app; [`DesktopNotification::thread`] is kept so a future
//! plugin with click callbacks can open the conversation without a planner change.

use std::sync::{Arc, PoisonError, RwLock};

use crate::services::emails::ThreadRef;

/// One OS notification, already rendered (localized, privacy mode applied).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopNotification {
    pub title: String,
    pub body: String,
    /// The conversation it is about, when it is about exactly one.
    pub thread: Option<ThreadRef>,
}

/// Where desktop notifications go.
pub trait Notifier: Send + Sync {
    /// Show one notification. The error is a human-readable reason, logged by
    /// the caller (never containing the notification's content).
    fn show(&self, notification: &DesktopNotification) -> std::result::Result<(), String>;

    /// Whether an EmailOps window currently has the keyboard focus.
    fn app_focused(&self) -> bool;
}

/// Shows nothing; the app is never "focused". Default until a real backend is
/// installed, and what the CLI runs with.
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn show(&self, _notification: &DesktopNotification) -> std::result::Result<(), String> {
        Ok(())
    }

    fn app_focused(&self) -> bool {
        false
    }
}

/// Test notifier: records every notification and reports a configurable focus.
#[derive(Default)]
pub struct VecNotifier {
    shown: RwLock<Vec<DesktopNotification>>,
    focused: std::sync::atomic::AtomicBool,
}

impl VecNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_focused(&self, focused: bool) {
        self.focused.store(focused, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn shown(&self) -> Vec<DesktopNotification> {
        self.shown.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Notifier for VecNotifier {
    fn show(&self, notification: &DesktopNotification) -> std::result::Result<(), String> {
        self.shown
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(notification.clone());
        Ok(())
    }

    fn app_focused(&self) -> bool {
        self.focused.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Production notifier: the OS notification centre via
/// `tauri-plugin-notification`, focus from the app's webview windows.
///
/// OS permission: like the meeting reminders, it relies on the plugin's
/// desktop backend, where the OS asks the user on the first notification
/// (macOS) or needs no permission (Windows/Linux).
#[cfg(feature = "desktop")]
pub struct TauriNotifier {
    app: tauri::AppHandle,
}

#[cfg(feature = "desktop")]
impl TauriNotifier {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[cfg(feature = "desktop")]
impl Notifier for TauriNotifier {
    fn show(&self, notification: &DesktopNotification) -> std::result::Result<(), String> {
        use tauri_plugin_notification::NotificationExt;
        self.app
            .notification()
            .builder()
            .title(&notification.title)
            .body(&notification.body)
            .show()
            .map_err(|e| e.to_string())
    }

    fn app_focused(&self) -> bool {
        use tauri::Manager;
        self.app
            .webview_windows()
            .values()
            .any(|window| window.is_focused().unwrap_or(false))
    }
}

static NOTIFIER: std::sync::LazyLock<RwLock<Arc<dyn Notifier>>> =
    std::sync::LazyLock::new(|| RwLock::new(Arc::new(NoopNotifier)));

/// The active notifier backend.
pub fn current() -> Arc<dyn Notifier> {
    NOTIFIER.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Install the active backend. Called once at app startup with `TauriNotifier`.
pub fn install(backend: Arc<dyn Notifier>) {
    *NOTIFIER.write().unwrap_or_else(PoisonError::into_inner) = backend;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_notifier_records_what_was_shown_and_reports_focus() {
        let notifier = VecNotifier::new();
        assert!(!notifier.app_focused());
        notifier.set_focused(true);
        assert!(notifier.app_focused());
        let n = DesktopNotification {
            title: "Ana".into(),
            body: "Plans".into(),
            thread: None,
        };
        notifier.show(&n).unwrap();
        assert_eq!(notifier.shown(), vec![n]);
    }

    #[test]
    fn noop_notifier_is_never_focused() {
        assert!(!NoopNotifier.app_focused());
    }
}
