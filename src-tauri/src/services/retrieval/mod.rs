//! Shared retrieval primitives used by `services::search`, `services::chat`,
//! and `services::agent_search`.
//!
//! The three consumers each have their own orchestration (query understanding,
//! consumer-specific re-ranking, result-type shaping) but the *retrieval* core
//! is the same: fetch FTS + vector candidate lists from the DB, fuse them with
//! Reciprocal Rank Fusion, optionally collapse to one email per thread, and
//! hand the ranked `(email_id, score)` list back to the consumer for
//! hydration and post-processing.
//!
//! Intentionally narrow scope:
//!   - Knows nothing about LLM query rewriting, embeddings models, or prompts.
//!   - Returns raw `(id, score)` pairs — never UI-facing types like
//!     `EmailWithScore` or `AgentSearchHit`. Each consumer hydrates its own
//!     shape so we don't accumulate cross-consumer fields here.
//!   - Recency bonuses, FTS-top-N boosts, accent-normalized scorers,
//!     direction filters, LLM relevance gates all stay in the consumer that
//!     owns them. They get composed *around* this module, not into it.

use crate::db::Database;
use crate::models::error::Result;

pub mod dedup;
pub mod rrf;

pub use dedup::dedup_by_thread;
pub use rrf::{fuse_rrf, Ranking, DEFAULT_RRF_K};

/// Inputs for an FTS5 candidate fetch.
///
/// `sender_email_eq` pushes a sender pin into SQL so bm25 top-K stays balanced
/// when the consumer wants "messages I sent" (without it, dense received
/// subjects dominate the ranking — see the regression test in
/// `db::embeddings::tests::fts_search_sender_email_eq_returns_only_matching_sender`).
pub struct FtsRequest<'a> {
    pub account_id: &'a str,
    pub query: &'a str,
    pub categories: Option<&'a [String]>,
    pub sender_email_eq: Option<&'a str>,
    pub limit: i32,
}

/// Run an FTS5 candidate fetch. Returns `(email_id, bm25)` where lower bm25 is
/// better (SQLite convention).
pub fn fetch_fts(db: &Database, req: FtsRequest<'_>) -> Result<Vec<(String, f64)>> {
    db.fts_search_filtered(
        req.query,
        Some(req.account_id),
        req.categories,
        req.sender_email_eq,
        req.limit,
    )
}

/// Inputs for a vector KNN fetch.
pub struct VectorRequest<'a> {
    pub account_id: &'a str,
    pub embedding: &'a [f32],
    pub categories: Option<&'a [String]>,
    pub limit: usize,
}

/// Run a vector KNN fetch. Returns `(email_id, similarity)` where higher is
/// better — `db.vec_search` already converts the underlying distance into a
/// similarity score and dedupes by email_id.
pub fn fetch_vector(db: &Database, req: VectorRequest<'_>) -> Result<Vec<(String, f32)>> {
    db.vec_search(req.embedding, Some(req.account_id), req.categories, req.limit)
}
