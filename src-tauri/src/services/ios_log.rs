//! iOS system-log sink for `app-log` events.
//!
//! ## Why this exists
//!
//! On desktop, `TauriLogger` emits every `AppLogEvent` to the frontend, where
//! the output panel shows it. The output panel is desktop-only — the phone
//! layout has no room for it — so on iOS the same events go to the webview and
//! are never rendered. Anything a background loop reports on a device is
//! therefore invisible, which is exactly the case where a developer has the
//! fewest other ways to look (no terminal, no DB browser, no dev tools).
//!
//! This wraps whatever logger is already installed and additionally forwards
//! each event to `NSLog`, so `xcrun devicectl device process launch --console`
//! and `log stream` show it live from a connected Mac.
//!
//! ## Why a registered function pointer
//!
//! Same reason as `ai::foundation_models` and the background-refresh bridge:
//! the crate is built as a `cdylib` too, and a dylib must resolve every symbol
//! at link time. A Rust `extern "C"` declaration of an ObjC function that lives
//! in the *app* target breaks the cargo link before Xcode is involved. So the
//! app pushes the sink in at `+load` instead. Until it does, this is inert —
//! which is also why the module compiles on every platform: nothing registers a
//! sink off iOS, and building it everywhere keeps its tests running on the host
//! rather than only on the one target where they cannot be observed.

use std::ffi::{c_char, CString};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use crate::models::AppLogEvent;
use crate::services::logger::Logger;

type LogFn = unsafe extern "C" fn(*const c_char);

/// Sink registered by the app at launch. Null until then.
static SINK: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

/// Called from `EmailOpsLogBridge.m` (`+load`).
///
/// # Safety
/// `sink` must be a valid C function pointer that reads the passed pointer as
/// a NUL-terminated UTF-8 string and does not retain it past the call.
#[no_mangle]
pub unsafe extern "C" fn emailops_ios_register_log_sink(sink: LogFn) {
    SINK.store(sink as *mut (), Ordering::Release);
}

/// Self-test called by the bridge immediately after it registers.
///
/// Proves the round trip — ObjC stored a pointer, Rust read *that* pointer and
/// called back — rather than each side merely believing it did its half. The
/// crate is built as both a cdylib and the staticlib Xcode links, so "two live
/// copies of this module, each with its own `SINK`" is a real failure mode that
/// looks exactly like a working registration from either side alone.
///
/// # Safety
/// Runs at `+load`, before `main`. Touches only an atomic and the registered
/// pointer, so it needs no initialised Rust runtime.
#[no_mangle]
pub extern "C" fn emailops_ios_log_self_test() {
    write_to_system_log("[info] system: log bridge round trip ok");
}

/// Forward one line to the system log. No-op when nothing is registered.
fn write_to_system_log(line: &str) {
    let sink = SINK.load(Ordering::Acquire);
    if sink.is_null() {
        return;
    }
    // Interior NULs cannot reach NSLog as a C string. A log line is never worth
    // failing over, so a message that contains one is simply dropped.
    let Ok(c_line) = CString::new(line) else {
        return;
    };
    // SAFETY: non-null by the check above, and only ever stored by
    // `emailops_ios_register_log_sink`, whose contract requires a valid fn ptr.
    let sink: LogFn = unsafe { std::mem::transmute::<*mut (), LogFn>(sink) };
    unsafe { sink(c_line.as_ptr()) };
}

/// Decide what one event looks like in the system log.
///
/// Pure so it can be tested without an iOS device: the whole point of the
/// module is a path that cannot be exercised in CI.
pub fn format_event(event: &AppLogEvent) -> String {
    format!("[{}] {}: {}", event.level, event.source, event.message)
}

/// Wraps another logger and additionally writes to the iOS system log.
pub struct SystemLogTee {
    inner: Arc<dyn Logger>,
}

impl SystemLogTee {
    /// Wrap `inner`. Events reach `inner` unchanged; the system log is additive.
    pub fn wrap(inner: Arc<dyn Logger>) -> Self {
        Self { inner }
    }
}

impl Logger for SystemLogTee {
    fn log(&self, event: AppLogEvent) {
        write_to_system_log(&format_event(&event));
        self.inner.log(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::logger::VecLogger;

    fn event(level: &str, source: &str, message: &str) -> AppLogEvent {
        AppLogEvent {
            level: level.to_string(),
            source: source.to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn a_formatted_event_carries_level_source_and_message() {
        // The device console interleaves every process on the phone, so a line
        // has to be greppable on its own.
        let line = format_event(&event("debug", "sync", "auto-sync skipped"));
        assert!(line.contains("debug"));
        assert!(line.contains("sync"));
        assert!(line.contains("auto-sync skipped"));
    }

    #[test]
    fn teeing_does_not_swallow_the_event() {
        // The regression that would make this module actively harmful: losing
        // the frontend copy while gaining the console one.
        let inner = Arc::new(VecLogger::new());
        let tee = SystemLogTee::wrap(inner.clone() as Arc<dyn Logger>);
        tee.log(event("error", "ai", "boom"));
        assert_eq!(inner.events().len(), 1);
        assert_eq!(inner.events()[0].message, "boom");
    }

    #[test]
    fn writing_with_no_sink_registered_is_inert() {
        // Desktop, CLI and tests never register one; this must not panic.
        write_to_system_log("nothing is listening");
    }

    #[test]
    fn a_message_with_an_interior_nul_is_dropped_not_fatal() {
        let tee = SystemLogTee::wrap(Arc::new(VecLogger::new()) as Arc<dyn Logger>);
        tee.log(event("info", "sync", "before\0after"));
    }
}
