use std::sync::Arc;

use crate::services::app_handle::AppHandle;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::ai::ollama::{OllamaClient, ParsedSearchQuery};
use crate::ai::provider::CompletionOptions;
use crate::db::Database;
use crate::models::error::Result;
use crate::models::Email;
use crate::services::ai::AiService;
use crate::services::retrieval::{fetch_fts, fetch_vector, fuse_rrf, FtsRequest, Ranking, VectorRequest};

fn emit_log(_app: &Option<AppHandle>, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

/// Email with optional relevance score and match reason
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailWithScore {
    #[serde(flatten)]
    pub email: Email,
    /// Similarity score (0.0 - 1.0) for semantic search, None for keyword search
    pub relevance_score: Option<f32>,
    /// Human-readable explanation of why this result matched
    pub match_reason: Option<String>,
    /// Timestamp of the newest email in this row's thread: the search box
    /// sorts and dates threads by their latest activity, matching or not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_latest_at: Option<i64>,
}

/// Hybrid search configuration
struct HybridConfig {
    /// Weight for vector (semantic) search (0.0 - 1.0)
    vector_weight: f32,
    /// Weight for keyword (FTS) search (0.0 - 1.0)
    text_weight: f32,
    /// Weight for accent-insensitive lexical matching
    normalized_text_weight: f32,
    /// Minimum combined score to include in results
    min_score: f32,
    /// Maximum results to return
    max_results: usize,
}

impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            vector_weight: 0.55,
            text_weight: 0.50,
            normalized_text_weight: 0.20,
            min_score: 0.001, // RRF scores are very small; any non-zero result passes
            max_results: 30,
        }
    }
}

/// Reciprocal Rank Fusion constant. Standard value from the RRF paper.
const RRF_K: f32 = 60.0;

