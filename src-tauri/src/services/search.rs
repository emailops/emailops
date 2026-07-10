use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use unicode_normalization::UnicodeNormalization;

use crate::ai::ollama::{parse_search_query_patterns, OllamaClient, ParsedSearchQuery};
use crate::ai::provider::{AIProvider, CompletionOptions};
use crate::db::Database;
use crate::models::error::Result;
use crate::models::Email;
use crate::services::ai::AiService;
use crate::services::retrieval::{fetch_fts, fetch_vector, fuse_rrf, FtsRequest, Ranking, VectorRequest};

fn emit_log(_app: &Option<AppHandle>, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub emails: Vec<EmailWithScore>,
    pub query: String,
    pub parsed_query: Option<ParsedSearchQuery>,
    pub ai_available: bool,
    pub search_method: SearchMethod,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMethod {
    /// RAG: Semantic search using embeddings + AI generation
    Rag,
    /// AI-powered natural language parsing via Ollama
    AiParsed,
    /// Pattern-based parsing (from:, to:, subject:, etc.)
    PatternParsed,
    /// Simple keyword search
    KeywordSearch,
}

/// Resolve the accounts a search targets: the given account, or every
/// enabled account for the unified ("All accounts") view.
fn search_target_accounts(db: &Arc<Database>, account_id: Option<&str>) -> Result<Vec<String>> {
    match account_id {
        Some(id) => Ok(vec![id.to_string()]),
        None => Ok(db
            .list_accounts()?
            .into_iter()
            .filter(|a| a.enabled)
            .map(|a| a.id)
            .collect()),
    }
}

/// Run `db.search_emails` once per target account and merge the results
/// newest-first, truncated to `limit`. Pattern parsing and every other
/// search decision happens once in the caller — only the DB call fans out.
#[allow(clippy::too_many_arguments)]
fn db_search_merged(
    db: &Arc<Database>,
    targets: &[String],
    query: &str,
    categories: Option<&[String]>,
    from_filter: Option<&str>,
    to_filter: Option<&str>,
    subject_filter: Option<&str>,
    after_timestamp: Option<i64>,
    before_timestamp: Option<i64>,
    tag_filters: Option<&[String]>,
    limit: i32,
) -> Result<Vec<Email>> {
    if let [single] = targets {
        // Single account: preserve the DB's exact ordering untouched.
        return db.search_emails(
            single,
            query,
            categories,
            from_filter,
            to_filter,
            subject_filter,
            after_timestamp,
            before_timestamp,
            tag_filters,
            limit,
        );
    }
    let mut merged: Vec<Email> = Vec::new();
    for account in targets {
        merged.extend(db.search_emails(
            account,
            query,
            categories,
            from_filter,
            to_filter,
            subject_filter,
            after_timestamp,
            before_timestamp,
            tag_filters,
            limit,
        )?);
    }
    // Each per-account result is newest-first; merge to a single newest-first
    // list and truncate to the same limit a single-account search gets.
    merged.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| b.id.cmp(&a.id)));
    merged.truncate(limit.max(0) as usize);
    Ok(merged)
}

