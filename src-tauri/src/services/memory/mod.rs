//! Memory subsystem — service layer.
//!
//! Owns durable memory facts, consolidation, embeddings, and memory-context
//! interaction signals. Pending tasks and thread follow-up state live in
//! `services::tasks`.
//!
//! Every helper is tolerant to missing rows: user-action instrumentation must
//! never fail the originating operation (marking an email read must not break
//! because the memory subsystem had a hiccup). Callers log errors and continue.

use std::sync::Arc;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{MemoryFact, ThreadState};
use serde_json::json;

pub mod config;
pub mod consolidation;
pub mod embeddings;
pub mod extractor;
pub mod header;

// ── Memory interaction signals ───────────────────────────────────────────────

/// Log that the user performed a search. Search history feeds the dream
/// job's query-diversity signal (OpenClaw-style).
pub fn on_search(db: &Arc<Database>, account_id: &str, query: &str) {
    let payload = json!({ "query": query }).to_string();
    if let Err(e) = db.log_interaction_event(account_id, "search", None, None, Some(&payload)) {
        crate::services::logger::log("error", "memory", format!("log_interaction_event(search) failed: {e}"));
    }
}

/// Log that a chat turn completed. The payload stores the user-side query so
/// the dream job can later correlate what the agent was asked about with
/// which facts it recalled.
pub fn on_chat_turn(db: &Arc<Database>, account_id: &str, query: &str) {
    let payload = json!({ "query": query }).to_string();
    if let Err(e) = db.log_interaction_event(account_id, "chat_turn", None, None, Some(&payload)) {
        crate::services::logger::log(
            "error",
            "memory",
            format!("log_interaction_event(chat_turn) failed: {e}"),
        );
    }
}

// ── Tool-backing helpers ────────────────────────────────────────────────────
//
// Thin wrappers behind the chat `memory_search` / `recall_entity` /
// `remember` / `list_open_threads` tools. Keep the SQL in `db::memory`;
// these exist so tools route through the service layer.

/// FTS5 search over memory facts; returns each hit paired with its score.
/// Also best-effort bumps the recency score of every returned fact so
/// frequently used facts rise (matches the prior in-tool behaviour).
pub fn search_facts(db: &Arc<Database>, account_id: &str, query: &str, limit: i32) -> Result<Vec<(MemoryFact, f64)>> {
    let hits = db.search_memory_facts_fts(account_id, query, limit)?;
    let now = chrono::Utc::now().timestamp();
    for (f, _) in &hits {
        // Best-effort — a bump failure must not poison the search result.
        let _ = db.bump_memory_fact_score(&f.id, 0.0, now);
    }
    Ok(hits)
}

/// Combine `contact`, `domain`, and `project` lookups for one subject key
/// (email / domain / slug). Order matches the existing tool ladder so the
/// caller can render rows in priority order.
pub fn recall_entity(db: &Arc<Database>, account_id: &str, key: &str) -> Result<Vec<MemoryFact>> {
    let mut out = Vec::new();
    for kind in ["contact", "domain", "project"] {
        match db.get_memory_facts_by_subject(account_id, kind, key) {
            Ok(rows) => out.extend(rows),
            // Per the module's "user actions must not fail" contract: log
            // and continue instead of bubbling a partial-result error.
            Err(e) => {
                crate::services::logger::log("error", "memory", format!("recall_entity({kind}, {key}) failed: {e}"))
            }
        }
    }
    Ok(out)
}

/// Persist a new memory fact as a `candidate`. The background consolidation
/// job is responsible for promotion. Returns the inserted fact so callers
/// can surface its id.
pub fn remember_fact(
    db: &Arc<Database>,
    account_id: &str,
    fact: &str,
    subject_kind: &str,
    subject_key: &str,
) -> Result<MemoryFact> {
    let now = chrono::Utc::now().timestamp();
    let row = MemoryFact {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.to_string(),
        subject_kind: subject_kind.to_ascii_lowercase(),
        subject_key: subject_key.to_ascii_lowercase(),
        fact: fact.to_string(),
        source: "user".to_string(),
        source_email_id: None,
        confidence: 0.9,
        score: 0.5,
        status: "candidate".to_string(),
        last_used_at: None,
        domain: None,
        vigency: None,
        company: None,
        created_at: now,
        updated_at: now,
    };
    db.insert_memory_fact(&row)?;
    Ok(row)
}

/// List threads that still need a reply (or whose state is otherwise open).
/// Thread state is stored under the memory subsystem, so this helper lives
/// here even though the chat tool framing is task-shaped.
pub fn list_open_threads(
    db: &Arc<Database>,
    account_id: &str,
    awaiting: Option<&str>,
    limit: i32,
) -> Result<Vec<ThreadState>> {
    db.list_open_thread_states(account_id, awaiting, limit)
}
