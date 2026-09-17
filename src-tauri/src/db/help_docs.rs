//! SQL for the "EmailOps help" corpus (`help_doc_chunks`, `help_docs_fts`,
//! `vec_help_docs`): rebuild, embedding upsert, FTS / KNN candidate fetch
//! and row hydration. Ranking, gating and language selection live in
//! `services::help_docs::retrieval`.

use rusqlite::params;

use super::Database;
use crate::models::error::Result;
use crate::models::HelpChunk;

/// Preference holding the fingerprint of the corpus the tables were built
/// from; a mismatch with the compiled-in corpus triggers a rebuild.
pub const PREF_HELP_CORPUS_HASH: &str = "help_docs.corpus_hash";

fn blob(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for v in embedding {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

const CHUNK_COLS: &str =
    "rowid, chunk_id, lang, page, section_index, part, anchor, page_title, heading, content, nav_target";

fn row_to_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, HelpChunk)> {
    Ok((
        row.get(0)?,
        HelpChunk {
            chunk_id: row.get(1)?,
            lang: row.get(2)?,
            page: row.get(3)?,
            section_index: row.get(4)?,
            part: row.get(5)?,
            anchor: row.get(6)?,
            page_title: row.get(7)?,
            heading: row.get(8)?,
            content: row.get(9)?,
            nav_target: row.get(10)?,
        },
    ))
}

impl Database {
    /// Per-feature gate for answering questions about the app from the
    /// bundled guides. Default `true` (mirrors `useHelpDocsEnabledStore`).
    pub fn is_help_docs_enabled(&self) -> Result<bool> {
        Ok(self
            .get_preference("help_docs_enabled")?
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(true))
    }

    /// Replace the whole corpus in one transaction: every chunk row, its FTS
    /// row, and (implicitly) every embedding — a rebuild means the text
    /// changed, so old vectors are wrong. `fts_heading` is what the FTS
    /// `heading` column indexes for each chunk (page title + heading).
    pub fn replace_help_doc_chunks(&self, chunks: &[(HelpChunk, String)]) -> Result<usize> {
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM vec_help_docs", [])?;
        tx.execute("DELETE FROM help_docs_fts", [])?;
        tx.execute("DELETE FROM help_doc_chunks", [])?;
        let now = chrono::Utc::now().timestamp();
        for (c, fts_heading) in chunks {
            tx.execute(
                "INSERT INTO help_doc_chunks
                   (chunk_id, lang, page, section_index, part, anchor, page_title, heading, content, nav_target, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    c.chunk_id,
                    c.lang,
                    c.page,
                    c.section_index,
                    c.part,
                    c.anchor,
                    c.page_title,
                    c.heading,
                    c.content,
                    c.nav_target,
                    now
                ],
            )?;
            let rowid = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO help_docs_fts (chunk_rowid, heading, content) VALUES (?1, ?2, ?3)",
                params![rowid, fts_heading, c.content],
            )?;
        }
        tx.commit()?;
        Ok(chunks.len())
    }

    pub fn count_help_doc_chunks(&self) -> Result<i64> {
        let conn = self.reader();
        Ok(conn.query_row("SELECT COUNT(*) FROM help_doc_chunks", [], |r| r.get(0))?)
    }

    /// Chunks with no embedding for `model` yet (never embedded, or embedded
    /// with a different model), oldest first.
    pub fn list_help_chunks_needing_embedding(&self, model: &str, limit: i32) -> Result<Vec<(i64, HelpChunk)>> {
        let conn = self.reader();
        let sql = format!(
            "SELECT {CHUNK_COLS} FROM help_doc_chunks
             WHERE embedding_model IS NULL OR embedding_model != ?1
             ORDER BY rowid LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![model, limit], row_to_chunk)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn count_help_chunks_embedded_with(&self, model: &str) -> Result<i64> {
        let conn = self.reader();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM help_doc_chunks WHERE embedding_model = ?1",
            params![model],
            |r| r.get(0),
        )?)
    }

    /// Store (or replace) the vector for one chunk and tag the row with the
    /// model that produced it. Read-then-write on the write connection.
    pub fn upsert_help_chunk_embedding(&self, rowid: i64, embedding: &[f32], model: &str) -> Result<()> {
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM vec_help_docs WHERE rowid = ?1", params![rowid])?;
        tx.execute(
            "INSERT INTO vec_help_docs (rowid, embedding) VALUES (?1, ?2)",
            params![rowid, blob(embedding)],
        )?;
        tx.execute(
            "UPDATE help_doc_chunks SET embedding_model = ?2 WHERE rowid = ?1",
            params![rowid, model],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// FTS5 candidates as `(rowid, bm25)`, lower bm25 = better. The query
    /// goes through the same escaper as mailbox search so punctuation never
    /// reaches the FTS parser.
    pub fn fts_search_help_docs(&self, query: &str, limit: i32) -> Result<Vec<(i64, f64)>> {
        // The whole corpus is about EmailOps: the name is in every question
        // and most sections, so as an FTS term it only ranks sections by how
        // often they repeat it. Drop it before the escaper sees it.
        let stripped: String = query
            .split_whitespace()
            .filter(|w| {
                !w.trim_matches(|c: char| !c.is_alphanumeric())
                    .eq_ignore_ascii_case("emailops")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let fts_query = super::embeddings::escape_fts_query(&stripped);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT chunk_rowid, bm25(help_docs_fts, 0.0, 3.0, 1.0) AS rank
             FROM help_docs_fts
             WHERE help_docs_fts MATCH ?1
             ORDER BY rank LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![fts_query, limit], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// KNN candidates as `(rowid, cosine similarity)`, best first, restricted
    /// to rows embedded with `model` so a half-migrated corpus never mixes
    /// vector spaces.
    pub fn vec_search_help_docs(&self, embedding: &[f32], model: &str, limit: usize) -> Result<Vec<(i64, f32)>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT rowid, distance FROM vec_help_docs
             WHERE embedding MATCH ?1
               AND rowid IN (SELECT rowid FROM help_doc_chunks WHERE embedding_model = ?3)
             ORDER BY distance LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![blob(embedding), limit as i64, model], |row| {
                let d: f32 = row.get(1)?;
                Ok((row.get::<_, i64>(0)?, 1.0 - d))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_help_chunks_by_rowids(&self, rowids: &[i64]) -> Result<Vec<(i64, HelpChunk)>> {
        if rowids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: Vec<String> = (1..=rowids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT {CHUNK_COLS} FROM help_doc_chunks WHERE rowid IN ({})",
            placeholders.join(", ")
        );
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let bound: Vec<&dyn rusqlite::ToSql> = rowids.iter().map(|r| r as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(bound.as_slice(), row_to_chunk)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every chunk of the given `(page, section_index)` sections in `lang` —
    /// the siblings a hit in another language is swapped for.
    pub fn get_help_chunk_siblings(&self, sections: &[(String, i32)], lang: &str) -> Result<Vec<HelpChunk>> {
        let conn = self.reader();
        let sql = format!(
            "SELECT {CHUNK_COLS} FROM help_doc_chunks
             WHERE lang = ?1 AND page = ?2 AND section_index = ?3
             ORDER BY part"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut out = Vec::new();
        for (page, section) in sections {
            let rows = stmt
                .query_map(params![lang, page, section], row_to_chunk)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.extend(rows.into_iter().map(|(_, c)| c));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &str, lang: &str, page: &str, section: i32, heading: &str, content: &str) -> (HelpChunk, String) {
        (
            HelpChunk {
                chunk_id: id.to_string(),
                lang: lang.to_string(),
                page: page.to_string(),
                section_index: section,
                part: 0,
                anchor: heading.to_lowercase().replace(' ', "-"),
                page_title: "AI features".into(),
                heading: heading.to_string(),
                content: content.to_string(),
                nav_target: Some("settings/ai".into()),
            },
            format!("AI features › {heading}"),
        )
    }

    fn seeded() -> Database {
        let db = Database::new_for_testing().expect("db");
        db.replace_help_doc_chunks(&[
            chunk(
                "en/ai-features#1.0",
                "en",
                "ai-features",
                1,
                "Choosing a backend",
                "Ollama runs at localhost:11434.",
            ),
            chunk(
                "es/ai-features#1.0",
                "es",
                "ai-features",
                1,
                "Elegir un backend",
                "Ollama escucha en localhost:11434.",
            ),
            chunk(
                "en/ai-features#2.0",
                "en",
                "ai-features",
                2,
                "The model catalog",
                "Qwen 3.5 4B needs 8 GB.",
            ),
        ])
        .expect("seed");
        db
    }

    #[test]
    fn replace_is_idempotent_and_counts_rows() {
        let db = seeded();
        assert_eq!(db.count_help_doc_chunks().unwrap(), 3);
        db.replace_help_doc_chunks(&[chunk("en/x#1.0", "en", "x", 1, "H", "c")])
            .unwrap();
        assert_eq!(db.count_help_doc_chunks().unwrap(), 1);
    }

    #[test]
    fn fts_finds_by_heading_and_content() {
        let db = seeded();
        let hits = db.fts_search_help_docs("ollama", 10).unwrap();
        assert_eq!(hits.len(), 2, "both languages mention Ollama");
        let hits = db.fts_search_help_docs("catalog", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(db.fts_search_help_docs("?!", 10).unwrap().is_empty());
    }

    /// The whole corpus is about EmailOps, so the app's name carries no
    /// signal and, left in, ranks sections by how often they repeat it.
    #[test]
    fn fts_ignores_the_app_name() {
        let db = Database::new_for_testing().expect("db");
        db.replace_help_doc_chunks(&[
            chunk(
                "en/privacy-security#0.0",
                "en",
                "privacy-security",
                0,
                "Privacy",
                "EmailOps keeps your mail on your machine. EmailOps never uploads it.",
            ),
            chunk(
                "en/ai-features#1.0",
                "en",
                "ai-features",
                1,
                "Choosing a backend",
                "Ollama runs at localhost:11434.",
            ),
        ])
        .expect("seed");
        assert!(db.fts_search_help_docs("EmailOps", 10).unwrap().is_empty());
        assert!(db.fts_search_help_docs("emailops?", 10).unwrap().is_empty());
        let with = db.fts_search_help_docs("Ollama in EmailOps", 10).unwrap();
        let without = db.fts_search_help_docs("Ollama", 10).unwrap();
        assert_eq!(with, without);
    }

    #[test]
    fn embeddings_are_tagged_with_model_and_searched_per_model() {
        let db = seeded();
        let pending = db.list_help_chunks_needing_embedding("m1", 10).unwrap();
        assert_eq!(pending.len(), 3);
        let mut v = vec![0.0f32; 768];
        v[0] = 1.0;
        for (rowid, _) in &pending {
            db.upsert_help_chunk_embedding(*rowid, &v, "m1").unwrap();
        }
        assert_eq!(db.count_help_chunks_embedded_with("m1").unwrap(), 3);
        assert!(db.list_help_chunks_needing_embedding("m1", 10).unwrap().is_empty());
        assert_eq!(db.list_help_chunks_needing_embedding("m2", 10).unwrap().len(), 3);

        let hits = db.vec_search_help_docs(&v, "m1", 5).unwrap();
        assert_eq!(hits.len(), 3);
        assert!((hits[0].1 - 1.0).abs() < 1e-4, "identical vector → similarity 1");
        assert!(db.vec_search_help_docs(&v, "m2", 5).unwrap().is_empty());
    }

    #[test]
    fn re_embedding_replaces_the_vector() {
        let db = seeded();
        let (rowid, _) = db.list_help_chunks_needing_embedding("m1", 1).unwrap().remove(0);
        let mut a = vec![0.0f32; 768];
        a[0] = 1.0;
        let mut b = vec![0.0f32; 768];
        b[1] = 1.0;
        db.upsert_help_chunk_embedding(rowid, &a, "m1").unwrap();
        db.upsert_help_chunk_embedding(rowid, &b, "m1").unwrap();
        let hits = db.vec_search_help_docs(&b, "m1", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!((hits[0].1 - 1.0).abs() < 1e-4);
    }

    #[test]
    fn hydrates_rows_and_siblings() {
        let db = seeded();
        let hits = db.fts_search_help_docs("catalog", 10).unwrap();
        let rows = db.get_help_chunks_by_rowids(&[hits[0].0]).unwrap();
        assert_eq!(rows[0].1.chunk_id, "en/ai-features#2.0");
        let sib = db.get_help_chunk_siblings(&[("ai-features".into(), 1)], "es").unwrap();
        assert_eq!(sib.len(), 1);
        assert_eq!(sib[0].chunk_id, "es/ai-features#1.0");
        assert!(db
            .get_help_chunk_siblings(&[("ai-features".into(), 2)], "es")
            .unwrap()
            .is_empty());
        assert!(db.get_help_chunks_by_rowids(&[]).unwrap().is_empty());
    }

    #[test]
    fn help_docs_enabled_defaults_to_true() {
        let db = Database::new_for_testing().unwrap();
        assert!(db.is_help_docs_enabled().unwrap());
        db.set_preference("help_docs_enabled", "false").unwrap();
        assert!(!db.is_help_docs_enabled().unwrap());
    }
}