/// Search emails using natural language or structured queries.
/// When AI is available, uses RAG (Retrieval-Augmented Generation) for semantic search.
///
/// `account_id: None` searches across every enabled account (unified
/// "All accounts" view) — the FTS path fans out per account and merges.
pub async fn search_emails(
    db: &Arc<Database>,
    account_id: Option<&str>,
    query: &str,
    use_ai: bool,
    categories: Option<&[String]>,
    app: Option<AppHandle>,
) -> Result<SearchResult> {
    let search_start = std::time::Instant::now();
    let target_accounts = search_target_accounts(db, account_id)?;

    // TEMPORARILY DISABLED: AI parsing + RAG. Ollama query parse times out after
    // 60s and vec_search on 47k embeddings takes 15s. Pure FTS is <100 ms and
    // returns properly ranked results. Re-enable once vec_search is fixed and
    // the parse timeout is dropped to ~5 s. The SearchBar hint copy in
    // `src/components/Search/SearchBar.tsx` is gated on this flag — keep them
    // in sync if you toggle the disable.
    let _requested_ai = use_ai;
    let use_ai = false;

    // ── 1. Pattern parsing (instant, no network) ─────────────────────────────
    let t_parse = std::time::Instant::now();
    let mut search_method = SearchMethod::KeywordSearch;
    let mut parsed_query: Option<ParsedSearchQuery> = None;

    if let Some(parsed) = parse_search_query_patterns(query) {
        search_method = SearchMethod::PatternParsed;
        emit_log(
            &app,
            "debug",
            "search",
            &format!(
                "[{:.0}ms] Pattern parsed: from={:?} to={:?} subject={:?} after={:?} before={:?} keywords={:?}",
                t_parse.elapsed().as_secs_f64() * 1000.0,
                parsed.from_filter,
                parsed.to_filter,
                parsed.subject_filter,
                parsed.after_timestamp,
                parsed.before_timestamp,
                parsed.keywords
            ),
        );
        parsed_query = Some(parsed);
    }

    // ── 2. Determine whether AI is needed ────────────────────────────────────
    // For pure pattern queries (from:X, subject:Y with no residual keywords)
    // we skip the expensive AI availability check entirely.
    let residual_keywords = parsed_query.as_ref().map(build_effective_query).unwrap_or_default();
    let needs_ai = use_ai && (parsed_query.is_none() || !residual_keywords.is_empty());

    let (ai_available, _model, ollama, ai_service, provider_name) = if needs_ai {
        let t0 = std::time::Instant::now();
        let model = db.get_preference("ai_model")?.unwrap_or_default();
        let ollama = OllamaClient::new(Some(&model));
        let ai_service = AiService::new(db.clone()).ok();
        let provider_name = AiService::get_config(db)
            .map(|cfg| cfg.provider)
            .unwrap_or_else(|_| "ollama".to_string());
        let available = if let Some(service) = ai_service.as_ref() {
            service.is_available().await
        } else {
            ollama.is_available().await
        };
        emit_log(
            &app,
            "debug",
            "search",
            &format!(
                "[{:.0}ms] AI init & availability check (available={}, provider={}, model={})",
                t0.elapsed().as_secs_f64() * 1000.0,
                available,
                provider_name,
                model
            ),
        );
        (available, model, ollama, ai_service, provider_name)
    } else {
        emit_log(&app, "debug", "search", "AI check skipped (not needed for this query)");
        let m = String::new();
        let ollama = OllamaClient::new(Some(&m));
        let svc: Option<AiService> = None;
        (false, m, ollama, svc, String::new())
    };

    // ── 3. AI query parsing (only when no pattern found) ─────────────────────
    if use_ai && ai_available && parsed_query.is_none() {
        let t_ai = std::time::Instant::now();
        emit_log(&app, "debug", "search", "Trying AI query parsing...");
        let parse_result = if provider_name == "ollama" {
            ollama.parse_search_query(query).await
        } else if let Some(service) = ai_service.as_ref() {
            parse_search_query_with_provider(service, query).await
        } else {
            Err(crate::models::error::AppError::AiError(
                "AI service unavailable".to_string(),
            ))
        };

        match parse_result {
            Ok(ai_parsed) => {
                emit_log(
                    &app,
                    "debug",
                    "search",
                    &format!(
                        "[{:.0}ms] AI parsed: from={:?} to={:?} subject={:?} keywords={:?}",
                        t_ai.elapsed().as_secs_f64() * 1000.0,
                        ai_parsed.from_filter,
                        ai_parsed.to_filter,
                        ai_parsed.subject_filter,
                        ai_parsed.keywords
                    ),
                );
                if ai_parsed.from_filter.is_some()
                    || ai_parsed.to_filter.is_some()
                    || ai_parsed.subject_filter.is_some()
                    || ai_parsed.is_unread.is_some()
                    || ai_parsed.after_timestamp.is_some()
                    || ai_parsed.before_timestamp.is_some()
                {
                    search_method = SearchMethod::AiParsed;
                    parsed_query = Some(ai_parsed);
                } else {
                    emit_log(
                        &app,
                        "debug",
                        "search",
                        &format!(
                            "[{:.0}ms] AI returned no structured filters, falling through to semantic search",
                            t_ai.elapsed().as_secs_f64() * 1000.0,
                        ),
                    );
                }
            }
            Err(e) => {
                emit_log(
                    &app,
                    "debug",
                    "search",
                    &format!(
                        "[{:.0}ms] AI parsing failed: {}",
                        t_ai.elapsed().as_secs_f64() * 1000.0,
                        e
                    ),
                );
            }
        }
    }

    // ── 4. Dispatch search ───────────────────────────────────────────────────
    let t_dispatch = std::time::Instant::now();
    // RAG/semantic retrieval is single-account only — the unified view takes
    // the FTS paths below (AI search is currently hard-disabled anyway).
    let semantic_account = if use_ai && ai_available && parsed_query.is_none() {
        account_id
    } else {
        None
    };
    let emails: Vec<EmailWithScore> = if let Some(single_account) = semantic_account {
        // No structured filters → try RAG semantic search
        emit_log(&app, "debug", "search", "Using hybrid semantic search (RAG)");
        match semantic_search(db, &ollama, single_account, query, categories).await {
            Ok((results, used_rag)) => {
                if used_rag {
                    search_method = SearchMethod::Rag;
                }
                emit_log(
                    &app,
                    "debug",
                    "search",
                    &format!(
                        "[{:.0}ms] RAG search returned {} results (used_rag={})",
                        t_dispatch.elapsed().as_secs_f64() * 1000.0,
                        results.len(),
                        used_rag,
                    ),
                );
                results
            }
            Err(e) => {
                emit_log(
                    &app,
                    "debug",
                    "search",
                    &format!(
                        "[{:.0}ms] Semantic search failed: {}, falling back to keyword",
                        t_dispatch.elapsed().as_secs_f64() * 1000.0,
                        e,
                    ),
                );
                let t_kw = std::time::Instant::now();
                let results = emails_to_scored(keyword_search(db, &target_accounts, query, categories, 100)?, None);
                emit_log(
                    &app,
                    "debug",
                    "search",
                    &format!(
                        "[{:.0}ms] Keyword fallback returned {} results",
                        t_kw.elapsed().as_secs_f64() * 1000.0,
                        results.len()
                    ),
                );
                results
            }
        }
    } else if let Some(ref parsed) = parsed_query {
        let results = structured_search(
            db,
            &ollama,
            account_id,
            &target_accounts,
            parsed,
            use_ai && ai_available,
            categories,
            &app,
        )
        .await?;
        emit_log(
            &app,
            "debug",
            "search",
            &format!(
                "[{:.0}ms] Structured search returned {} results",
                t_dispatch.elapsed().as_secs_f64() * 1000.0,
                results.len(),
            ),
        );
        results
    } else {
        // Simple keyword search
        let match_reason = format!("Contains \"{}\"", query);
        let results = emails_to_scored(
            keyword_search(db, &target_accounts, query, categories, 100)?,
            Some(&match_reason),
        );
        emit_log(
            &app,
            "debug",
            "search",
            &format!(
                "[{:.0}ms] Keyword search returned {} results",
                t_dispatch.elapsed().as_secs_f64() * 1000.0,
                results.len(),
            ),
        );
        results
    };

    emit_log(
        &app,
        "debug",
        "search",
        &format!(
            "[{:.0}ms] Search complete: method={:?}, {} results for {:?}",
            search_start.elapsed().as_secs_f64() * 1000.0,
            search_method,
            emails.len(),
            query,
        ),
    );

    Ok(SearchResult {
        emails,
        query: query.to_string(),
        parsed_query,
        ai_available,
        search_method,
    })
}

