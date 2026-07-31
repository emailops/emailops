//! Transport-agnostic stand-in for `tauri::AppHandle`.
//!
//! ## Why this exists
//!
//! Roughly 20 service modules carry an `app: Option<AppHandle>` (or `&AppHandle`)
//! parameter that they no longer *use* — `services/logger.rs` and `services/events.rs`
//! became the real seams, and the helpers were refactored to delegate to them. The
//! parameter stayed in the signatures only to avoid touching ~78 call sites.
//!
//! That leftover parameter is the single thing preventing `services/`, `sync/` and
//! `db/` from compiling without Tauri — which is what a headless server (or any
//! non-desktop front end) needs.
//!
//! So: alias it. With the `desktop` feature on, `AppHandle` *is* `tauri::AppHandle`
//! and nothing changes. With it off, `AppHandle` is a zero-sized stub that satisfies
//! the same signatures. Call sites, including every `Option<AppHandle>` and every
//! `app.clone()`, keep compiling verbatim.
//!
//! ## The `emit` forwarder
//!
//! ~30 call sites still do `let _ = app.emit("some-event", payload)` directly rather
//! than going through the seam. Rewriting them all is the *correct* end state, but it
//! is a wide, risky diff through sync, embeddings and attachments — and it is not what
//! makes a headless build possible.
//!
//! So the stub grows exactly one method: `emit`, forwarding to `services::events::emit`.
//! That keeps every existing call site compiling untouched **and** routes it correctly,
//! because `events::emit` resolves the sink from the ambient per-user context. Desktop
//! builds are byte-for-byte unchanged (they get the real `tauri::AppHandle`).
//!
//! This is a deliberate trade: it perpetuates the `AppHandle`-threading pattern in
//! exchange for a small, reviewable diff. Migrating those call sites to call
//! `events::emit`/`logger::log` directly — and then dropping the parameter entirely —
//! is tracked as follow-up work, not a blocker.
//!
//! ## Removal
//!
//! Once every service signature has dropped its unused `app` parameter, this module
//! disappears. Until then it is the cheapest possible bridge: one type alias instead
//! of a 78-call-site refactor.

#[cfg(feature = "desktop")]
pub type AppHandle = tauri::AppHandle;

/// Zero-sized stand-in used when the crate is built without Tauri.
#[cfg(not(feature = "desktop"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppHandle;

#[cfg(not(feature = "desktop"))]
impl AppHandle {
    /// Drop-in for `tauri::Emitter::emit`, forwarding to the ambient event sink.
    ///
    /// Returns `Result` so that the existing `let _ = app.emit(...)` call sites keep
    /// compiling. The error type is `std::convert::Infallible` because delivery is
    /// fire-and-forget: `events::emit` already logs a serialisation failure rather
    /// than propagating it (see `services/events.rs`).
    pub fn emit<S: serde::Serialize>(
        &self,
        event: &str,
        payload: S,
    ) -> std::result::Result<(), std::convert::Infallible> {
        crate::services::events::emit(event, payload);
        Ok(())
    }
}

#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::*;

    #[test]
    fn stub_is_zero_sized() {
        assert_eq!(std::mem::size_of::<AppHandle>(), 0);
    }

    #[test]
    fn stub_survives_the_option_and_clone_patterns_call_sites_use() {
        let app = AppHandle;
        let maybe: Option<AppHandle> = Some(app);
        let cloned = maybe.clone();
        assert_eq!(maybe, cloned);
        assert!(cloned.is_some());
    }
}
