//! `EventSink` trait seam.
//!
//! Sibling of [`crate::services::logger`]. Where `logger` centralises the
//! `app-log` event, `events` centralises every *other* UI event the backend
//! pushes to a front-end: chat token streams (`chat-stream`), processing
//! phases (`chat-phase`), retrieved sources (`chat-sources`), tool effects
//! (`chat-tool-effect`), classification/sync/embedding progress, etc.
//!
//! Before this seam those events were emitted via `app.emit("name", payload)`,
//! which forced every service that wanted to surface progress to thread an
//! `AppHandle` through its signature. Routing them through a global installable
//! sink removes that coupling so the same service code runs:
//!   - in the desktop app (`TauriEventSink` → `AppHandle::emit`),
//!   - in the CLI one-shot path (a stdout sink installed by `cli`),
//!   - in the interactive REPL (`ChannelEventSink` → a live renderer),
//!   - in tests (`VecEventSink`, asserting which events fired).
//!
//! ## Wire compatibility
//!
//! The free [`emit`] helper serialises the payload to `serde_json::Value` and
//! hands it to the sink. `serde_json::Value` serialises to the exact same JSON
//! a typed struct would, so the desktop front-end receives byte-for-byte the
//! same payloads it did when services called `app.emit` directly.

use std::sync::{Arc, PoisonError, RwLock};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

/// A single emitted event: the event name (e.g. `"chat-stream"`) and its
/// already-serialised JSON payload.
pub type Event = (String, Value);

/// Sink for non-log UI events.
pub trait EventSink: Send + Sync {
    /// Deliver one event. Implementations must not block — emission happens on
    /// hot paths (per-token streaming, sync batches) and a slow sink would
    /// back-pressure the whole pipeline.
    fn emit(&self, name: &str, payload: Value);
}

/// Production sink: forwards events to the front-end via `AppHandle::emit`.
pub struct TauriEventSink {
    app: tauri::AppHandle,
}

impl TauriEventSink {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl EventSink for TauriEventSink {
    fn emit(&self, name: &str, payload: Value) {
        use tauri::Emitter;
        // Ignore emit failures: a missing webview must not crash the
        // background pipeline (mirrors `logger::TauriLogger`).
        let _ = self.app.emit(name, payload);
    }
}

/// No-op sink. Active before any real sink is installed (process bootstrap)
/// and the default until a test or the CLI opts in.
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _name: &str, _payload: Value) {}
}

/// Sink that forwards events into an unbounded channel. The REPL (and, later, a
/// TUI) installs this and consumes the receiver on a render task, turning the
/// chat service's token stream into live terminal output without the service
/// knowing anything about the front-end.
pub struct ChannelEventSink {
    tx: UnboundedSender<Event>,
}

impl ChannelEventSink {
    pub fn new(tx: UnboundedSender<Event>) -> Self {
        Self { tx }
    }
}

impl EventSink for ChannelEventSink {
    fn emit(&self, name: &str, payload: Value) {
        // Receiver dropped → renderer has gone away; nothing to do.
        let _ = self.tx.send((name.to_string(), payload));
    }
}

/// Test sink that records every event. Tests assert on `events()` (or the
/// `payloads_for` filter) after the code under test runs.
#[derive(Default)]
pub struct VecEventSink {
    events: RwLock<Vec<Event>>,
}

impl VecEventSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every (name, payload) recorded so far.
    pub fn events(&self) -> Vec<Event> {
        self.events.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Count events whose name matches `name`.
    pub fn count(&self, name: &str) -> usize {
        self.events
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(n, _)| n == name)
            .count()
    }

    /// Payloads of every event whose name matches `name`, in emission order.
    pub fn payloads_for(&self, name: &str) -> Vec<Value> {
        self.events
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, p)| p.clone())
            .collect()
    }
}

impl EventSink for VecEventSink {
    fn emit(&self, name: &str, payload: Value) {
        self.events
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push((name.to_string(), payload));
    }
}

// ── Global registry ──────────────────────────────────────────────────────────