/// Perform hybrid search combining semantic (vector) and keyword (FTS) search
/// Based on MEMORY_SYSTEM_SPEC.md hybrid search algorithm
// Reactivating semantic search is its own work: nothing lists through it today.
#[allow(dead_code)]
async fn semantic_search(
    db: &Arc<Database>,
    ollama: &OllamaClient,
    account_id: &str,
    query: &str,
    categories: Option<&[String]>,
) -> Result<(Vec<EmailWithScore>, bool)> {
    let config = HybridConfig::default();
    let query_terms = extract_query_terms(query);

    // 1. Get vector search results
    let t = std::time::Instant::now();
    let vector_scores = get_vector_scores(db, ollama, account_id, query, categories).await?;
    emit_log(
        &None,
        "debug",
        "search",
        &format!(
            "[timing] vector_search: {:.0}ms ({} results)",
            t.elapsed().as_secs_f64() * 1000.0,
            vector_scores.len(),
        ),
    );

    // 2. Get FTS search results (indexed — always fast)
    let t = std::time::Instant::now();
    let fts_scores = get_fts_scores(db, account_id, query, categories)?;
    emit_log(
        &None,
        "debug",
        "search",
        &format!(
            "[timing] fts_search: {:.0}ms ({} results)",
            t.elapsed().as_secs_f64() * 1000.0,
            fts_scores.len(),
        ),
    );

    // If no embeddings and no FTS results, fall back to basic keyword search.
    // Skip the normalized (full-table) scorer in this case — it would load all
    // emails into memory for no benefit since we already have no candidates.
    if vector_scores.is_empty() && fts_scores.is_empty() {
        let t = std::time::Instant::now();
        let results = db.search_emails(account_id, query, categories, None, None, None, None, None, None, 100)?;
        emit_log(
            &None,
            "debug",
            "search",
            &format!(
                "[timing] db.search_emails (empty-semantic fallback): {:.0}ms ({} results)",
                t.elapsed().as_secs_f64() * 1000.0,
                results.len(),
            ),
        );
        return Ok((emails_to_scored(results, Some("Keyword match")), false));
    }

    // 3. Normalized accent-insensitive scoring — only over the candidate set found
    //    by vector + FTS above. We pass the IDs explicitly so `get_normalized_scores`
    //    only loads those records, not all 35k emails.
    let candidate_ids: Vec<String> = vector_scores
        .iter()
        .map(|(id, _)| id.clone())
        .chain(fts_scores.iter().map(|(id, _)| id.clone()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let t = std::time::Instant::now();
    let normalized_scores = get_normalized_scores_for_ids(db, query, &candidate_ids)?;
    emit_log(
        &None,
        "debug",
        "search",
        &format!(
            "[timing] normalized_scoring: {:.0}ms ({} candidates, {} scored)",
            t.elapsed().as_secs_f64() * 1000.0,
            candidate_ids.len(),
            normalized_scores.len(),
        ),
    );

    // 3. Reciprocal Rank Fusion via `services::retrieval::fuse_rrf`.
    //    The fuse_rrf helper only returns fused (id, score) — we still need
    //    the per-ranker has_vec/has_fts/has_normalized flags for the human-
    //    readable match reason, so we derive those from the input id sets.
    let mut vec_sorted: Vec<&(String, f32)> = vector_scores.iter().collect();
    vec_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut fts_sorted: Vec<&(String, f64)> = fts_scores.iter().collect();
    fts_sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut norm_sorted: Vec<&(String, f32)> = normalized_scores.iter().collect();
    norm_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let vec_ids: Vec<String> = vec_sorted.iter().map(|(id, _)| id.clone()).collect();
    let fts_ids: Vec<String> = fts_sorted.iter().map(|(id, _)| id.clone()).collect();
    let norm_ids: Vec<String> = norm_sorted.iter().map(|(id, _)| id.clone()).collect();

    let vec_set: std::collections::HashSet<&String> = vec_ids.iter().collect();
    let fts_set: std::collections::HashSet<&String> = fts_ids.iter().collect();
    let norm_set: std::collections::HashSet<&String> = norm_ids.iter().collect();

    let fused = fuse_rrf(
        &[
            Ranking {
                ids_in_order: &vec_ids,
                weight: config.vector_weight,
            },
            Ranking {
                ids_in_order: &fts_ids,
                weight: config.text_weight,
            },
            Ranking {
                ids_in_order: &norm_ids,
                weight: config.normalized_text_weight,
            },
        ],
        RRF_K,
    );
    let combined_scores: std::collections::HashMap<String, (f32, bool, bool, bool)> = fused
        .into_iter()
        .map(|(id, score)| {
            let has_vec = vec_set.contains(&id);
            let has_fts = fts_set.contains(&id);
            let has_norm = norm_set.contains(&id);
            (id, (score, has_vec, has_fts, has_norm))
        })
        .collect();

    // 4. Sort by combined score and filter
    let mut scored_list: Vec<(String, f32, bool, bool, bool)> = combined_scores
        .into_iter()
        .map(|(id, (score, has_vec, has_fts, has_normalized))| (id, score, has_vec, has_fts, has_normalized))
        .filter(|(_, score, _, _, _)| *score >= config.min_score)
        .collect();

    scored_list.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored_list.truncate(config.max_results * 3);

    if scored_list.is_empty() {
        // No good matches, fall back to basic keyword search
        let t = std::time::Instant::now();
        let results = db.search_emails(account_id, query, categories, None, None, None, None, None, None, 100)?;
        emit_log(
            &None,
            "debug",
            "search",
            &format!(
                "[timing] db.search_emails (no-score fallback): {:.0}ms ({} results)",
                t.elapsed().as_secs_f64() * 1000.0,
                results.len(),
            ),
        );
        return Ok((emails_to_scored(results, Some("Keyword match")), false));
    }

    // 5. Fetch full email records
    let email_ids: Vec<String> = scored_list.iter().map(|(id, _, _, _, _)| id.clone()).collect();
    let t = std::time::Instant::now();
    let emails = db.get_emails_by_ids(&email_ids)?;
    emit_log(
        &None,
        "debug",
        "search",
        &format!(
            "[timing] get_emails_by_ids: {:.0}ms ({} fetched)",
            t.elapsed().as_secs_f64() * 1000.0,
            emails.len(),
        ),
    );

    // Create lookup map for scores
    let score_map: std::collections::HashMap<String, (f32, bool, bool, bool)> = scored_list
        .into_iter()
        .map(|(id, score, has_vec, has_fts, has_normalized)| (id, (score, has_vec, has_fts, has_normalized)))
        .collect();

    // 6. Build results with match reasons
    let mut emails_with_scores: Vec<EmailWithScore> = emails
        .into_iter()
        .filter_map(|email| {
            let (score, has_vec, has_fts, has_normalized) = score_map.get(&email.id)?;
            let lexical_overlap = term_overlap_score(&email, &query_terms);
            let adjusted_score = (*score + lexical_overlap).min(1.0);
            let match_reason =
                build_hybrid_match_reason(adjusted_score, *has_vec, *has_fts, *has_normalized, lexical_overlap);

            Some(EmailWithScore {
                email,
                relevance_score: Some(adjusted_score),
                match_reason: Some(match_reason),
                thread_latest_at: None,
            })
        })
        .collect();

    // Sort by score (db.get_emails_by_ids may not preserve order)
    emails_with_scores.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    emails_with_scores = dedupe_scored_results_by_thread(emails_with_scores);
    emails_with_scores.truncate(config.max_results);

    Ok((emails_with_scores, true))
}

/// Get vector similarity scores using sqlite-vec KNN search
async fn get_vector_scores(
    db: &Arc<Database>,
    ollama: &OllamaClient,
    account_id: &str,
    query: &str,
    categories: Option<&[String]>,
) -> Result<Vec<(String, f32)>> {
    // Generate embedding for the search query (Ollama HTTP call)
    let t = std::time::Instant::now();
    let query_embedding = match ollama.generate_embedding(query).await {
        Ok(emb) => emb,
        Err(e) => {
            emit_log(
                &None,
                "debug",
                "search",
                &format!(
                    "[timing] ollama.generate_embedding: {:.0}ms (FAILED: {})",
                    t.elapsed().as_secs_f64() * 1000.0,
                    e,
                ),
            );
            return Ok(Vec::new());
        }
    };
    emit_log(
        &None,
        "debug",
        "search",
        &format!(
            "[timing] ollama.generate_embedding: {:.0}ms (dim={})",
            t.elapsed().as_secs_f64() * 1000.0,
            query_embedding.len(),
        ),
    );

    // KNN search via sqlite-vec (returns deduplicated email_id, similarity pairs)
    let t = std::time::Instant::now();
    let result = fetch_vector(
        db,
        VectorRequest {
            account_id,
            embedding: &query_embedding,
            categories,
            limit: 150,
        },
    );
    emit_log(
        &None,
        "debug",
        "search",
        &format!(
            "[timing] vec_search (KNN): {:.0}ms ({} hits)",
            t.elapsed().as_secs_f64() * 1000.0,
            result.as_ref().map(|v| v.len()).unwrap_or(0),
        ),
    );
    result
}

/// Get FTS search scores
fn get_fts_scores(
    db: &Arc<Database>,
    account_id: &str,
    query: &str,
    categories: Option<&[String]>,
) -> Result<Vec<(String, f64)>> {
    fetch_fts(
        db,
        FtsRequest {
            account_id,
            query,
            categories,
            sender_email_eq: None,
            limit: 50,
        },
    )
}

/// Build human-readable match reason for hybrid search
fn build_hybrid_match_reason(
    score: f32,
    has_vector: bool,
    has_fts: bool,
    has_normalized: bool,
    lexical_overlap: f32,
) -> String {
    let percentage = (score * 100.0).round() as i32;
    let mut parts = Vec::new();
    if has_vector {
        parts.push("semantic");
    }
    if has_fts {
        parts.push("keyword");
    }
    if has_normalized {
        parts.push("normalized");
    }
    if lexical_overlap > 0.0 {
        parts.push("lexical boost");
    }

    if parts.is_empty() {
        format!("{}% match", percentage)
    } else {
        format!("{}% match ({})", percentage, parts.join(" + "))
    }
}

/// Convert plain emails to EmailWithScore with optional match reason
fn emails_to_scored(emails: Vec<Email>, match_reason: Option<&str>) -> Vec<EmailWithScore> {
    emails
        .into_iter()
        .map(|email| EmailWithScore {
            email,
            relevance_score: None,
            match_reason: match_reason.map(|s| s.to_string()),
            thread_latest_at: None,
        })
        .collect()
}

fn dedupe_scored_results_by_thread(results: Vec<EmailWithScore>) -> Vec<EmailWithScore> {
    let mut seen_threads = std::collections::HashSet::new();
    let mut deduped = Vec::with_capacity(results.len());

    for result in results {
        if seen_threads.insert(result.email.thread_id.clone()) {
            deduped.push(result);
        }
    }

    deduped
}

fn normalized_keyword_score(email: &Email, query_terms: &[String]) -> f32 {
    let haystack = normalize_for_search(&format!(
        "{} {} {} {} {}",
        email.subject, email.sender, email.sender_email, email.snippet, email.body
    ));
    let subject = normalize_for_search(&email.subject);
    let sender = normalize_for_search(&format!("{} {}", email.sender, email.sender_email));

    let mut score = 0.0;
    for term in query_terms {
        if subject.contains(term) {
            score += 0.5;
        } else if sender.contains(term) {
            score += 0.35;
        } else if haystack.contains(term) {
            score += 0.2;
        }
    }

    score
}

/// Score only the specified email IDs — used in the hybrid search hot path so we
/// never load the full mailbox into memory just to score a small candidate set.
fn get_normalized_scores_for_ids(db: &Arc<Database>, query: &str, email_ids: &[String]) -> Result<Vec<(String, f32)>> {
    if email_ids.is_empty() {
        return Ok(Vec::new());
    }
    let query_terms = extract_query_terms(query);
    if query_terms.is_empty() {
        return Ok(Vec::new());
    }
    let emails = db.get_emails_by_ids(email_ids)?;
    Ok(emails
        .into_iter()
        .filter_map(|email| {
            let score = normalized_keyword_score(&email, &query_terms);
            if score > 0.0 {
                Some((email.id, score.min(1.0)))
            } else {
                None
            }
        })
        .collect())
}

pub(crate) fn matches_parsed_filters(email: &Email, parsed: &ParsedSearchQuery) -> bool {
    if let Some(ref from) = parsed.from_filter {
        let needle = normalize_for_search(from);
        let haystack = normalize_for_search(&format!("{} {}", email.sender, email.sender_email));
        if !haystack.contains(&needle) {
            return false;
        }
    }

    if let Some(ref to) = parsed.to_filter {
        let needle = normalize_for_search(to);
        let recipients = email
            .recipients
            .iter()
            .map(|recipient| normalize_for_search(recipient))
            .collect::<Vec<_>>()
            .join(" ");
        if !recipients.contains(&needle) {
            return false;
        }
    }

    if let Some(ref subject) = parsed.subject_filter {
        let needle = normalize_for_search(subject);
        if !normalize_for_search(&email.subject).contains(&needle) {
            return false;
        }
    }

    if parsed.is_unread == Some(true) && email.is_read {
        return false;
    }

    if let Some(after) = parsed.after_timestamp {
        if email.timestamp < after {
            return false;
        }
    }

    if let Some(before) = parsed.before_timestamp {
        if email.timestamp > before {
            return false;
        }
    }

    true
}

fn normalize_for_search(text: &str) -> String {
    text.nfd()
        .filter(|character| !matches!(character, '\u{0300}'..='\u{036f}'))
        .collect::<String>()
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() || character == '-' {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_query_terms(query: &str) -> Vec<String> {
    normalize_for_search(query)
        .split_whitespace()
        .filter(|term| term.len() >= 3 && !is_search_stopword(term))
        .map(str::to_string)
        .collect()
}

fn is_search_stopword(term: &str) -> bool {
    matches!(
        term,
        "about"
            | "acerca"
            | "antes"
            | "con"
            | "correos"
            | "de"
            | "del"
            | "despues"
            | "el"
            | "emails"
            | "esta"
            | "este"
            | "from"
            | "hoy"
            | "la"
            | "las"
            | "los"
            | "mail"
            | "mails"
            | "mensajes"
            | "para"
            | "sobre"
            | "the"
            | "this"
            | "unread"
            | "week"
    )
}

fn term_overlap_score(email: &Email, query_terms: &[String]) -> f32 {
    if query_terms.is_empty() {
        return 0.0;
    }

    let text = normalize_for_search(&format!(
        "{} {} {} {} {}",
        email.subject, email.sender, email.sender_email, email.snippet, email.body
    ));
    let subject = normalize_for_search(&email.subject);

    let mut matches = 0_u32;
    let mut subject_matches = 0_u32;
    for term in query_terms {
        if text.contains(term) {
            matches += 1;
        }
        if subject.contains(term) {
            subject_matches += 1;
        }
    }

    (matches.min(4) as f32 * 0.03) + (subject_matches.min(2) as f32 * 0.04)
}

// Kept with `semantic_search`, for the same reason.
#[allow(dead_code)]
async fn parse_search_query_with_provider(ai_service: &AiService, query: &str) -> Result<ParsedSearchQuery> {
    let prompt = format!(
        r#"You are a search query parser for an email application.
Parse the following query and return ONLY a JSON object.

Query: "{}"

Required JSON fields:
- keywords (array of strings)
- from_filter (string|null)
- to_filter (string|null)
- subject_filter (string|null)
- has_attachment (boolean|null)
- is_unread (boolean|null)
- after_timestamp ("YYYY-MM-DD"|null)
- before_timestamp ("YYYY-MM-DD"|null)

Return JSON only, no markdown."#,
        query
    );

    let response = ai_service
        .complete(
            &prompt,
            "search_parse_query",
            Some(CompletionOptions {
                temperature: Some(0.0),
                max_tokens: Some(300),
                think: None,
                json_shape: None,
            }),
        )
        .await?;

    let json_payload = extract_json_payload(&response);
    serde_json::from_str(&json_payload).map_err(|e| {
        crate::models::error::AppError::AiError(format!(
            "Failed to parse provider search query JSON: {}. Response: {}",
            e, response
        ))
    })
}

fn extract_json_payload(text: &str) -> String {
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if end >= start {
            return text[start..=end].to_string();
        }
    }
    text.to_string()
}

// -- Legacy model management (delegates to Ollama) --

pub async fn list_ollama_models() -> Result<Vec<String>> {
    let ollama = OllamaClient::new(None);
    ollama.list_model_names().await
}

pub async fn get_ai_model(db: &Database) -> Result<String> {
    if let Some(model) = db.get_preference("ai_model")? {
        return Ok(model);
    }
    const PREFERRED: &[&str] = &["gemma4:e2b", "gemma4", "gemma3:4b", "gemma3"];
    let ollama = OllamaClient::new(None);
    if let Ok(models) = ollama.list_model_names().await {
        for preferred in PREFERRED {
            if let Some(m) = models.iter().find(|m: &&String| m.starts_with(preferred)) {
                let _ = db.set_preference("ai_model", m);
                return Ok(m.clone());
            }
        }
        if let Some(first) = models.first() {
            let _ = db.set_preference("ai_model", first);
            return Ok(first.clone());
        }
    }
    Ok(String::new())
}

pub fn set_ai_model(db: &Database, model: &str) -> Result<()> {
    db.set_preference("ai_model", model)
}

#[cfg(test)]
mod reply_parsing_tests {
    use super::*;

    #[test]
    fn a_closing_brace_before_the_opening_one_is_not_a_json_span() {
        assert_eq!(extract_json_payload("} not json {"), "} not json {");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    // ── The search box lists what the smart filter lists ─────────────────

    fn open_prod_db() -> Option<(Arc<Database>, String)> {
        let db_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.emailops.app")
            .join("emailops.db");

        if !db_path.exists() {
            eprintln!("Production DB not found at {:?}, skipping", db_path);
            return None;
        }

        eprintln!("DB path: {:?}", db_path);
        eprintln!(
            "DB size: {:.1} MB",
            std::fs::metadata(&db_path).unwrap().len() as f64 / 1_000_000.0
        );

        let db = Arc::new(Database::open_readonly(db_path).expect("Failed to open production DB"));

        let (account_id, email_count): (String, i64) = db
            .reader()
            .query_row(
                "SELECT account_id, COUNT(*) as cnt FROM emails GROUP BY account_id ORDER BY cnt DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        eprintln!("Account: {} ({} emails)\n", account_id, email_count);

        Some((db, account_id))
    }

    /// Deep-dive diagnostic for vec_search performance on the production DB.
    /// Run with: cargo test -p emailops report_vec_search -- --nocapture --ignored
    #[tokio::test]
    #[ignore]
    async fn report_vec_search() {
        let (db, account_id) = match open_prod_db() {
            Some(v) => v,
            None => return,
        };

        eprintln!("\n╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║              vec_search Deep-Dive Report                    ║");
        eprintln!("╚══════════════════════════════════════════════════════════════╝");

        // ── 1. Table stats ────────────────────────────────────────────────────
        let vec_count: i64 = db
            .reader()
            .query_row("SELECT COUNT(*) FROM vec_emails", [], |r| r.get(0))
            .unwrap_or(-1);
        let chunks_count: i64 = db
            .reader()
            .query_row("SELECT COUNT(*) FROM embedding_chunks", [], |r| r.get(0))
            .unwrap_or(-1);
        let emails_with_embeddings: i64 = db
            .reader()
            .query_row("SELECT COUNT(DISTINCT email_id) FROM embedding_chunks", [], |r| {
                r.get(0)
            })
            .unwrap_or(-1);
        let vec_sql: String = db
            .reader()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='vec_emails'",
                [],
                |r| r.get(0),
            )
            .unwrap_or_default();

        eprintln!("\n━━━ 1. Table stats ━━━");
        eprintln!("  vec_emails rows       : {}", vec_count);
        eprintln!("  embedding_chunks rows : {}", chunks_count);
        eprintln!("  emails with embeddings: {}", emails_with_embeddings);
        eprintln!("  vec_emails DDL        : {}", vec_sql.replace('\n', " "));
        let chunks_per_email = if emails_with_embeddings > 0 {
            chunks_count as f64 / emails_with_embeddings as f64
        } else {
            0.0
        };
        eprintln!("  avg chunks per email  : {:.2}", chunks_per_email);

        // ── 2. Generate a real query embedding ────────────────────────────────
        let model = db.get_preference("ai_model").unwrap_or(None).unwrap_or_default();
        let ollama = OllamaClient::new(Some(&model));
        let t = std::time::Instant::now();
        let query_emb = match ollama.generate_embedding("facturas aws").await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Could not get embedding: {} — skipping rest", e);
                return;
            }
        };
        eprintln!(
            "\n  query embedding generated: {:.0}ms (dim={})",
            t.elapsed().as_secs_f64() * 1000.0,
            query_emb.len(),
        );

        let blob: Vec<u8> = query_emb.iter().flat_map(|f| f.to_le_bytes()).collect();

        // ── 3. Pure KNN query: ORDER BY distance LIMIT k (current form) ──────
        eprintln!("\n━━━ 2. KNN benchmark: `ORDER BY distance LIMIT k` ━━━");
        for k in [10, 50, 150, 500, 1000] {
            // warm-up
            let _ = db
                .reader()
                .prepare("SELECT rowid, distance FROM vec_emails WHERE embedding MATCH ?1 ORDER BY distance LIMIT ?2");
            let t = std::time::Instant::now();
            let conn = db.reader();
            let mut stmt = conn
                .prepare("SELECT rowid, distance FROM vec_emails WHERE embedding MATCH ?1 ORDER BY distance LIMIT ?2")
                .unwrap();
            let rows: Vec<(i64, f32)> = stmt
                .query_map(rusqlite::params![blob, k as i64], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            eprintln!(
                "  k={:<5} → {:5}ms ({} results)",
                k,
                t.elapsed().as_secs_f64() * 1000.0,
                rows.len(),
            );
        }

        // ── 4. Canonical sqlite-vec KNN: `k = ?` form ────────────────────────
        eprintln!("\n━━━ 3. KNN benchmark: `k = ?` (canonical sqlite-vec form) ━━━");
        for k in [10, 50, 150, 500, 1000] {
            let t = std::time::Instant::now();
            let conn = db.reader();
            let mut stmt = conn
                .prepare("SELECT rowid, distance FROM vec_emails WHERE embedding MATCH ?1 AND k = ?2")
                .unwrap();
            let rows: Vec<(i64, f32)> = stmt
                .query_map(rusqlite::params![blob, k as i64], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            eprintln!(
                "  k={:<5} → {:5}ms ({} results)",
                k,
                t.elapsed().as_secs_f64() * 1000.0,
                rows.len(),
            );
        }

        // ── 5. Use write_conn vs reader() — does pool contention matter? ─────
        eprintln!("\n━━━ 4. Connection type: reader() vs write_conn ━━━");
        let t = std::time::Instant::now();
        let conn = db.reader();
        let mut stmt = conn
            .prepare("SELECT rowid, distance FROM vec_emails WHERE embedding MATCH ?1 AND k = 150")
            .unwrap();
        let _rows: Vec<(i64, f32)> = stmt
            .query_map(rusqlite::params![blob], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        eprintln!("  reader() k=150     : {:.0}ms", t.elapsed().as_secs_f64() * 1000.0);

        // ── 5. Full db.vec_search (current impl) ────────────────────────────
        eprintln!("\n━━━ 5. Full db.vec_search (current impl) ━━━");
        for run in 1..=3 {
            let t = std::time::Instant::now();
            let hits = db
                .vec_search(&query_emb, Some(&account_id), None, 150)
                .unwrap_or_default();
            eprintln!(
                "  Run {}: {:.0}ms ({} hits)",
                run,
                t.elapsed().as_secs_f64() * 1000.0,
                hits.len(),
            );
        }

        // ── 5b. vec_search step-by-step to find the 15s ──────────────────────
        eprintln!("\n━━━ 5b. vec_search step-by-step ━━━");
        let limit = 150usize;
        let expanded = (limit * 5) as i64; // 750

        // Step 1a: KNN via reader()
        let t = std::time::Instant::now();
        let knn_rows: Vec<(i64, f32)> = {
            let conn = db.reader();
            let mut stmt = conn
                .prepare("SELECT rowid, distance FROM vec_emails WHERE embedding MATCH ?1 ORDER BY distance LIMIT ?2")
                .unwrap();
            stmt.query_map(rusqlite::params![blob, expanded], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        eprintln!(
            "  Step 1 (KNN via reader, LIMIT {}): {:.0}ms ({} rowids)",
            expanded,
            t.elapsed().as_secs_f64() * 1000.0,
            knn_rows.len(),
        );

        // Step 1b: KNN via write_conn (what vec_search actually uses)
        let t = std::time::Instant::now();
        let _knn_wc: Vec<(i64, f32)> = {
            let conn = db.connection();
            let mut stmt = conn
                .prepare("SELECT rowid, distance FROM vec_emails WHERE embedding MATCH ?1 ORDER BY distance LIMIT ?2")
                .unwrap();
            stmt.query_map(rusqlite::params![blob, expanded], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        eprintln!(
            "  Step 1 (KNN via write_conn — what vec_search uses): {:.0}ms",
            t.elapsed().as_secs_f64() * 1000.0,
        );

        // Step 2: rowid → email_id JOIN with emails and account filter (as vec_search does)
        let rowids: Vec<i64> = knn_rows.iter().map(|(r, _)| *r).collect();
        if !rowids.is_empty() {
            let placeholders: String = (0..rowids.len())
                .map(|i| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(", ");

            // Variant A: the current vec_search SQL (WRITE conn, JOIN, account filter)
            let t = std::time::Instant::now();
            let _ = {
                let conn = db.connection();
                let mut sql = format!(
                    "SELECT ec.rowid, ec.email_id FROM embedding_chunks ec
                     JOIN emails e ON ec.email_id = e.id
                     WHERE ec.rowid IN ({})",
                    placeholders
                );
                sql.push_str(&format!(" AND e.account_id = ?{}", rowids.len() + 1));
                let mut stmt = conn.prepare(&sql).unwrap();
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = rowids
                    .iter()
                    .map(|r| Box::new(*r) as Box<dyn rusqlite::ToSql>)
                    .collect();
                params.push(Box::new(account_id.clone()));
                let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                let rows: Vec<(i64, String)> = stmt
                    .query_map(params_refs.as_slice(), |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .filter_map(|r| r.ok())
                    .collect();
                rows.len()
            };
            eprintln!(
                "  Step 2a (JOIN emails + account filter, write_conn): {:.0}ms",
                t.elapsed().as_secs_f64() * 1000.0,
            );

            // Variant B: same but via reader()
            let t = std::time::Instant::now();
            let _ = {
                let conn = db.reader();
                let mut sql = format!(
                    "SELECT ec.rowid, ec.email_id FROM embedding_chunks ec
                     JOIN emails e ON ec.email_id = e.id
                     WHERE ec.rowid IN ({})",
                    placeholders
                );
                sql.push_str(&format!(" AND e.account_id = ?{}", rowids.len() + 1));
                let mut stmt = conn.prepare(&sql).unwrap();
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = rowids
                    .iter()
                    .map(|r| Box::new(*r) as Box<dyn rusqlite::ToSql>)
                    .collect();
                params.push(Box::new(account_id.clone()));
                let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                let rows: Vec<(i64, String)> = stmt
                    .query_map(params_refs.as_slice(), |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .filter_map(|r| r.ok())
                    .collect();
                rows.len()
            };
            eprintln!(
                "  Step 2b (same, via reader): {:.0}ms",
                t.elapsed().as_secs_f64() * 1000.0,
            );

            // Variant C: no JOIN, just embedding_chunks
            let t = std::time::Instant::now();
            let _ = {
                let conn = db.reader();
                let sql = format!(
                    "SELECT ec.rowid, ec.email_id FROM embedding_chunks ec WHERE ec.rowid IN ({})",
                    placeholders
                );
                let mut stmt = conn.prepare(&sql).unwrap();
                let params: Vec<Box<dyn rusqlite::ToSql>> = rowids
                    .iter()
                    .map(|r| Box::new(*r) as Box<dyn rusqlite::ToSql>)
                    .collect();
                let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                let rows: Vec<(i64, String)> = stmt
                    .query_map(params_refs.as_slice(), |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .filter_map(|r| r.ok())
                    .collect();
                rows.len()
            };
            eprintln!(
                "  Step 2c (no JOIN, just ec lookup): {:.0}ms",
                t.elapsed().as_secs_f64() * 1000.0,
            );
        }

        // ── 7. Analysis ──────────────────────────────────────────────────────
        eprintln!("\n━━━ 6. Analysis ━━━");
        let ddl_has_ann = vec_sql.contains("diskann") || vec_sql.contains("hnsw");
        eprintln!(
            "  ANN index present   : {} ({})",
            ddl_has_ann,
            if ddl_has_ann { "good" } else { "NO — brute-force KNN" }
        );
        eprintln!("  Rows scanned per query: {} (every vec_emails row)", vec_count);
        eprintln!(
            "  Floats compared     : ~{} per query (rows × 768 dims)",
            vec_count * 768,
        );
        eprintln!();
        eprintln!("  Brute-force cosine on ~50k × 768 floats is inherently 5-15 s");
        eprintln!("  on modern CPUs. The fix requires EITHER:");
        eprintln!("    a) Declaring an ANN index (diskann/hnsw) when creating vec0 — requires sqlite-vec 0.1.6+");
        eprintln!("    b) int8 / binary quantization for 4-16x speedup");
        eprintln!("    c) Partitioning by account_id (vec0 metadata columns)");
        eprintln!("    d) Skipping vec_search when FTS already has strong matches");

        eprintln!("\n══════════════════════════════════════════════════════════════\n");
    }

    /// Benchmark smart filter operations on the production DB.
    ///
    /// Run with: cargo test -p emailops bench_smart_filters -- --nocapture --ignored
    #[test]
    #[ignore]
    fn bench_smart_filters() {
        let (db, account_id) = match open_prod_db() {
            Some(v) => v,
            None => return,
        };

        eprintln!("=== Smart filter benchmarks ===\n");

        // 1. get_quick_filter_stats (domain + sender GROUP BY)
        let t = std::time::Instant::now();
        let stats = db
            .get_quick_filter_stats(crate::db::AccountScope::Account(&account_id), &[], &[])
            .unwrap();
        eprintln!(
            "[{:.0}ms] get_quick_filter_stats: {} domains, {} senders",
            t.elapsed().as_secs_f64() * 1000.0,
            stats.top_domains.len(),
            stats.top_senders.len(),
        );

        // 2. get_tag_stats (for each tag type)
        for tag_type in ["intent", "topic", "priority"] {
            let t = std::time::Instant::now();
            let tag_stats = db
                .get_tag_stats(crate::db::AccountScope::Account(&account_id), tag_type, 15)
                .unwrap();
            eprintln!(
                "[{:.0}ms] get_tag_stats({:?}): {} values",
                t.elapsed().as_secs_f64() * 1000.0,
                tag_type,
                tag_stats.len(),
            );
        }

        // 3. refresh_filter_stats — skipped for read-only DB (it writes suggestions)

        // 6. get_filtered_emails — by tag
        let t = std::time::Instant::now();
        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(&account_id),
                "intent",
                "informational",
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();
        eprintln!(
            "[{:.0}ms] get_filtered_emails(tag=intent:informational): {} emails, total={}",
            t.elapsed().as_secs_f64() * 1000.0,
            result.emails.len(),
            result.total_count,
        );

        // 7. get_filtered_emails — broad tags, one account and every account.
        // The sidebar's widest filters: every classified email carries a priority.
        for (label, scope) in [
            ("account", crate::db::AccountScope::Account(&account_id)),
            ("all", crate::db::AccountScope::AllEnabled),
        ] {
            for value in ["urgent", "normal", "low"] {
                let t = std::time::Instant::now();
                let result = db
                    .get_filtered_emails(scope, "priority", value, &crate::models::EmailWindow::default(), 50, 0)
                    .unwrap();
                eprintln!(
                    "[{:.0}ms] get_filtered_emails(tag=priority:{value}, scope={label}): {} emails",
                    t.elapsed().as_secs_f64() * 1000.0,
                    result.emails.len(),
                );
            }
        }

        eprintln!("\n=== Done ===\n");
    }
}
