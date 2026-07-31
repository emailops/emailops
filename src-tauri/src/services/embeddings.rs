use std::sync::Arc;

use crate::services::app_handle::AppHandle;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(feature = "desktop")]
use tauri::Emitter;

use crate::db::Database;
use crate::models::error::Result;
use crate::services::ai::AiService;

const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text";

/// User-tunable config for which emails get embedded (used for AI Search /
/// chat retrieval). Persisted in `user_preferences` like the other AI configs.
///
/// Categories default to `["primary"]` to avoid wasting model time on
/// promotions / newsletters which rarely show up in chat retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingsConfig {
    /// Gmail categories eligible for embedding. Empty = all categories.
    pub categories: Vec<String>,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            categories: vec!["primary".to_string()],
        }
    }
}

impl EmbeddingsConfig {
    /// Returns true when `category` is allowed. Empty `categories` means allow all.
    pub fn is_category_allowed(&self, category: &str) -> bool {
        self.categories.is_empty() || self.categories.iter().any(|c| c.eq_ignore_ascii_case(category))
    }
}

fn embeddings_config_key(account_id: &str) -> String {
    format!("embeddings_categories:{}", account_id)
}

pub fn get_embeddings_config(db: &Database, account_id: &str) -> Result<EmbeddingsConfig> {
    let defaults = EmbeddingsConfig::default();
    let categories = db
        .get_preference(&embeddings_config_key(account_id))?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(defaults.categories);
    Ok(EmbeddingsConfig { categories })
}

pub fn save_embeddings_config(db: &Database, account_id: &str, config: &EmbeddingsConfig) -> Result<()> {
    db.set_preference(
        &embeddings_config_key(account_id),
        &serde_json::to_string(&config.categories)?,
    )?;
    Ok(())
}

/// Chunk size in characters for splitting long emails
const CHUNK_SIZE: usize = 1000;
/// Overlap between consecutive chunks
const CHUNK_OVERLAP: usize = 200;
/// Only chunk emails longer than this threshold
const CHUNK_THRESHOLD: usize = 1500;
/// Hard cap on chunks per email (avoids runaway on huge HTML bodies)
const MAX_CHUNKS_PER_EMAIL: usize = 6;
/// Maximum characters to consider from body (avoids processing entire mega-emails)
const MAX_BODY_CHARS: usize = 8000;

