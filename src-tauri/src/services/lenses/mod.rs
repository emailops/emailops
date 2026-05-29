//! Lenses feature: AI-powered views of the mailbox.
//!
//! - `scope`     — evaluate a `LensScope` to the matching `email_id` list.
//! - `extractor` — run the LLM against one email + a Lens schema.
//! - `runner`    — orchestrate backfills, incremental updates, and re-extractions.
//! - `templates` — built-in template manifest (Phase 2; minimal stub in Phase 1).
//!
//! See `docs/lenses-prd.md` for the full design.

pub mod extractor;
pub mod runner;
pub mod scope;
pub mod templates;

use std::sync::Arc;

use tauri::AppHandle;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::lens::{LensRowsPage, LensSummary, SortSpec};

// ── Tool-backing helpers ────────────────────────────────────────────────────
//
// Thin wrappers behind the chat `list_lenses` / `get_lens_data` tools and any
// future read-only command. Keeps the SQL in `db::lenses`.

/// All lenses, ordered as the sidebar would render them (sort_order then
/// created_at). Used by the chat `list_lenses` tool so the LLM can resolve
/// a user-facing name to an id before fetching rows.
pub fn list_lenses(db: &Arc<Database>) -> Result<Vec<LensSummary>> {
    db.list_lenses()
}

/// Page of persisted lens rows for one lens. Reads only — does not trigger
/// extraction even when zero rows exist; that decision is the caller's.
pub fn get_lens_rows(
    db: &Arc<Database>,
    lens_id: &str,
    sort: Option<&SortSpec>,
    limit: i64,
    offset: i64,
) -> Result<LensRowsPage> {
    db.get_lens_rows(lens_id, sort, limit, offset)
}

/// Standard log source for all Lens-related events.
pub const LOG_SOURCE: &str = "lens";

pub(crate) fn emit_log(_app: Option<&AppHandle>, level: &str, message: impl Into<String>) {
    crate::services::logger::log(level, LOG_SOURCE, message);
}
