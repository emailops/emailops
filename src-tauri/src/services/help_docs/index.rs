//! Keep `help_doc_chunks` in step with the guides compiled into the binary.
//!
//! Two independent halves, so text search works before the (slower) vectors
//! exist:
//!   - [`ensure_text_index`] — synchronous, cheap: hash the compiled-in
//!     corpus, and when it differs from the hash stored at the last build,
//!     replace every row + FTS entry in one transaction.
//!   - [`ensure_embeddings`] — async: embed every chunk that has no vector
//!     for the active embedding model, in small batches. Runs on the AI
//!     queue from `prewarm_chat` (app) and before a turn in the CLI/evals.

use std::sync::Arc;

use crate::ai::provider::AIProvider;
use crate::db::help_docs::PREF_HELP_CORPUS_HASH;
use crate::db::Database;
use crate::models::error::Result;

use super::corpus::{corpus, corpus_hash, embedding_text};

/// Chunks embedded per provider call.
const EMBED_BATCH: usize = 8;
/// Same default the memory subsystem uses when the preference is unset —
/// the label only has to be consistent between write and read.
const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text";

/// The label vectors are tagged with: the configured embedding model, so a
/// model switch re-embeds the corpus instead of mixing vector spaces.
pub fn embedding_model_label(db: &Database) -> String {
    db.get_preference("ai_embedding_model")
        .ok()
        .flatten()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string())
}

/// Pure: does the stored index match the compiled-in corpus? A missing hash
/// or an empty table (a fresh DB, or a migration that recreated the tables)
/// both mean "rebuild".
pub fn needs_text_rebuild(stored_hash: Option<&str>, current_hash: &str, stored_rows: i64) -> bool {
    stored_rows == 0 || stored_hash != Some(current_hash)
}

/// Rebuild the text index when the corpus changed. Returns `true` when it
/// rebuilt. Cheap when nothing changed (one hash + one COUNT).
pub fn ensure_text_index(db: &Database) -> Result<bool> {
    let current = corpus_hash();
    let stored = db.get_preference(PREF_HELP_CORPUS_HASH)?;
    let rows = db.count_help_doc_chunks()?;
    if !needs_text_rebuild(stored.as_deref(), &current, rows) {
        return Ok(false);
    }
    let rows: Vec<(crate::models::HelpChunk, String)> = corpus().iter().map(|c| (c.clone(), fts_heading(c))).collect();
    let n = db.replace_help_doc_chunks(&rows)?;
    db.set_preference(PREF_HELP_CORPUS_HASH, &current)?;
    log(
        "info",
        format!("help index: rebuilt {n} guide sections from the bundled docs"),
    );
    Ok(true)
}

/// What the FTS `heading` column indexes: page title and heading, so a
/// query naming the feature hits the section.
fn fts_heading(c: &crate::models::HelpChunk) -> String {
    if c.heading == c.page_title {
        c.page_title.clone()
    } else {
        format!("{} › {}", c.page_title, c.heading)
    }
}

/// Embed every chunk missing a vector for the active model. Best-effort:
/// a provider failure logs and returns how many were embedded so far, so a
/// turn can still run FTS-only. Returns the number embedded this call.
pub async fn ensure_embeddings(db: &Arc<Database>, provider: &dyn AIProvider) -> Result<u32> {
    let model = embedding_model_label(db);
    let pending = db.list_help_chunks_needing_embedding(&model, i32::MAX)?;
    if pending.is_empty() {
        return Ok(0);
    }
    log(
        "info",
        format!("help index: embedding {} guide sections with {model}", pending.len()),
    );
    let mut done: u32 = 0;
    for batch in pending.chunks(EMBED_BATCH) {
        let texts: Vec<String> = batch.iter().map(|(_, c)| embedding_text(c)).collect();
        let results = match provider.embed_batch(&texts).await {
            Ok(r) => r,
            Err(e) => {
                log(
                    "warn",
                    format!("help index: embedding stopped after {done} sections ({e}); text search still works"),
                );
                return Ok(done);
            }
        };
        for ((rowid, chunk), r) in batch.iter().zip(results) {
            if let Err(e) = db.upsert_help_chunk_embedding(*rowid, &r.embedding, &model) {
                log(
                    "warn",
                    format!("help index: could not store the vector for {} ({e})", chunk.chunk_id),
                );
                continue;
            }
            done += 1;
        }
    }
    log("success", format!("help index: {done} guide sections embedded"));
    Ok(done)
}