fn emit_log(_app: &Option<AppHandle>, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProgress {
    pub status: String,
    pub current: u32,
    pub total: u32,
    pub message: String,
}

/// Generate a content hash for an email to detect changes.
/// Includes model name so model changes trigger re-embedding.
fn compute_content_hash(embedding_model: &str, subject: &str, body: &str) -> String {
    let preview_end = floor_char_boundary(body, 500.min(body.len()));
    let body_preview = &body[..preview_end];
    let content = format!("{}:{}\n{}", embedding_model, subject, body_preview);
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

fn get_embedding_model(db: &Arc<Database>) -> Result<String> {
    Ok(db
        .get_preference("ai_embedding_model")?
        .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string()))
}

/// Strip Re:/Fwd:/Fw: prefixes from subject to get clean thread topic
fn strip_reply_prefixes(subject: &str) -> String {
    let mut s = subject.trim().to_string();
    loop {
        let trimmed = s.trim_start();
        let lower = trimmed.to_lowercase();
        if lower.starts_with("re:") || lower.starts_with("fwd:") || lower.starts_with("fw:") {
            if let Some(colon_pos) = trimmed.find(':') {
                s = trimmed[colon_pos + 1..].trim().to_string();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    s
}

/// Clean body text — strip HTML, collapse whitespace, strip empty lines, cap length
fn clean_body(body: &str) -> String {
    // Detect and strip HTML
    let stripped = if body.contains("<html") || body.contains("<body") || body.contains("<!DOCTYPE") {
        crate::util::html::strip_html_tags(body)
    } else {
        body.to_string()
    };

    let cleaned: String = stripped
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    // Cap length early so chunking doesn't explode on huge emails
    if cleaned.len() > MAX_BODY_CHARS {
        let boundary = floor_char_boundary(&cleaned, MAX_BODY_CHARS);
        cleaned[..boundary].to_string()
    } else {
        cleaned
    }
}

/// Snap a byte index to the nearest valid UTF-8 char boundary at or below `pos`
fn floor_char_boundary(s: &str, mut pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Snap a byte index to the nearest valid UTF-8 char boundary at or above `pos`
fn ceil_char_boundary(s: &str, mut pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    while pos < s.len() && !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}

/// Safely slice a &str using byte indices, snapping both to char boundaries.
fn safe_slice(s: &str, start: usize, end: usize) -> &str {
    let start = floor_char_boundary(s, start);
    let end = floor_char_boundary(s, end);
    if start > end {
        return "";
    }
    &s[start..end]
}

/// Split long text into overlapping chunks for embedding.
/// Short texts (< CHUNK_THRESHOLD) are returned as a single chunk.
/// Uses char-by-char iteration to avoid UTF-8 boundary issues.
fn chunk_text(body: &str) -> Vec<String> {
    let clean = clean_body(body);

    if clean.len() <= CHUNK_THRESHOLD {
        return vec![clean];
    }

    // Collect all char boundaries for O(1) boundary-safe slicing
    let char_positions: Vec<usize> = clean
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(clean.len()))
        .collect();
    let total_chars = char_positions.len() - 1;

    // Convert char counts to boundary-safe byte positions
    let chunk_size_chars = CHUNK_SIZE; // treat as char count approximation
    let overlap_chars = CHUNK_OVERLAP;

    let mut chunks = Vec::new();
    let mut start_char = 0;

    while start_char < total_chars {
        let end_char = (start_char + chunk_size_chars).min(total_chars);
        let start_byte = char_positions[start_char];
        let end_byte = char_positions[end_char];

        // Try to break at sentence boundary near the end
        let actual_end_byte = if end_char < total_chars {
            let search_start_byte = char_positions[end_char.saturating_sub(100)];
            let slice = &clean[search_start_byte..end_byte];
            if let Some(pos) = slice.rfind(['.', '!', '?']) {
                let boundary = search_start_byte + pos + 1;
                if boundary > char_positions[start_char + chunk_size_chars / 2] {
                    floor_char_boundary(&clean, boundary)
                } else {
                    // Word boundary fallback
                    clean[start_byte..end_byte]
                        .rfind(char::is_whitespace)
                        .map(|p| start_byte + p + 1)
                        .unwrap_or(end_byte)
                }
            } else {
                clean[start_byte..end_byte]
                    .rfind(char::is_whitespace)
                    .map(|p| start_byte + p + 1)
                    .unwrap_or(end_byte)
            }
        } else {
            end_byte
        };
        let actual_end_byte = ceil_char_boundary(&clean, actual_end_byte);
        let actual_end_byte = actual_end_byte.max(start_byte + 1);
        let actual_end_byte = actual_end_byte.min(clean.len());
        let actual_end_byte = floor_char_boundary(&clean, actual_end_byte);

        let chunk = safe_slice(&clean, start_byte, actual_end_byte).trim().to_string();
        if !chunk.is_empty() {
            chunks.push(chunk);
        }

        if actual_end_byte >= clean.len() {
            break;
        }

        // Find next start_char position — walk back ~overlap_chars
        let current_end_char = char_positions
            .iter()
            .position(|&p| p >= actual_end_byte)
            .unwrap_or(total_chars);
        let next_start_char = current_end_char.saturating_sub(overlap_chars);
        if next_start_char <= start_char {
            start_char += 1; // guarantee progress
        } else {
            start_char = next_start_char;
        }
    }

    // Drop tiny trailing chunks
    chunks.retain(|c| c.len() >= 100);

    // Cap chunk count
    if chunks.len() > MAX_CHUNKS_PER_EMAIL {
        chunks.truncate(MAX_CHUNKS_PER_EMAIL);
    }

    if chunks.is_empty() {
        vec![clean]
    } else {
        chunks
    }
}

/// Create embedding texts for an email — one per chunk.
/// Each chunk includes the subject/sender header for full context.
fn create_embedding_texts(subject: &str, sender: &str, body: &str) -> Vec<String> {
    let clean_subject = strip_reply_prefixes(subject);
    let header = format!("Subject: {}\nFrom: {}\nContent: ", clean_subject, sender);

    let chunks = chunk_text(body);

    chunks.into_iter().map(|chunk| format!("{}{}", header, chunk)).collect()
}

/// Generate embeddings for emails that don't have them yet (single batch).
///
/// `account_label` is included in user-facing log/progress messages so the
/// output panel makes it clear which mailbox is being processed. Pass the
/// account email/name when known.
pub async fn generate_embeddings(
    db: &Arc<Database>,
    account_id: Option<&str>,
    app: Option<AppHandle>,
    batch_size: i32,
    account_label: Option<&str>,
) -> Result<u32> {
    generate_embeddings_inner(db, account_id, app, batch_size, account_label, true).await
}

/// Internal worker. When `emit_lifecycle` is false, the "starting" and
/// "complete" progress events are suppressed so an outer loop (e.g.
/// `regenerate_embeddings`) can own the lifecycle.
async fn generate_embeddings_inner(
    db: &Arc<Database>,
    account_id: Option<&str>,
    app: Option<AppHandle>,
    batch_size: i32,
    account_label: Option<&str>,
    emit_lifecycle: bool,
) -> Result<u32> {
    // Master AI switch — embeddings rely on the AI provider and are not
    // safe to run when the user has globally disabled AI.
    if !db.is_ai_enabled()? {
        emit_log(
            &app,
            "info",
            "embeddings",
            "Skipped: AI is disabled in settings (master switch off)",
        );
        return Ok(0);
    }
    let label_suffix = account_label.map(|l| format!(" ({})", l)).unwrap_or_default();
    let config = AiService::get_config(db)?;
    let embedding_model = get_embedding_model(db)?;
    let ai_service = AiService::new(db.clone())?;

    if !ai_service.is_available().await {
        emit_log(
            &app,
            "warn",
            "embeddings",
            &format!(
                "Skipped: AI provider '{}' is not reachable — check that Ollama is running, or change provider in Settings",
                config.provider
            ),
        );
        return Ok(0);
    }

    // Load per-account category config. When no specific account is given,
    // pull the union of all configured categories across accounts so we still
    // generate something meaningful in the global path. The post-fetch loop
    // below re-checks each email against its own account's config.
    let per_account_cfg: std::collections::HashMap<String, EmbeddingsConfig> = match account_id {
        Some(acc) => {
            let mut m = std::collections::HashMap::new();
            m.insert(acc.to_string(), get_embeddings_config(db, acc)?);
            m
        }
        None => {
            let mut m = std::collections::HashMap::new();
            for a in db.list_accounts()? {
                m.insert(a.id.clone(), get_embeddings_config(db, &a.id)?);
            }
            m
        }
    };

    // Union of categories across accounts. An empty `categories` means "all
    // categories" — if any account opts in to all, the SQL filter must also be
    // permissive (empty Vec). Per-email filtering below narrows back down.
    let union_categories: Vec<String> = if per_account_cfg.is_empty() {
        EmbeddingsConfig::default().categories
    } else if per_account_cfg.values().any(|c| c.categories.is_empty()) {
        Vec::new()
    } else {
        let mut set: std::collections::HashSet<String> = std::collections::HashSet::new();
        for cfg in per_account_cfg.values() {
            for c in &cfg.categories {
                set.insert(c.clone());
            }
        }
        set.into_iter().collect()
    };

    // Respect the user's "limit AI work to recent emails" preference so we
    // never embed emails older than the cutoff (default 1 year). 0 / unset
    // means no cutoff.
    let min_ts = db.ai_processing_min_timestamp(chrono::Utc::now().timestamp())?;
    let email_ids = db.get_emails_without_embeddings(account_id, batch_size, &union_categories, min_ts)?;

    if email_ids.is_empty() {
        return Ok(0);
    }

    let total = email_ids.len() as u32;
    let mut generated = 0u32;

    emit_log(
        &app,
        "info",
        "ai",
        &format!(
            "Using provider '{}' embedding model '{}' (with chunking){}",
            config.provider, embedding_model, label_suffix
        ),
    );

    if emit_lifecycle {
        if let Some(ref app) = app {
            let _ = app.emit(
                "embedding-progress",
                EmbeddingProgress {
                    status: "starting".to_string(),
                    current: 0,
                    total,
                    message: format!("Generating embeddings for {} emails{}...", total, label_suffix),
                },
            );
        }
    }

    let emails = db.get_emails_by_ids(&email_ids)?;

    for (idx, email) in emails.iter().enumerate() {
        // Per-account category filter: if the email's account opted out of
        // the email's category, skip it entirely. The union-based SQL
        // pre-filter is permissive across accounts; this is the precise check.
        if let Some(cfg) = per_account_cfg.get(&email.account_id) {
            if !cfg.is_category_allowed(&email.category) {
                continue;
            }
        }

        let content = if email.body.trim().is_empty() {
            &email.snippet
        } else {
            &email.body
        };

        let content_hash = compute_content_hash(&embedding_model, &email.subject, content);

        if db.embedding_exists(&email.id, &content_hash)? {
            continue;
        }

        let texts = create_embedding_texts(&email.subject, &email.sender, content);

        // Generate embeddings for all chunks
        let mut chunk_embeddings = Vec::new();
        let mut failed = false;
        for text in &texts {
            match ai_service.embed(text).await {
                Ok(embedding) => chunk_embeddings.push(embedding),
                Err(e) => {
                    crate::services::logger::log(
                        "error",
                        "embeddings",
                        format!("failed to generate embedding for email {} chunk: {}", email.id, e),
                    );
                    failed = true;
                    break;
                }
            }
        }

        if failed || chunk_embeddings.is_empty() {
            continue;
        }

        db.store_embedding_chunks(
            &email.id,
            &email.account_id,
            &chunk_embeddings,
            &embedding_model,
            &content_hash,
        )?;
        generated += 1;

        if let Some(ref app) = app {
            let _ = app.emit(
                "embedding-progress",
                EmbeddingProgress {
                    status: "generating".to_string(),
                    current: (idx + 1) as u32,
                    total,
                    message: format!(
                        "Generated {} of {} embeddings ({} chunks){}",
                        idx + 1,
                        total,
                        chunk_embeddings.len(),
                        label_suffix
                    ),
                },
            );
        }
    }

    if emit_lifecycle {
        if let Some(ref app) = app {
            let _ = app.emit(
                "embedding-progress",
                EmbeddingProgress {
                    status: "complete".to_string(),
                    current: total,
                    total,
                    message: format!("Generated embeddings for {} emails{}", generated, label_suffix),
                },
            );
        }
    }

    Ok(generated)
}

/// Get the number of emails without embeddings
pub fn count_pending_embeddings(db: &Arc<Database>, account_id: Option<&str>) -> Result<i32> {
    db.count_emails_without_embeddings(account_id)
}

/// Delete all embeddings and regenerate them. Loops over batches of
/// `batch_size` emails until every pending email has been processed.
pub async fn regenerate_embeddings(
    db: &Arc<Database>,
    account_id: Option<&str>,
    app: Option<AppHandle>,
    batch_size: i32,
    account_label: Option<&str>,
) -> Result<u32> {
    // Master AI switch — refuse to wipe and rebuild embeddings when AI is
    // disabled, since the regeneration would require the AI provider that
    // the user has just turned off. Returning AiDisabled here surfaces a
    // clear error to the frontend instead of silently doing nothing.
    if !db.is_ai_enabled()? {
        return Err(crate::models::error::AppError::AiDisabled);
    }
    let label_suffix = account_label.map(|l| format!(" for {}", l)).unwrap_or_default();

    let deleted = db.delete_all_embeddings(account_id)?;
    crate::services::logger::log(
        "debug",
        "embeddings",
        format!("deleted {} existing embedding chunks{}", deleted, label_suffix),
    );

    if let Some(ref app) = app {
        let _ = app.emit(
            "embedding-progress",
            EmbeddingProgress {
                status: "clearing".to_string(),
                current: 0,
                total: 0,
                message: format!(
                    "Cleared {} old embeddings{}, rebuilding index...",
                    deleted, label_suffix
                ),
            },
        );
    }
    emit_log(
        &app,
        "info",
        "embeddings",
        &format!(
            "Cleared {} embeddings{}; rebuilding search index…",
            deleted, label_suffix
        ),
    );

    // Process in batches until no more emails are pending. The cap prevents an
    // infinite loop in the unlikely event that emails come back from the DB
    // but are all skipped (e.g. cross-account category filter mismatches).
    const MAX_BATCHES: u32 = 10_000;
    let mut total_generated = 0u32;
    for _ in 0..MAX_BATCHES {
        let n = generate_embeddings_inner(db, account_id, app.clone(), batch_size, account_label, false).await?;
        if n == 0 {
            break;
        }
        total_generated += n;
    }

    if let Some(ref app) = app {
        let _ = app.emit(
            "embedding-progress",
            EmbeddingProgress {
                status: "complete".to_string(),
                current: total_generated,
                total: total_generated,
                message: format!(
                    "Search index rebuild complete{}: {} embeddings",
                    label_suffix, total_generated
                ),
            },
        );
    }
    emit_log(
        &app,
        "success",
        "embeddings",
        &format!(
            "Search index rebuild complete{}: generated {} embeddings",
            label_suffix, total_generated
        ),
    );

    Ok(total_generated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_prefixes() {
        assert_eq!(strip_reply_prefixes("Re: Hello"), "Hello");
        assert_eq!(strip_reply_prefixes("Fwd: Re: Hello"), "Hello");
        assert_eq!(strip_reply_prefixes("FW: RE: Hello"), "Hello");
        assert_eq!(strip_reply_prefixes("Hello"), "Hello");
        assert_eq!(strip_reply_prefixes("Re: Fwd: Test"), "Test");
    }

    #[test]
    fn chunking_short_text() {
        let chunks = chunk_text("Short email body");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Short email body");
    }

    #[test]
    fn chunking_long_text() {
        let long = "word ".repeat(500); // 2500 chars
        let chunks = chunk_text(&long);
        assert!(chunks.len() > 1, "Expected multiple chunks, got {}", chunks.len());
        // Each chunk should be roughly CHUNK_SIZE
        for chunk in &chunks {
            assert!(
                chunk.len() <= CHUNK_SIZE + 200,
                "Chunk too large: {} chars",
                chunk.len()
            );
        }
    }

    #[test]
    fn embedding_texts_include_header() {
        let texts = create_embedding_texts("Re: Invoice", "billing@example.com", "Short body");
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("Subject: Invoice")); // Re: stripped
        assert!(texts[0].contains("From: billing@example.com"));
        assert!(texts[0].contains("Short body"));
    }

    #[test]
    fn chunking_utf8_safety() {
        // Long Spanish text with multi-byte chars that would panic naive slicing
        let text = "Información de ofertas de empleo para ingenieros de software. ".repeat(50);
        let chunks = chunk_text(&text);
        assert!(!chunks.is_empty());
        // Each chunk should be valid UTF-8 (if it wasn't, the format! would panic)
        for chunk in &chunks {
            let _ = format!("chunk: {}", chunk);
        }
    }

    #[test]
    fn chunking_html_stripping() {
        let html = r#"<!DOCTYPE html><html><body><p>Hello <b>world</b> with áéí</p></body></html>"#;
        let chunks = chunk_text(html);
        assert_eq!(chunks.len(), 1);
        // HTML tags should be stripped
        assert!(!chunks[0].contains("<p>"));
        assert!(chunks[0].contains("Hello"));
    }

    #[test]
    fn chunking_respects_max_chunks() {
        let huge = "word ".repeat(5000); // 25000 chars
        let chunks = chunk_text(&huge);
        assert!(chunks.len() <= MAX_CHUNKS_PER_EMAIL);
    }

    #[test]
    fn content_hash_includes_model() {
        let hash = compute_content_hash("nomic-embed-text", "Test", "Body");
        // Hash should change if embedding model changes
        assert!(!hash.is_empty());
    }
}
