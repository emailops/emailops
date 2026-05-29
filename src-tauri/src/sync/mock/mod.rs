//! Provider-API record/replay harness for sync testing.
//!
//! Production sync goes through `GmailClient` and `OutlookClient` (under
//! `sync::gmail` / `sync::outlook`). Both can be configured with a
//! `with_base_url(...)` override (Phase 1 of this work). This module provides
//! the surrounding machinery that lets tests:
//!
//! 1. Record real HTTP interactions against the live provider APIs into
//!    JSON "cassette" files using the `record_provider_cassette` example
//!    (under `src-tauri/examples/`).
//! 2. Replay those cassettes via a [`server::MockProviderServer`] that wraps
//!    `wiremock`, returning the same `(status, headers, body)` the live API
//!    returned at record time.
//! 3. Force error scenarios (401 → refresh, 429 with `Retry-After`, 503,
//!    truncated JSON, missing `@odata.nextLink`) by hand-editing the
//!    cassette JSON and re-running.
//!
//! The whole module is gated behind `cfg(any(test, debug_assertions))` so
//! release builds carry none of this code or its transitive deps.

#![cfg(any(test, debug_assertions))]

pub mod cassette;
pub mod sanitize;

pub use cassette::{Cassette, Interaction, RecordedRequest, RecordedResponse};
pub use sanitize::{sanitize_cassette, sanitize_value, SANITIZED_BODY_LIMIT};

// The wiremock-based replay server lives under `src-tauri/tests/common/`
// rather than here — `wiremock` is a dev-dependency and isn't visible to
// integration-test crates when imported through `emailops_lib`. Putting the
// helper alongside the tests keeps the dependency graph honest.