/// Text index first (synchronous), then vectors. The one call the app's
/// prewarm and the CLI make before a chat turn.
pub async fn ensure_index(db: &Arc<Database>, provider: &dyn AIProvider) -> Result<()> {
    ensure_text_index(db)?;
    ensure_embeddings(db, provider).await?;
    Ok(())
}

fn log(level: &str, message: impl Into<String>) {
    crate::services::logger::log(level, "ai", message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_when_hash_missing_or_stale_or_table_empty() {
        assert!(needs_text_rebuild(None, "abc", 10));
        assert!(needs_text_rebuild(Some("old"), "abc", 10));
        assert!(needs_text_rebuild(Some("abc"), "abc", 0));
        assert!(!needs_text_rebuild(Some("abc"), "abc", 10));
    }

    #[test]
    fn ensure_text_index_builds_once_then_is_a_no_op() {
        let db = Database::new_for_testing().unwrap();
        assert!(ensure_text_index(&db).unwrap());
        assert_eq!(db.count_help_doc_chunks().unwrap() as usize, corpus().len());
        assert!(!ensure_text_index(&db).unwrap());
        assert_eq!(
            db.get_preference(PREF_HELP_CORPUS_HASH).unwrap().as_deref(),
            Some(corpus_hash().as_str())
        );
    }

    #[test]
    fn ensure_text_index_rebuilds_after_a_stale_hash() {
        let db = Database::new_for_testing().unwrap();
        ensure_text_index(&db).unwrap();
        db.set_preference(PREF_HELP_CORPUS_HASH, "stale").unwrap();
        assert!(ensure_text_index(&db).unwrap());
        assert_eq!(db.count_help_doc_chunks().unwrap() as usize, corpus().len());
    }

    #[test]
    fn text_index_finds_sections_by_feature_name_in_every_language() {
        let db = Database::new_for_testing().unwrap();
        ensure_text_index(&db).unwrap();
        for q in ["Ollama", "OpenRouter", "calendario", "Kalender", "calendrier"] {
            assert!(
                !db.fts_search_help_docs(q, 5, None).unwrap().is_empty(),
                "no FTS hit for {q}"
            );
        }
    }

    #[test]
    fn embedding_label_falls_back_to_default() {
        let db = Database::new_for_testing().unwrap();
        assert_eq!(embedding_model_label(&db), DEFAULT_EMBEDDING_MODEL);
        db.set_preference("ai_embedding_model", "bge-m3").unwrap();
        assert_eq!(embedding_model_label(&db), "bge-m3");
    }

    #[tokio::test]
    async fn ensure_embeddings_fills_every_chunk_with_the_fake_provider() {
        use crate::ai::provider::FakeAiProvider;
        let db = Arc::new(Database::new_for_testing().unwrap());
        ensure_text_index(&db).unwrap();
        let provider = FakeAiProvider::default().with_embedding_dim(768);
        let n = ensure_embeddings(&db, &provider).await.unwrap();
        assert_eq!(n as usize, corpus().len());
        assert_eq!(
            ensure_embeddings(&db, &provider).await.unwrap(),
            0,
            "second run is a no-op"
        );
        let model = embedding_model_label(&db);
        assert_eq!(
            db.count_help_chunks_embedded_with(&model).unwrap() as usize,
            corpus().len()
        );
    }
}
