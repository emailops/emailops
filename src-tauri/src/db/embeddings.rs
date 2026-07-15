use std::collections::HashMap;

use crate::db::Database;
use crate::models::error::Result;
use rusqlite::params;

/// Convert f32 embedding to raw little-endian bytes for sqlite-vec
fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

impl Database {
    /// Store embedding chunks for an email (one or more chunks).
    /// Deletes any existing chunks for the email first.
    pub fn store_embedding_chunks(
        &self,
        email_id: &str,
        account_id: &str,
        embeddings: &[Vec<f32>],
        model: &str,
        content_hash: &str,
    ) -> Result<()> {
        let conn = self.connection();
        let now = chrono::Utc::now().timestamp();

        // Delete existing chunks for this email
        conn.execute(
            "DELETE FROM vec_emails WHERE rowid IN (SELECT rowid FROM embedding_chunks WHERE email_id = ?1)",
            params![email_id],
        )?;
        conn.execute("DELETE FROM embedding_chunks WHERE email_id = ?1", params![email_id])?;

        // Insert new chunks
        for (idx, embedding) in embeddings.iter().enumerate() {
            conn.execute(
                "INSERT INTO embedding_chunks (email_id, account_id, chunk_index, embedding_model, content_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![email_id, account_id, idx as i32, model, content_hash, now],
            )?;
            let rowid = conn.last_insert_rowid();
            let blob = embedding_to_blob(embedding);
            conn.execute(
                "INSERT INTO vec_emails (rowid, embedding) VALUES (?1, ?2)",
                params![rowid, blob],
            )?;
        }

        Ok(())
    }