static SINK: std::sync::LazyLock<RwLock<Arc<dyn EventSink>>> =
    std::sync::LazyLock::new(|| RwLock::new(Arc::new(NoopEventSink)));

/// Get a handle to the active sink backend.
pub fn current() -> Arc<dyn EventSink> {
    SINK.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Install a new active sink. Called at app startup (with `TauriEventSink`),
/// by the CLI/REPL, and by tests.
pub fn install(backend: Arc<dyn EventSink>) {
    *SINK.write().unwrap_or_else(PoisonError::into_inner) = backend;
}

/// Emit an event through the active backend.
///
/// Generic over `Serialize` so call sites read `events::emit("chat-stream",
/// &event)` exactly like the old `app.emit("chat-stream", &event)`. The payload
/// is serialised to `serde_json::Value` here; a serialisation failure is logged
/// and the event dropped (never panics on a hot path).
pub fn emit<S: Serialize>(name: &str, payload: S) {
    match serde_json::to_value(payload) {
        Ok(value) => current().emit(name, value),
        Err(e) => crate::services::logger::log(
            "error",
            "system",
            format!("failed to serialise '{name}' event payload: {e}"),
        ),
    }
}

/// Swap in a `VecEventSink` and return it so the test can inspect events.
pub fn install_for_testing() -> Arc<VecEventSink> {
    let sink = Arc::new(VecEventSink::new());
    install(sink.clone() as Arc<dyn EventSink>);
    sink
}

/// Shared serialization lock for every test that touches the process-global
/// `SINK` or `LOGGER`. Because `events::emit` reads the global sink and
/// `logger::log` reads the global logger, tests in *different* modules
/// (`events`, `logger`, `emails::events`, …) race unless they all serialize on
/// the same mutex — a per-module lock only protects intra-module tests. Any new
/// seam-touching test must hold this guard.
#[cfg(test)]
pub(crate) fn seam_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
    M.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    use seam_test_lock as lock;

    #[test]
    fn vec_sink_records_events_in_order() {
        let _g = lock();
        let sink = install_for_testing();
        emit("chat-phase", serde_json::json!({ "phase": "routing" }));
        emit("chat-stream", serde_json::json!({ "token": "hi" }));
        emit("chat-stream", serde_json::json!({ "token": " there" }));
        let events = sink.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].0, "chat-phase");
        assert_eq!(events[1].0, "chat-stream");
        assert_eq!(sink.count("chat-stream"), 2);
        let tokens: Vec<_> = sink
            .payloads_for("chat-stream")
            .into_iter()
            .map(|p| p["token"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(tokens, vec!["hi", " there"]);
        install(Arc::new(NoopEventSink));
    }

    #[test]
    fn emit_serialises_typed_payloads_like_app_emit() {
        let _g = lock();
        let sink = install_for_testing();
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Sample {
            message_id: String,
            done: bool,
        }
        emit(
            "chat-stream",
            &Sample {
                message_id: "m1".into(),
                done: true,
            },
        );
        let payloads = sink.payloads_for("chat-stream");
        assert_eq!(payloads.len(), 1);
        // camelCase field renaming is preserved through Value, matching the
        // JSON the frontend received from app.emit.
        assert_eq!(payloads[0]["messageId"], "m1");
        assert_eq!(payloads[0]["done"], true);
        install(Arc::new(NoopEventSink));
    }

    #[test]
    fn channel_sink_forwards_to_receiver() {
        let _g = lock();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        install(Arc::new(ChannelEventSink::new(tx)));
        emit("chat-stream", serde_json::json!({ "token": "x" }));
        let (name, payload) = rx.try_recv().expect("event forwarded to channel");
        assert_eq!(name, "chat-stream");
        assert_eq!(payload["token"], "x");
        install(Arc::new(NoopEventSink));
    }

    #[test]
    fn noop_sink_is_default_and_swappable() {
        let _g = lock();
        install(Arc::new(NoopEventSink));
        // Emitting through the noop sink must not panic and records nothing.
        emit("anything", serde_json::json!({}));
        let sink = install_for_testing();
        assert_eq!(sink.events().len(), 0);
        install(Arc::new(NoopEventSink));
    }
}
