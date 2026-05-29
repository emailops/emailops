//! Embed candidate memory facts so they can be recalled via hybrid search.
//!
//! Facts are short (one sentence) so we don't chunk — one vector per fact.
//! Embedding happens in `ai_background` jobs kicked off by the consolidation
//! tick and the extractor.

use std::sync::Arc;

use tauri::AppHandle;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::MemoryFact;
use crate::services::ai::AiService;

const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text";
const MAX_BATCH: i32 = 64;

/// Embed up to `MAX_BATCH` candidate facts for `account_id` that don't yet
/// have an embedding row. Best-effort: provider failures log and return the
/// count successfully embedded so far.
pub async fn embed_pending_facts(db: &Arc<Database>, app: &AppHandle, account_id: &str) -> Result<u32> {
    let rows = db.list_facts_needing_embedding(account_id, MAX_BATCH)?;
    if rows.is_empty() {
        return Ok(0);
    }

    let ai = match AiService::new(db.clone(), Some(app.clone())) {
        Ok(svc) => svc,
        Err(e) => {
            emit_log(
                app,
                "warn",
                "memory",
                &format!("fact embedding disabled (no AI provider): {e}"),
            );
            return Ok(0);
        }
    };

    let model = db
        .get_preference("ai_embedding_model")
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string());

    emit_log(app, "info", "memory", &format!("Embedding {} memory facts", rows.len()));

    let mut ok: u32 = 0;
    for (fact_id, text) in &rows {
        match ai.embed(text).await {
            Ok(vec) => {
                if let Err(e) = db.upsert_fact_embedding(fact_id, &vec, &model) {
                    emit_log(
                        app,
                        "warn",
                        "memory",
                        &format!("upsert_fact_embedding failed for {fact_id}: {e}"),
                    );
                } else {
                    ok += 1;
                }
            }
            Err(e) => {
                emit_log(app, "warn", "memory", &format!("embed failed for {fact_id}: {e}"));
            }
        }
    }
    emit_log(
        app,
        "success",
        "memory",
        &format!("Embedded {}/{} facts", ok, rows.len()),
    );
    Ok(ok)
}

/// Hybrid search over facts: vector KNN + FTS5, fused via RRF. Mirrors the
/// pattern used for email retrieval in `services/chat.rs`.
pub async fn hybrid_search_facts(
    db: &Arc<Database>,
    app: Option<&AppHandle>,
    account_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<MemoryFact>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    // FTS first — always available, very fast.
    let fts_hits = db
        .search_memory_facts_fts(account_id, query, limit as i32 * 2)
        .unwrap_or_default();

    // Vector path — optional (requires embedding provider).
    let vec_hits: Vec<(MemoryFact, f32)> = if let Some(app_handle) = app {
        match AiService::new(db.clone(), Some(app_handle.clone())) {
            Ok(ai) => match ai.embed(query).await {
                Ok(v) => db
                    .vec_search_memory_facts(&v, account_id, limit as i32 * 2)
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    Ok(rrf_fuse(fts_hits, vec_hits, limit))
}

/// Reciprocal Rank Fusion. `k=60` is the classic default.
fn rrf_fuse(fts: Vec<(MemoryFact, f64)>, vec: Vec<(MemoryFact, f32)>, limit: usize) -> Vec<MemoryFact> {
    use std::collections::HashMap;
    const K: f64 = 60.0;

    let mut scores: HashMap<String, (MemoryFact, f64)> = HashMap::new();

    for (rank, (fact, _)) in fts.into_iter().enumerate() {
        let s = 1.0 / (K + rank as f64 + 1.0);
        scores
            .entry(fact.id.clone())
            .and_modify(|e| e.1 += s)
            .or_insert((fact, s));
    }
    for (rank, (fact, _)) in vec.into_iter().enumerate() {
        let s = 1.0 / (K + rank as f64 + 1.0);
        scores
            .entry(fact.id.clone())
            .and_modify(|e| e.1 += s)
            .or_insert((fact, s));
    }

    let mut out: Vec<(MemoryFact, f64)> = scores.into_values().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(limit);
    out.into_iter().map(|(f, _)| f).collect()
}

fn emit_log(_app: &AppHandle, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MemoryFact;

    fn fact(id: &str) -> MemoryFact {
        MemoryFact {
            id: id.to_string(),
            account_id: "a1".into(),
            subject_kind: "contact".into(),
            subject_key: "x@ex.com".into(),
            fact: format!("fact body {id}"),
            source: "extraction".into(),
            source_email_id: None,
            confidence: 0.5,
            score: 0.0,
            status: "candidate".into(),
            last_used_at: None,
            domain: None,
            vigency: None,
            company: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn rrf_prefers_items_appearing_in_both_lists() {
        let fts = vec![(fact("a"), 1.0), (fact("b"), 0.5), (fact("c"), 0.3)];
        let vec = vec![(fact("b"), 0.1), (fact("c"), 0.2), (fact("d"), 0.4)];
        let fused = rrf_fuse(fts, vec, 3);
        let ids: Vec<_> = fused.iter().map(|f| f.id.as_str()).collect();
        // `b` and `c` appear in both → should outrank `a`/`d`.
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
    }

    #[test]
    fn rrf_respects_limit() {
        let fts = (0..10)
            .map(|i| (fact(&format!("f{i}")), 1.0 - i as f64 * 0.1))
            .collect();
        let vec = Vec::new();
        let fused = rrf_fuse(fts, vec, 3);
        assert_eq!(fused.len(), 3);
    }
}