    /// KNN vector search using sqlite-vec.
    /// Returns (email_id, similarity_score) pairs, deduplicated by email_id (best chunk wins).
    ///
    /// Splits the work into three small index-driven queries instead of one big
    /// JOIN. The prior JOIN-with-account-filter produced a pathological plan
    /// (15 s on 47k emails); these three steps total <200 ms on the same data:
    ///   1. KNN on vec_emails (sqlite-vec)            — ~100 ms
    ///   2. rowid → email_id via embedding_chunks PK  — ~20 ms
    ///   3. filter email_ids by account_id/category   — ~20 ms
    pub fn vec_search(
        &self,
        query_embedding: &[f32],
        account_id: Option<&str>,
        categories: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        let blob = embedding_to_blob(query_embedding);

        // Step 1: KNN from vec0 (broad fetch). Read-only, use the pool.
        // When an account is specified, pre-filter rowids via embedding_chunks
        // so each account effectively has an independent vector store at the
        // KNN level — vectors from other accounts don't compete for the top-K.
        let expanded_limit = (limit * 5) as i32;
        let knn_results: Vec<(i64, f32)> = {
            let conn = self.reader();
            if let Some(acc) = account_id {
                let mut stmt = conn.prepare(
                    "SELECT rowid, distance FROM vec_emails
                     WHERE embedding MATCH ?1
                       AND rowid IN (SELECT rowid FROM embedding_chunks WHERE account_id = ?3)
                     ORDER BY distance LIMIT ?2",
                )?;
                let rows: Vec<(i64, f32)> = stmt
                    .query_map(params![blob, expanded_limit, acc], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .filter_map(|r| r.ok())
                    .collect();
                rows
            } else {
                let mut stmt = conn.prepare(
                    "SELECT rowid, distance FROM vec_emails WHERE embedding MATCH ?1 ORDER BY distance LIMIT ?2",
                )?;
                let rows: Vec<(i64, f32)> = stmt
                    .query_map(params![blob, expanded_limit], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .filter_map(|r| r.ok())
                    .collect();
                rows
            }
        };

        if knn_results.is_empty() {
            return Ok(Vec::new());
        }

        let distance_map: HashMap<i64, f32> = knn_results.into_iter().collect();
        let rowids: Vec<i64> = distance_map.keys().copied().collect();

        // Step 2: rowid → email_id via embedding_chunks PK (NO JOIN — that query
        // plan is catastrophic on large emails tables).
        let placeholders: String = (0..rowids.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let rowid_to_email: Vec<(i64, String)> = {
            let conn = self.reader();
            let sql = format!(
                "SELECT ec.rowid, ec.email_id FROM embedding_chunks ec WHERE ec.rowid IN ({})",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let params_vec: Vec<Box<dyn rusqlite::ToSql>> = rowids
                .iter()
                .map(|r| Box::new(*r) as Box<dyn rusqlite::ToSql>)
                .collect();
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
            let rows: Vec<(i64, String)> = stmt
                .query_map(params_refs.as_slice(), |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        if rowid_to_email.is_empty() {
            return Ok(Vec::new());
        }

        // Step 3: narrow by a second indexed query against emails. Each row
        // checks PK + indexed columns, no table scan. Always runs — even with
        // no account/category filter, spam/trash rows must never be retrieved
        // (a spam email classified `primary` sails through the category filter).
        let filtered: Vec<(i64, String)> = {
            let unique_emails: Vec<String> = rowid_to_email
                .iter()
                .map(|(_, eid)| eid.clone())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            let eid_phs: String = (0..unique_emails.len())
                .map(|i| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(", ");

            let mut sql = format!(
                "SELECT id FROM emails WHERE id IN ({}) AND mailbox NOT IN ('spam', 'trash')",
                eid_phs
            );
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = unique_emails
                .iter()
                .map(|e| Box::new(e.clone()) as Box<dyn rusqlite::ToSql>)
                .collect();
            let mut next_idx = unique_emails.len() + 1;

            if let Some(acc) = account_id {
                sql.push_str(&format!(" AND account_id = ?{}", next_idx));
                params_vec.push(Box::new(acc.to_string()));
                next_idx += 1;
            }
            if let Some(cats) = categories.filter(|c| !c.is_empty()) {
                let cat_phs: Vec<String> = (0..cats.len()).map(|i| format!("?{}", next_idx + i)).collect();
                sql.push_str(&format!(" AND category IN ({})", cat_phs.join(", ")));
                for cat in cats {
                    params_vec.push(Box::new(cat.clone()));
                }
            }

            let allowed: std::collections::HashSet<String> = {
                let conn = self.reader();
                let mut stmt = conn.prepare(&sql)?;
                let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
                let rows: std::collections::HashSet<String> = stmt
                    .query_map(params_refs.as_slice(), |row| row.get::<_, String>(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                rows
            };
            rowid_to_email
                .into_iter()
                .filter(|(_, eid)| allowed.contains(eid))
                .collect()
        };

        // Step 4: dedup by email_id, keep best similarity
        let mut best_per_email: HashMap<String, f32> = HashMap::new();
        for (rowid, email_id) in &filtered {
            if let Some(&distance) = distance_map.get(rowid) {
                let similarity = 1.0 - distance;
                let entry = best_per_email.entry(email_id.clone()).or_insert(0.0);
                if similarity > *entry {
                    *entry = similarity;
                }
            }
        }

        let mut results: Vec<(String, f32)> = best_per_email.into_iter().collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Check if an embedding exists and matches the content hash
    pub fn embedding_exists(&self, email_id: &str, content_hash: &str) -> Result<bool> {
        let conn = self.connection();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM embedding_chunks WHERE email_id = ?1 AND content_hash = ?2",
            params![email_id, content_hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get emails that don't have embeddings yet.
    ///
    /// `categories` filters to emails matching one of the given Gmail-style
    /// categories. Empty slice = no category filter (all categories).
    ///
    /// `min_timestamp` excludes emails older than the given unix-seconds
    /// cutoff (typically `Database::ai_processing_min_timestamp`). `None`
    /// means no age cutoff.
    pub fn get_emails_without_embeddings(
        &self,
        account_id: Option<&str>,
        limit: i32,
        categories: &[String],
        min_timestamp: Option<i64>,
    ) -> Result<Vec<String>> {
        let conn = self.connection();

        let mut sql = String::from(
            // Spam/trash are never embedded — retrieval must never surface
            // them, so embedding them is pure wasted compute. pending_sync = 0:
            // optimistic sent copies awaiting reconciliation are deleted when
            // the provider's real copy arrives, so they are skipped too.
            "SELECT e.id FROM emails e
             LEFT JOIN embedding_chunks ec ON e.id = ec.email_id
             WHERE ec.email_id IS NULL AND e.is_deleted = 0
               AND e.mailbox NOT IN ('spam', 'trash') AND e.pending_sync = 0",
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(acc) = account_id {
            params_vec.push(Box::new(acc.to_string()));
            sql.push_str(&format!(" AND e.account_id = ?{}", params_vec.len()));
        }
        if !categories.is_empty() {
            let placeholders: Vec<String> = (0..categories.len())
                .map(|i| format!("?{}", params_vec.len() + i + 1))
                .collect();
            sql.push_str(&format!(" AND e.category IN ({})", placeholders.join(",")));
            for c in categories {
                params_vec.push(Box::new(c.clone()));
            }
        }
        if let Some(ts) = min_timestamp {
            params_vec.push(Box::new(ts));
            sql.push_str(&format!(" AND e.timestamp >= ?{}", params_vec.len()));
        }
        params_vec.push(Box::new(limit));
        sql.push_str(&format!(" LIMIT ?{}", params_vec.len()));

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(params_refs.as_slice(), |row| row.get(0))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Count emails without embeddings
    pub fn count_emails_without_embeddings(&self, account_id: Option<&str>) -> Result<i32> {
        let conn = self.connection();

        // Filters mirror get_emails_without_embeddings (deletion + mailbox) so
        // the pending count reaches zero when the embed loop finishes.
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::ToSql>>) = match account_id {
            Some(acc) => (
                "SELECT COUNT(*) FROM emails e
                 LEFT JOIN embedding_chunks ec ON e.id = ec.email_id
                 WHERE ec.email_id IS NULL AND e.is_deleted = 0
                   AND e.mailbox NOT IN ('spam', 'trash') AND e.pending_sync = 0
                   AND e.account_id = ?1"
                    .to_string(),
                vec![Box::new(acc.to_string())],
            ),
            None => (
                "SELECT COUNT(*) FROM emails e
                 LEFT JOIN embedding_chunks ec ON e.id = ec.email_id
                 WHERE ec.email_id IS NULL AND e.is_deleted = 0
                   AND e.mailbox NOT IN ('spam', 'trash') AND e.pending_sync = 0"
                    .to_string(),
                vec![],
            ),
        };

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let count: i32 = stmt.query_row(params_refs.as_slice(), |row| row.get(0))?;
        Ok(count)
    }

    /// Delete embeddings for an email
    pub fn delete_embedding(&self, email_id: &str) -> Result<()> {
        let conn = self.connection();
        let mut stmt = conn.prepare("SELECT rowid FROM embedding_chunks WHERE email_id = ?1")?;
        let rowids: Vec<i64> = stmt
            .query_map(params![email_id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        for rowid in &rowids {
            conn.execute("DELETE FROM vec_emails WHERE rowid = ?1", params![rowid])?;
        }
        conn.execute("DELETE FROM embedding_chunks WHERE email_id = ?1", params![email_id])?;
        Ok(())
    }

    /// Delete all embeddings, optionally filtered by account
    pub fn delete_all_embeddings(&self, account_id: Option<&str>) -> Result<u32> {
        let conn = self.connection();

        match account_id {
            Some(acc) => {
                conn.execute(
                    "DELETE FROM vec_emails WHERE rowid IN (
                        SELECT rowid FROM embedding_chunks WHERE account_id = ?1
                    )",
                    params![acc],
                )?;
                let deleted = conn.execute("DELETE FROM embedding_chunks WHERE account_id = ?1", params![acc])?;
                Ok(deleted as u32)
            }
            None => {
                conn.execute("DELETE FROM vec_emails", [])?;
                let deleted = conn.execute("DELETE FROM embedding_chunks", [])?;
                Ok(deleted as u32)
            }
        }
    }

    /// Full-text search using FTS5
    /// Returns email IDs with their BM25 relevance scores.
    pub fn fts_search(
        &self,
        query: &str,
        account_id: Option<&str>,
        categories: Option<&[String]>,
        limit: i32,
    ) -> Result<Vec<(String, f64)>> {
        self.fts_search_filtered(query, account_id, categories, None, limit)
    }

    /// Same as `fts_search` but lets the caller pin the sender to a specific
    /// address. Used by agent-search when the user's query implies a "sent by
    /// me" direction — pushing the filter into SQL keeps the bm25 top-K honest
    /// (without it, dense-subject received emails crowd out the few sent ones).
    pub fn fts_search_filtered(
        &self,
        query: &str,
        account_id: Option<&str>,
        categories: Option<&[String]>,
        sender_email_eq: Option<&str>,
        limit: i32,
    ) -> Result<Vec<(String, f64)>> {
        let conn = self.connection();

        let fts_query = escape_fts_query(query);

        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let mut sql = String::from(
            // bm25 weights match column order: email_id (UNINDEXED, ignored), subject, sender, body
            // Spam/trash never surface in retrieval — a spam email classified
            // `primary` must not sail through the category filter.
            r#"SELECT f.email_id, bm25(emails_fts, 0.0, 3.0, 2.0, 1.0) as rank
               FROM emails_fts f
               JOIN emails e ON f.email_id = e.id
               WHERE emails_fts MATCH ?1
                 AND e.mailbox NOT IN ('spam', 'trash')"#,
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(fts_query)];
        let mut param_idx = 2;

        if let Some(acc) = account_id {
            sql.push_str(&format!(" AND e.account_id = ?{}", param_idx));
            params_vec.push(Box::new(acc.to_string()));
            param_idx += 1;
        }

        if let Some(sender) = sender_email_eq {
            sql.push_str(&format!(" AND e.sender_email = ?{}", param_idx));
            params_vec.push(Box::new(sender.to_string()));
            param_idx += 1;
        }

        if let Some(categories) = categories.filter(|categories| !categories.is_empty()) {
            let placeholders: Vec<String> = categories
                .iter()
                .enumerate()
                .map(|(idx, _)| format!("?{}", param_idx + idx))
                .collect();
            sql.push_str(&format!(" AND e.category IN ({})", placeholders.join(", ")));
            for category in categories {
                params_vec.push(Box::new(category.clone()));
            }
            param_idx += categories.len();
        }

        sql.push_str(&format!(" ORDER BY rank LIMIT ?{}", param_idx));
        params_vec.push(Box::new(limit));

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            let email_id: String = row.get(0)?;
            let rank: f64 = row.get(1)?;
            Ok((email_id, rank))
        });

        match rows {
            Ok(rows) => {
                let mut results = Vec::new();
                for r in rows.flatten() {
                    results.push(r);
                }
                Ok(results)
            }
            Err(e) => {
                crate::services::logger::log("error", "embeddings", format!("FTS search failed: {}", e));
                Ok(Vec::new())
            }
        }
    }

    /// Rebuild FTS index from existing emails, stripping HTML from bodies.
    pub fn rebuild_fts_index(&self) -> Result<u32> {
        self.connection().execute("DELETE FROM emails_fts", [])?;
        self.populate_fts_from_emails()
    }
}

/// Common Spanish + English stopwords that add noise to FTS ranking when the
/// user's question is a full natural-language sentence. Keep conservative —
/// only truly low-content words. Anything domain-specific (even short) stays.
const FTS_STOPWORDS: &[&str] = &[
    // Spanish articles / prepositions / pronouns / connectors
    "de", "del", "la", "las", "el", "los", "un", "una", "unos", "unas", "al", "en", "y", "o", "u", "que", "qué", "como",
    "cómo", "con", "por", "para", "sin", "sobre", "entre", "pero", "mas", "si", "sí", "no", "ni", "me", "te", "se",
    "le", "les", "lo", "mi", "tu", "su", "sus", "mis", "tus", "yo", "tú", "él", "ella", "nos", "os", "ya", "muy",
    "más", "menos", "poco", "mucho", "ha", "he", "hay", "era", "es", "son", "fue", "ser", "estar", "esta", "este",
    "estos", "estas", "eso", "esa", "ese", "esos", "esas", "aquí", "ahí", "allí", "también", "tambien", "solo", "sólo",
    "dije", "di", "dice", "dijo", "parte", "cosa", "algo", "otro", "otra", "otros", "otras", "cada", "todo", "toda",
    "todos", "todas", "algun", "alguna", // English (lightweight — user queries can be bilingual)
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "am", "i", "you", "he", "she", "it", "we",
    "they", "my", "your", "his", "her", "its", "our", "their", "of", "to", "in", "on", "at", "for", "with", "as", "by",
    "from", "that", "this", "these", "those", "and", "or", "but", "not", "no", "if", "so", "do", "does", "did", "has",
    "have", "had", "will", "would", "can", "could", "should", "about", "into", "over", "up", "down", "out", "any",
    "some", "all", "more", "less", "also",
];

fn is_fts_stopword(w: &str) -> bool {
    let lower = w.to_lowercase();
    FTS_STOPWORDS.iter().any(|s| *s == lower)
}

/// Extract substrings between matching quote pairs (straight `"..."`, curly
/// `“...”`, or single `'...'`). Returns the inner text of each pair. The
/// original string is returned with those spans replaced by spaces so the
/// caller can process the remainder as loose keywords without re-matching
/// the same words.
fn extract_quoted_phrases(query: &str) -> (Vec<String>, String) {
    let mut phrases: Vec<String> = Vec::new();
    let mut remainder = String::with_capacity(query.len());
    let mut buf = String::new();
    let mut in_quote: Option<char> = None;
    let pair_of = |c: char| -> Option<char> {
        match c {
            '"' => Some('"'),
            '\'' => Some('\''),
            '“' => Some('”'),
            '‘' => Some('’'),
            _ => None,
        }
    };
    for ch in query.chars() {
        if let Some(close) = in_quote {
            if ch == close {
                if !buf.trim().is_empty() {
                    phrases.push(buf.trim().to_string());
                }
                buf.clear();
                in_quote = None;
                remainder.push(' ');
            } else {
                buf.push(ch);
            }
        } else if let Some(close) = pair_of(ch) {
            in_quote = Some(close);
        } else {
            remainder.push(ch);
        }
    }
    // Unclosed quote — salvage the partial phrase as loose content.
    if !buf.trim().is_empty() {
        remainder.push(' ');
        remainder.push_str(&buf);
    }
    (phrases, remainder)
}

/// Normalize a word for FTS matching: keep alphanumerics plus `-` and `_`,
/// drop everything else. Returns `None` if nothing substantive remains.
fn normalize_fts_word(w: &str) -> Option<String> {
    let cleaned: String = w
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Build an FTS5 query from a natural-language user question.
///
/// Strategy:
///   1. Extract quoted substrings and emit them as FTS5 phrase queries — these
///      are high-signal (user is literally quoting) so they dominate ranking.
///   2. Split the rest on whitespace, normalize to alphanumeric tokens,
///      filter out common ES/EN stopwords and words shorter than 3 chars,
///      de-duplicate while preserving order.
///   3. OR everything together so BM25 still ranks by per-term relevance.
///
/// Falls back to a 2-char / no-stopword-filter pass if the aggressive filter
/// leaves us with nothing (e.g. the user asked a very short one-word query).
fn escape_fts_query(query: &str) -> String {
    let (phrases, remainder) = extract_quoted_phrases(query);

    let mut parts: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Phrase queries first. Emit the phrase as an exact FTS5 phrase when it
    // has ≥ 2 tokens (strong signal when it matches), AND also surface each
    // content-bearing phrase token as an OR keyword so rare terms (e.g.
    // `ACME_API_KEY`) still contribute when the full phrase has no
    // verbatim hit in the corpus.
    for phrase in &phrases {
        let tokens: Vec<String> = phrase
            .split_whitespace()
            .filter_map(normalize_fts_word)
            .filter(|t| t.len() >= 2)
            .collect();
        if tokens.len() >= 2 {
            parts.push(format!("\"{}\"", tokens.join(" ")));
        } else if let Some(single) = tokens.first() {
            parts.push(format!("\"{}\"", single));
        }
        for tok in tokens {
            if tok.len() < 3 || is_fts_stopword(&tok) {
                continue;
            }
            let lower = tok.to_lowercase();
            if seen.insert(lower) {
                parts.push(format!("\"{}\"", tok));
            }
        }
    }

    for w in remainder.split_whitespace() {
        if let Some(tok) = normalize_fts_word(w) {
            if tok.len() < 3 || is_fts_stopword(&tok) {
                continue;
            }
            let lower = tok.to_lowercase();
            if seen.insert(lower) {
                parts.push(format!("\"{}\"", tok));
            }
        }
    }

    if parts.is_empty() {
        // Fallback: looser filter (original 2-char min, no stopword list) so
        // short queries like `"factura"` or `"kickoff"` still work.
        let fallback: Vec<String> = query
            .split_whitespace()
            .filter_map(normalize_fts_word)
            .filter(|w| w.len() >= 2)
            .map(|w| format!("\"{}\"", w))
            .collect();
        if fallback.is_empty() {
            return String::new();
        }
        return fallback.join(" OR ");
    }

    parts.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn insert_fts_email(
        db: &Database,
        id: &str,
        account_id: &str,
        sender_email: &str,
        subject: &str,
        body: &str,
        timestamp: i64,
    ) {
        insert_fts_email_in_mailbox(db, id, account_id, sender_email, subject, body, timestamp, "inbox");
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_fts_email_in_mailbox(
        db: &Database,
        id: &str,
        account_id: &str,
        sender_email: &str,
        subject: &str,
        body: &str,
        timestamp: i64,
        mailbox: &str,
    ) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
            params![account_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                     VALUES (?1, ?2, ?1, ?3, 'Test Sender', ?4, 'x.com', '[]', '[]', '', ?5, 0, 'primary', ?6, 0)",
            params![id, account_id, subject, sender_email, timestamp, mailbox],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO email_bodies (email_id, body) VALUES (?1, ?2)",
            params![id, body],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, ?2, 'Test Sender', ?3)",
            params![id, subject, body],
        )
        .unwrap();
    }

    // OR semantics: multi-word NL queries must return emails matching ANY token,
    // not all of them. This is the inverse of `db.search_emails()` (AND-joined,
    // used by the UI filter bar) and the reason agent-search needs fts_search.
    #[test]
    fn fts_search_or_semantics_matches_any_token() {
        let db = Database::new_for_testing().unwrap();
        insert_fts_email(&db, "e1", "acc1", "alice@x.com", "Propuesta inicial", "draft", 100);
        insert_fts_email(&db, "e2", "acc1", "bob@x.com", "Oferta de servicios", "draft", 200);
        insert_fts_email(&db, "e3", "acc1", "carol@x.com", "Newsletter weekly", "draft", 300);

        // "propuesta oferta" — no email contains BOTH; an AND join would return 0.
        let hits = db.fts_search("propuesta oferta", Some("acc1"), None, 10).unwrap();
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"e1"), "e1 (propuesta) should match, got {:?}", ids);
        assert!(ids.contains(&"e2"), "e2 (oferta) should match, got {:?}", ids);
        assert!(!ids.contains(&"e3"), "e3 must not match, got {:?}", ids);
    }

    #[test]
    fn fts_search_account_isolation() {
        let db = Database::new_for_testing().unwrap();
        insert_fts_email(&db, "e1", "acc1", "alice@x.com", "factura junio", "body", 100);
        insert_fts_email(&db, "e2", "acc2", "alice@x.com", "factura julio", "body", 200);

        let hits = db.fts_search("factura", Some("acc1"), None, 10).unwrap();
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["e1"]);
    }

    // Spanish/English stopwords like "en", "los", "que", "el" must be dropped
    // before MATCH so a sentence-shaped query doesn't ANDmark them as required.
    #[test]
    fn fts_search_strips_stopwords_from_natural_language_query() {
        let db = Database::new_for_testing().unwrap();
        insert_fts_email(&db, "e1", "acc1", "alice@x.com", "Propuesta consultoria", "draft", 100);

        let hits = db
            .fts_search("emails en los que he enviado propuesta", Some("acc1"), None, 10)
            .unwrap();
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["e1"], "stopwords must not block the match");
    }

    // Captures the regression that motivated the SQL push-down: when the user
    // wants emails THEY sent and most FTS matches in the corpus are received,
    // a top-K cut on bm25 (subject-weighted) buries the few sent emails. The
    // sender_email_eq filter pushes the constraint into SQL so the top-K
    // contains only sent rows.
    #[test]
    fn fts_search_sender_email_eq_returns_only_matching_sender() {
        let db = Database::new_for_testing().unwrap();
        let user = "me@me.com";
        // 20 received emails with short keyword-heavy subjects → high bm25.
        for i in 0..20 {
            insert_fts_email(
                &db,
                &format!("r{i}"),
                "acc1",
                &format!("client{i}@x.com"),
                "Propuesta", // dense subject
                "body",
                1_000 + i,
            );
        }
        // 3 sent emails — longer subjects (worse bm25) but still match.
        for i in 0..3 {
            insert_fts_email(
                &db,
                &format!("s{i}"),
                "acc1",
                user,
                "Re: Propuesta de consultoria detallada para cliente XYZ",
                "body",
                2_000 + i,
            );
        }

        let unfiltered = db.fts_search("propuesta", Some("acc1"), None, 10).unwrap();
        let sent_in_top10 = unfiltered.iter().filter(|(id, _)| id.starts_with('s')).count();
        // Document current behaviour: bm25 ranks the dense received subjects
        // first; the few sent rows fall off the top-10.
        assert!(
            sent_in_top10 < 3,
            "expected bm25 to bury sent emails under dense received subjects, got {} sent in top-10",
            sent_in_top10
        );

        let filtered = db
            .fts_search_filtered("propuesta", Some("acc1"), None, Some(user), 10)
            .unwrap();
        let ids: Vec<String> = filtered.iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(ids.len(), 3, "expected all 3 sent rows, got {:?}", ids);
        for id in &ids {
            assert!(id.starts_with('s'), "non-sent id leaked: {}", id);
        }
    }

    // Regression: chat RAG surfaced spam-mailbox emails because the retrieval
    // primitives only filtered by account/category. A spam email classified
    // `primary` sailed through both the FTS and the vector path.
    #[test]
    fn fts_search_excludes_spam_and_trash_mailboxes() {
        let db = Database::new_for_testing().unwrap();
        insert_fts_email_in_mailbox(
            &db,
            "e-in",
            "acc1",
            "alice@x.com",
            "factura junio",
            "body",
            100,
            "inbox",
        );
        insert_fts_email_in_mailbox(
            &db,
            "e-spam",
            "acc1",
            "bot@bad.com",
            "factura premio",
            "body",
            200,
            "spam",
        );
        insert_fts_email_in_mailbox(
            &db,
            "e-trash",
            "acc1",
            "bot@bad.com",
            "factura vieja",
            "body",
            300,
            "trash",
        );

        let hits = db.fts_search("factura", Some("acc1"), None, 10).unwrap();
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"e-in"), "inbox email must match, got {:?}", ids);
        assert!(!ids.contains(&"e-spam"), "spam email must be excluded, got {:?}", ids);
        assert!(!ids.contains(&"e-trash"), "trash email must be excluded, got {:?}", ids);
    }

    #[test]
    fn vec_search_excludes_spam_and_trash_mailboxes() {
        let db = Database::new_for_testing().unwrap();
        insert_fts_email_in_mailbox(&db, "e-in", "acc1", "alice@x.com", "subject", "body", 100, "inbox");
        insert_fts_email_in_mailbox(&db, "e-spam", "acc1", "bot@bad.com", "subject", "body", 200, "spam");
        insert_fts_email_in_mailbox(&db, "e-trash", "acc1", "bot@bad.com", "subject", "body", 300, "trash");

        // Identical embeddings so all three are equally-ranked KNN hits; only
        // the mailbox filter can tell them apart.
        let emb = vec![0.1_f32; 768];
        for id in ["e-in", "e-spam", "e-trash"] {
            db.store_embedding_chunks(id, "acc1", std::slice::from_ref(&emb), "test-model", "hash")
                .unwrap();
        }

        let hits = db.vec_search(&emb, Some("acc1"), None, 10).unwrap();
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"e-in"), "inbox email must be retrieved, got {:?}", ids);
        assert!(!ids.contains(&"e-spam"), "spam email must be excluded, got {:?}", ids);
        assert!(!ids.contains(&"e-trash"), "trash email must be excluded, got {:?}", ids);
    }

    // Regression: the embedding pipeline embedded spam/trash emails — wasted
    // compute for content that retrieval must never surface. Candidate
    // selection must skip them.
    #[test]
    fn get_emails_without_embeddings_excludes_spam_and_trash() {
        let db = Database::new_for_testing().unwrap();
        insert_fts_email_in_mailbox(&db, "e-in", "acc1", "alice@x.com", "subject", "body", 100, "inbox");
        insert_fts_email_in_mailbox(&db, "e-sent", "acc1", "me@me.com", "subject", "body", 150, "sent");
        insert_fts_email_in_mailbox(&db, "e-spam", "acc1", "bot@bad.com", "subject", "body", 200, "spam");
        insert_fts_email_in_mailbox(&db, "e-trash", "acc1", "bot@bad.com", "subject", "body", 300, "trash");

        let ids = db.get_emails_without_embeddings(Some("acc1"), 10, &[], None).unwrap();
        assert!(
            ids.contains(&"e-in".to_string()),
            "inbox email must be a candidate, got {:?}",
            ids
        );
        assert!(
            ids.contains(&"e-sent".to_string()),
            "sent email must be a candidate, got {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e-spam".to_string()),
            "spam email must be skipped, got {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e-trash".to_string()),
            "trash email must be skipped, got {:?}",
            ids
        );
    }

    // Optimistic sent copies awaiting reconciliation must not be embedded —
    // they are deleted when the provider's real copy arrives.
    #[test]
    fn get_emails_without_embeddings_excludes_pending_sent_rows() {
        let db = Database::new_for_testing().unwrap();
        insert_fts_email_in_mailbox(&db, "e-sent", "acc1", "me@me.com", "subject", "body", 100, "sent");
        insert_fts_email_in_mailbox(&db, "e-pend", "acc1", "me@me.com", "subject", "body", 150, "sent");
        db.connection()
            .execute("UPDATE emails SET pending_sync = 1 WHERE id = 'e-pend'", [])
            .unwrap();

        let ids = db.get_emails_without_embeddings(Some("acc1"), 10, &[], None).unwrap();
        assert!(ids.contains(&"e-sent".to_string()), "got {:?}", ids);
        assert!(
            !ids.contains(&"e-pend".to_string()),
            "pending rows must be excluded, got {:?}",
            ids
        );
        assert_eq!(db.count_emails_without_embeddings(Some("acc1")).unwrap(), 1);
    }

    // The pending count must mirror the fetcher's filters (mailbox + deletion)
    // or the UI reports pending work the embed loop will never pick up.
    #[test]
    fn count_emails_without_embeddings_matches_fetcher_filters() {
        let db = Database::new_for_testing().unwrap();
        insert_fts_email_in_mailbox(&db, "e-in", "acc1", "alice@x.com", "subject", "body", 100, "inbox");
        insert_fts_email_in_mailbox(&db, "e-spam", "acc1", "bot@bad.com", "subject", "body", 200, "spam");
        insert_fts_email_in_mailbox(&db, "e-trash", "acc1", "bot@bad.com", "subject", "body", 300, "trash");
        insert_fts_email_in_mailbox(&db, "e-del", "acc1", "gone@x.com", "subject", "body", 400, "inbox");
        db.connection()
            .execute("UPDATE emails SET is_deleted = 1 WHERE id = 'e-del'", [])
            .unwrap();
        // Already embedded — must not count as pending.
        insert_fts_email_in_mailbox(&db, "e-emb", "acc1", "bob@x.com", "subject", "body", 500, "inbox");
        db.store_embedding_chunks("e-emb", "acc1", &[vec![0.1_f32; 768]], "test-model", "hash")
            .unwrap();

        let count = db.count_emails_without_embeddings(Some("acc1")).unwrap();
        assert_eq!(count, 1, "only e-in is pending (spam/trash/deleted/embedded excluded)");
    }

    // Sanity: passing only stopwords should produce an empty FTS query and
    // return no hits rather than panicking or matching everything.
    #[test]
    fn fts_search_returns_empty_for_stopword_only_query() {
        let db = Database::new_for_testing().unwrap();
        insert_fts_email(&db, "e1", "acc1", "alice@x.com", "Propuesta", "body", 100);

        let hits = db.fts_search("en los que de la", Some("acc1"), None, 10).unwrap();
        assert!(
            hits.is_empty(),
            "stopword-only query must return no hits, got {:?}",
            hits
        );
    }
}