#[allow(clippy::too_many_arguments)]
async fn structured_search(
    db: &Arc<Database>,
    ollama: &OllamaClient,
    account_id: Option<&str>,
    target_accounts: &[String],
    parsed: &ParsedSearchQuery,
    ai_enabled: bool,
    categories: Option<&[String]>,
    app: &Option<AppHandle>,
) -> Result<Vec<EmailWithScore>> {
    let residual_query = build_effective_query(parsed);
    let filter_reason = build_filter_match_reason(parsed);

    // Semantic retrieval stays single-account; unified mode (None) always
    // takes the DB filter path below.
    if let (true, false, Some(single_account)) = (ai_enabled, residual_query.is_empty(), account_id) {
        let t_sem = std::time::Instant::now();
        emit_log(
            app,
            "debug",
            "search",
            &format!("Structured: semantic retrieval with residual query: {}", residual_query),
        );
        let (semantic_results, used_rag) =
            semantic_search(db, ollama, single_account, &residual_query, categories).await?;

        if used_rag {
            let filtered = filter_scored_results(semantic_results, parsed, &filter_reason);
            if !filtered.is_empty() {
                emit_log(
                    app,
                    "debug",
                    "search",
                    &format!(
                        "[{:.0}ms] Structured semantic retrieval returned {} filtered results",
                        t_sem.elapsed().as_secs_f64() * 1000.0,
                        filtered.len()
                    ),
                );
                return Ok(filtered);
            }
        }
        emit_log(
            app,
            "debug",
            "search",
            &format!(
                "[{:.0}ms] Semantic path empty, falling back to DB filter",
                t_sem.elapsed().as_secs_f64() * 1000.0,
            ),
        );
    }

    let t_db = std::time::Instant::now();
    emit_log(
        app,
        "debug",
        "search",
        &format!(
            "Structured: DB filter search (from={:?}, to={:?}, subject={:?}, residual={:?})",
            parsed.from_filter, parsed.to_filter, parsed.subject_filter, residual_query
        ),
    );

    let tag_filters = if parsed.tag_filters.is_empty() {
        None
    } else {
        Some(parsed.tag_filters.as_slice())
    };
    let mut results = db_search_merged(
        db,
        target_accounts,
        &residual_query,
        categories,
        parsed.from_filter.as_deref(),
        parsed.to_filter.as_deref(),
        parsed.subject_filter.as_deref(),
        parsed.after_timestamp,
        parsed.before_timestamp,
        tag_filters,
        100,
    )?;
    emit_log(
        app,
        "debug",
        "search",
        &format!(
            "[{:.0}ms] DB filter query returned {} results",
            t_db.elapsed().as_secs_f64() * 1000.0,
            results.len(),
        ),
    );

    if results.is_empty() && !residual_query.is_empty() {
        let t_fallback = std::time::Instant::now();
        results = keyword_search(db, target_accounts, &residual_query, categories, 100)?;
        results.retain(|email| matches_parsed_filters(email, parsed));
        emit_log(
            app,
            "debug",
            "search",
            &format!(
                "[{:.0}ms] Keyword fallback after empty DB filter returned {} results",
                t_fallback.elapsed().as_secs_f64() * 1000.0,
                results.len(),
            ),
        );
    }

    Ok(emails_to_scored(results, Some(&filter_reason)))
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

/// Build a human-readable match reason from parsed filters
fn build_filter_match_reason(parsed: &ParsedSearchQuery) -> String {
    let mut parts = Vec::new();

    if let Some(ref from) = parsed.from_filter {
        parts.push(format!("from \"{}\"", from));
    }
    if let Some(ref to) = parsed.to_filter {
        parts.push(format!("to \"{}\"", to));
    }
    if let Some(ref subject) = parsed.subject_filter {
        parts.push(format!("subject contains \"{}\"", subject));
    }
    if parsed.is_unread == Some(true) {
        parts.push("unread".to_string());
    }
    if parsed.after_timestamp.is_some() || parsed.before_timestamp.is_some() {
        parts.push("date filtered".to_string());
    }
    if !parsed.keywords.is_empty() {
        parts.push(format!("contains \"{}\"", parsed.keywords.join(" ")));
    }

    if parts.is_empty() {
        "Matched filters".to_string()
    } else {
        format!("Matched: {}", parts.join(", "))
    }
}

fn build_effective_query(parsed: &ParsedSearchQuery) -> String {
    parsed.keywords.join(" ")
}

/// Keyword search via FTS with a cheap accent-insensitive retry.
///
/// Rationale: the previous fallback loaded all 47k emails (2.5 GB of body text)
/// into memory for a linear scan. A prod verification run showed that across 20
/// diverse queries the fallback found zero additional results but cost 409 s
/// on one empty-FTS query. FTS5 with `unicode61` already strips diacritics, so
/// the only remaining edge case is when the user's query itself contains
/// characters FTS normalizes differently. For those, we retry once with a
/// normalized query.
fn keyword_search(
    db: &Arc<Database>,
    targets: &[String],
    query: &str,
    categories: Option<&[String]>,
    limit: i32,
) -> Result<Vec<Email>> {
    let t = std::time::Instant::now();
    let direct = db_search_merged(
        db, targets, query, categories, None, None, None, None, None, None, limit,
    )?;
    emit_log(
        &None,
        "debug",
        "search",
        &format!(
            "[timing] db.search_emails: {:.0}ms ({} results)",
            t.elapsed().as_secs_f64() * 1000.0,
            direct.len(),
        ),
    );
    if !direct.is_empty() {
        return Ok(direct);
    }

    // Retry with normalized query (lowercase, accents stripped, punctuation → space).
    // Cheap — just another FTS call. No-op if the query is already normalized.
    let normalized = normalize_for_search(query);
    if normalized.is_empty() || normalized == query.trim().to_lowercase() {
        return Ok(direct);
    }
    let t = std::time::Instant::now();
    let retry = db_search_merged(
        db,
        targets,
        &normalized,
        categories,
        None,
        None,
        None,
        None,
        None,
        None,
        limit,
    )?;
    emit_log(
        &None,
        "debug",
        "search",
        &format!(
            "[timing] db.search_emails (normalized retry {:?}): {:.0}ms ({} results)",
            normalized,
            t.elapsed().as_secs_f64() * 1000.0,
            retry.len(),
        ),
    );
    Ok(retry)
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

fn filter_scored_results(
    results: Vec<EmailWithScore>,
    parsed: &ParsedSearchQuery,
    filter_reason: &str,
) -> Vec<EmailWithScore> {
    results
        .into_iter()
        .filter(|result| matches_parsed_filters(&result.email, parsed))
        .map(|mut result| {
            result.match_reason = Some(match &result.match_reason {
                Some(reason) => format!("{}, {}", reason, filter_reason),
                None => filter_reason.to_string(),
            });
            result
        })
        .collect()
}

fn matches_parsed_filters(email: &Email, parsed: &ParsedSearchQuery) -> bool {
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
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
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
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn seed_account(db: &Database, id: &str, email: &str, enabled: bool) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, enabled)
                 VALUES (?1, 'gmail', ?2, 'Test', 0, ?3)",
                rusqlite::params![id, email, enabled as i32],
            )
            .unwrap();
    }

    fn seed_searchable_email(db: &Database, id: &str, account: &str, thread: &str, subject: &str, timestamp: i64) {
        db.connection()
            .execute(
                "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                     VALUES (?1,?2,?3,?4,'Sender','s@ex.com','ex.com','[]','[]','snip',?5,0,'primary',0)",
                rusqlite::params![id, account, thread, subject, timestamp],
            )
            .unwrap();
        db.connection()
            .execute(
                "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, ?2, 'Sender', 'body')",
                rusqlite::params![id, subject],
            )
            .unwrap();
    }

    // Unified ("All accounts") search: `account_id: None` must merge FTS
    // results across enabled accounts, newest first, excluding disabled ones.
    #[tokio::test]
    async fn search_unified_merges_enabled_accounts_newest_first() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "a1@ex.com", true);
        seed_account(&db, "acc2", "a2@ex.com", true);
        seed_account(&db, "acc3", "a3@ex.com", false);

        seed_searchable_email(&db, "e1", "acc1", "t1", "Invoice January", 100);
        seed_searchable_email(&db, "e2", "acc2", "t2", "Invoice February", 200);
        seed_searchable_email(&db, "e3", "acc3", "t3", "Invoice March", 300);
        seed_searchable_email(&db, "e4", "acc1", "t4", "Lunch plans", 400);

        let result = search_emails(&db, None, "invoice", false, None, None).await.unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.email.id.as_str()).collect();

        assert_eq!(
            ids,
            vec!["e2", "e1"],
            "both enabled accounts merged newest-first; disabled acc3 excluded"
        );
    }

    // Single-account behavior must stay exactly as before.
    #[tokio::test]
    async fn search_single_account_unchanged() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "a1@ex.com", true);
        seed_account(&db, "acc2", "a2@ex.com", true);
        seed_searchable_email(&db, "e1", "acc1", "t1", "Invoice January", 100);
        seed_searchable_email(&db, "e2", "acc2", "t2", "Invoice February", 200);

        let result = search_emails(&db, Some("acc1"), "invoice", false, None, None)
            .await
            .unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.email.id.as_str()).collect();
        assert_eq!(ids, vec!["e1"], "single-account search must not leak other accounts");
    }

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

    /// Full-stack integration test for `from:` search through the service layer.
    /// Exercises the exact code path the Tauri command uses, minus AppHandle.
    ///
    /// Run with: cargo test -p emailops bench_search_service_from -- --nocapture --ignored
    #[tokio::test]
    #[ignore]
    async fn bench_search_service_from() {
        let (db, account_id) = match open_prod_db() {
            Some(v) => v,
            None => return,
        };

        // Pick a sender with some emails
        let from_name: String = db
            .reader()
            .query_row(
                "SELECT SUBSTR(sender_email, 1, INSTR(sender_email, '@') - 1) \
                 FROM emails WHERE account_id = ?1 \
                 GROUP BY sender_email ORDER BY COUNT(*) DESC LIMIT 1 OFFSET 2",
                rusqlite::params![account_id],
                |row| row.get(0),
            )
            .unwrap();
        let query = format!("from:{}", from_name);
        eprintln!("=== Service-level search: {:?} ===\n", query);

        // Warm up
        let _ = search_emails(&db, Some(&account_id), &query, false, None, None).await;

        // Timed runs
        for run in 1..=3 {
            let t = std::time::Instant::now();
            let result = search_emails(&db, Some(&account_id), &query, false, None, None)
                .await
                .unwrap();
            eprintln!(
                "Run {}: {:.0}ms — {} results (method={:?})",
                run,
                t.elapsed().as_secs_f64() * 1000.0,
                result.emails.len(),
                result.search_method,
            );
        }

        // Step-by-step breakdown
        eprintln!("\n--- Step-by-step breakdown ---");

        // 1. Pattern parse
        let t = std::time::Instant::now();
        let parsed = parse_search_query_patterns(&query).expect("should parse from: query");
        eprintln!("[{:.0}ms] Pattern parse", t.elapsed().as_secs_f64() * 1000.0);

        // 2. Build effective query (residual keywords)
        let t = std::time::Instant::now();
        let residual = build_effective_query(&parsed);
        let needs_ai = !residual.is_empty();
        eprintln!(
            "[{:.0}ms] Residual query: {:?}, needs_ai={}",
            t.elapsed().as_secs_f64() * 1000.0,
            residual,
            needs_ai,
        );

        // 3. db.search_emails (the DB layer)
        let t = std::time::Instant::now();
        let db_results = db
            .search_emails(
                &account_id,
                &residual,
                None,
                parsed.from_filter.as_deref(),
                parsed.to_filter.as_deref(),
                parsed.subject_filter.as_deref(),
                parsed.after_timestamp,
                parsed.before_timestamp,
                None,
                100,
            )
            .unwrap();
        eprintln!(
            "[{:.0}ms] db.search_emails returned {} results",
            t.elapsed().as_secs_f64() * 1000.0,
            db_results.len(),
        );

        // 4. emails_to_scored conversion
        let t = std::time::Instant::now();
        let filter_reason = build_filter_match_reason(&parsed);
        let scored = emails_to_scored(db_results, Some(&filter_reason));
        eprintln!(
            "[{:.0}ms] emails_to_scored ({} results)",
            t.elapsed().as_secs_f64() * 1000.0,
            scored.len(),
        );

        // 5. Simulate reader contention: hold a reader while searching
        eprintln!("\n--- Contention test: hold reader during search ---");
        let db2 = db.clone();
        let account_id2 = account_id.clone();
        let query2 = query.clone();

        // Hold one reader with a long-running query (simulate get_filtered_emails)
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let barrier2 = barrier.clone();
        let db_blocker = db.clone();
        let blocker = std::thread::spawn(move || {
            let conn = db_blocker.reader();
            eprintln!("  Blocker: acquired reader, sleeping 2s...");
            barrier2.wait(); // signal that lock is held
            std::thread::sleep(std::time::Duration::from_secs(2));
            drop(conn);
            eprintln!("  Blocker: released reader");
        });

        barrier.wait(); // wait for blocker to hold the reader
        let t = std::time::Instant::now();
        let result = search_emails(&db2, Some(&account_id2), &query2, false, None, None)
            .await
            .unwrap();
        let contention_ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "  Search under contention: {:.0}ms — {} results",
            contention_ms,
            result.emails.len(),
        );
        blocker.join().unwrap();

        if contention_ms > 2000.0 {
            eprintln!("  *** BLOCKED BY CONTENTION — pool exhausted or reader serialized ***");
        } else {
            eprintln!("  OK — search was not blocked by the held reader");
        }

        eprintln!("\n=== Done ===\n");
    }

    /// Run with: cargo test -p emailops bench_search_service_keyword -- --nocapture --ignored
    #[tokio::test]
    #[ignore]
    async fn bench_search_service_keyword() {
        let (db, account_id) = match open_prod_db() {
            Some(v) => v,
            None => return,
        };

        let query = "invoice";
        eprintln!("=== Service-level search: {:?} ===\n", query);

        for run in 1..=3 {
            let t = std::time::Instant::now();
            let result = search_emails(&db, Some(&account_id), query, false, None, None)
                .await
                .unwrap();
            eprintln!(
                "Run {}: {:.0}ms — {} results (method={:?})",
                run,
                t.elapsed().as_secs_f64() * 1000.0,
                result.emails.len(),
                result.search_method,
            );
        }
        eprintln!("\n=== Done ===\n");
    }

    /// Verify whether `normalized_keyword_search` ever produces results beyond
    /// pure FTS. If every query returns the same count from both paths, the
    /// fallback is dead code and can be deleted.
    ///
    /// Run with: cargo test -p emailops verify_fallback_is_dead_code -- --nocapture --ignored
    #[tokio::test]
    #[ignore]
    async fn verify_fallback_is_dead_code() {
        let (db, account_id) = match open_prod_db() {
            Some(v) => v,
            None => return,
        };

        let queries = [
            "invoice",
            "meeting",
            "project update",
            "factura",
            "facturación",
            "año",
            "aws",
            "google ads",
            "zoom meeting",
            "resume",
            "résumé",
            "action",
            "acción",
            "notification",
            "notificación",
            "xyznonexistent12345",
            "facturas aws",
            "meeting notes",
            "tax form",
            "contract agreement",
        ];

        eprintln!("\n━━━ FTS vs FTS+Fallback on {} queries ━━━\n", queries.len());

        let mut fallback_helped = 0;
        let mut identical = 0;
        let mut both_empty = 0;

        for query in queries {
            let t = std::time::Instant::now();
            let direct = db
                .search_emails(&account_id, query, None, None, None, None, None, None, None, 100)
                .unwrap();
            let fts_ms = t.elapsed().as_secs_f64() * 1000.0;

            let t = std::time::Instant::now();
            let combined = keyword_search(&db, std::slice::from_ref(&account_id), query, None, 100).unwrap();
            let combined_ms = t.elapsed().as_secs_f64() * 1000.0;

            if direct.is_empty() && combined.is_empty() {
                both_empty += 1;
                eprintln!(
                    "  {:28} FTS: 0 ({:4.0}ms) | combined: 0 ({:5.0}ms) both-empty",
                    format!("{:?}", query),
                    fts_ms,
                    combined_ms,
                );
            } else if direct.len() == combined.len() {
                identical += 1;
                eprintln!(
                    "  {:28} FTS: {:3} ({:4.0}ms) | combined: {:3} ({:5.0}ms) identical",
                    format!("{:?}", query),
                    direct.len(),
                    fts_ms,
                    combined.len(),
                    combined_ms,
                );
            } else if direct.is_empty() && !combined.is_empty() {
                fallback_helped += 1;
                eprintln!(
                    "  {:28} FTS: 0 ({:4.0}ms) | combined: {:3} ({:5.0}ms) FALLBACK HELPED",
                    format!("{:?}", query),
                    fts_ms,
                    combined.len(),
                    combined_ms,
                );
            } else {
                eprintln!(
                    "  {:28} FTS: {:3} ({:4.0}ms) | combined: {:3} ({:5.0}ms) DIFFERENT",
                    format!("{:?}", query),
                    direct.len(),
                    fts_ms,
                    combined.len(),
                    combined_ms,
                );
            }
        }

        eprintln!("\n━━━ Summary ━━━");
        eprintln!("  Total queries   : {}", queries.len());
        eprintln!("  Identical       : {}", identical);
        eprintln!("  Both empty      : {}", both_empty);
        eprintln!("  Fallback helped : {}", fallback_helped);
        if fallback_helped == 0 {
            eprintln!("\n  → Fallback is dead code on this DB. Safe to delete.");
        } else {
            eprintln!(
                "\n  → Fallback helped on {} queries. Investigate before deleting.",
                fallback_helped
            );
        }
    }

    /// Reproduce and diagnose a specific slow search on the production DB.
    /// Mirrors exactly what the `search_emails` Tauri command does.
    ///
    /// Run with: cargo test -p emailops report_slow_search -- --nocapture --ignored
    /// Override the query via env:  FTS_QUERY="facturas aws" cargo test ... -- --nocapture --ignored
    #[tokio::test]
    #[ignore]
    async fn report_slow_search() {
        let (db, account_id) = match open_prod_db() {
            Some(v) => v,
            None => return,
        };

        let query = std::env::var("FTS_QUERY").unwrap_or_else(|_| "facturas aws".to_string());
        let use_ai: bool = std::env::var("FTS_USE_AI")
            .map(|v| v != "0" && v.to_lowercase() != "false")
            .unwrap_or(true);

        eprintln!("\n╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║               Slow Search Reproduction Report                ║");
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        eprintln!("║ query    : {:?}", query);
        eprintln!("║ use_ai   : {}", use_ai);
        eprintln!("║ account  : {}", account_id);
        eprintln!("╚══════════════════════════════════════════════════════════════╝");

        // ── Run 1: full service-level search (what the Tauri command does) ──
        eprintln!("\n━━━ End-to-end service search (with timing breakdown) ━━━");
        let t_total = std::time::Instant::now();
        let result = search_emails(&db, Some(&account_id), &query, use_ai, None, None)
            .await
            .unwrap();
        let total_ms = t_total.elapsed().as_secs_f64() * 1000.0;

        eprintln!("\n┌──────────────────────────────────────────────────────────────");
        eprintln!(
            "│ TOTAL: {:.0}ms — {} results (method={:?}, ai_available={})",
            total_ms,
            result.emails.len(),
            result.search_method,
            result.ai_available
        );
        eprintln!("└──────────────────────────────────────────────────────────────");

        // ── Direct DB keyword search (no AI, no normalization) ──────────────
        eprintln!("\n━━━ Direct db.search_emails (pure FTS path, no AI) ━━━");
        for run in 1..=3 {
            let t = std::time::Instant::now();
            let results = db
                .search_emails(&account_id, &query, None, None, None, None, None, None, None, 100)
                .unwrap();
            eprintln!(
                "  Run {}: {:.0}ms — {} results",
                run,
                t.elapsed().as_secs_f64() * 1000.0,
                results.len(),
            );
        }

        // ── Individual sub-step timing (confirms breakdown) ─────────────────
        eprintln!("\n━━━ Isolated sub-step timing ━━━");

        // FTS only
        let t = std::time::Instant::now();
        let fts_scores = db.fts_search(&query, Some(&account_id), None, 50).unwrap_or_default();
        eprintln!(
            "  db.fts_search (top 50): {:.0}ms — {} hits",
            t.elapsed().as_secs_f64() * 1000.0,
            fts_scores.len(),
        );

        // Show first few FTS hits to sanity-check relevance
        if !fts_scores.is_empty() {
            eprintln!("  Top FTS hits (BM25 score — more negative is better):");
            for (email_id, score) in fts_scores.iter().take(5) {
                let subj: Option<String> = db
                    .reader()
                    .query_row(
                        "SELECT subject FROM emails WHERE id = ?1",
                        rusqlite::params![email_id],
                        |r| r.get(0),
                    )
                    .ok();
                eprintln!("    {:.3}  {}  {:?}", score, email_id, subj.unwrap_or_default());
            }
        }

        // Ollama embedding — only if use_ai
        if use_ai {
            let model = db.get_preference("ai_model").unwrap_or(None).unwrap_or_default();
            let ollama = OllamaClient::new(Some(&model));
            let t = std::time::Instant::now();
            let available = ollama.is_available().await;
            eprintln!(
                "  ollama.is_available: {:.0}ms — available={}, model={:?}",
                t.elapsed().as_secs_f64() * 1000.0,
                available,
                model,
            );

            if available {
                let t = std::time::Instant::now();
                match ollama.generate_embedding(&query).await {
                    Ok(emb) => {
                        eprintln!(
                            "  ollama.generate_embedding: {:.0}ms — dim={}",
                            t.elapsed().as_secs_f64() * 1000.0,
                            emb.len(),
                        );

                        // vec_search (KNN)
                        let t = std::time::Instant::now();
                        let vec_hits = db.vec_search(&emb, Some(&account_id), None, 150).unwrap_or_default();
                        eprintln!(
                            "  db.vec_search (KNN 150): {:.0}ms — {} hits",
                            t.elapsed().as_secs_f64() * 1000.0,
                            vec_hits.len(),
                        );
                    }
                    Err(e) => eprintln!(
                        "  ollama.generate_embedding FAILED after {:.0}ms: {}",
                        t.elapsed().as_secs_f64() * 1000.0,
                        e,
                    ),
                }
            }
        }

        eprintln!("\n══════════════════════════════════════════════════════════════");
        eprintln!("  Report complete");
        eprintln!("══════════════════════════════════════════════════════════════\n");
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

        // 4. get_filtered_emails — by domain
        if let Some(domain) = stats.top_domains.first() {
            let t = std::time::Instant::now();
            let result = db
                .get_filtered_emails(
                    crate::db::AccountScope::Account(&account_id),
                    Some(&domain.value),
                    None,
                    None,
                    None,
                    None,
                    50,
                    0,
                )
                .unwrap();
            eprintln!(
                "[{:.0}ms] get_filtered_emails(domain={:?}): {} emails, total={}",
                t.elapsed().as_secs_f64() * 1000.0,
                domain.value,
                result.emails.len(),
                result.total_count,
            );
        }

        // 5. get_filtered_emails — by sender
        if let Some(sender) = stats.top_senders.first() {
            let t = std::time::Instant::now();
            let result = db
                .get_filtered_emails(
                    crate::db::AccountScope::Account(&account_id),
                    None,
                    Some(&sender.value),
                    None,
                    None,
                    None,
                    50,
                    0,
                )
                .unwrap();
            eprintln!(
                "[{:.0}ms] get_filtered_emails(sender={:?}): {} emails, total={}",
                t.elapsed().as_secs_f64() * 1000.0,
                sender.value,
                result.emails.len(),
                result.total_count,
            );
        }

        // 6. get_filtered_emails — by tag
        let t = std::time::Instant::now();
        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(&account_id),
                None,
                None,
                Some("intent"),
                Some("informational"),
                None,
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

        eprintln!("\n=== Done ===\n");
    }
}
