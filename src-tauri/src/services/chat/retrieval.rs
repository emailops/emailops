//! Retrieval for chat-with-your-emails: hybrid vector + FTS with RRF fusion,
//! recency weighting, thread-dedup, optional query rewrite/HyDE and LLM
//! reranking, plus the smart-body-slice helpers `build_prompt` uses.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::time::timeout;

use crate::ai::provider::AIProvider;
use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Email, RetrievalTrace};
use crate::services::retrieval::{
    dedup_by_thread, fetch_fts, fetch_vector, fuse_rrf, FtsRequest, Ranking, VectorRequest,
};
use crate::util::html::strip_html_for_fts;

use super::emit_log;
use super::truncate_chars;

// ── Tuning constants ────────────────────────────────────────────────────────

/// Number of source emails to feed to the LLM per turn.
pub(super) const TOP_K_SOURCES: usize = 8;
/// Upstream candidate pool passed into the LLM reranker. Must be ≥ TOP_K_SOURCES.
/// Set to ~2x top-k so the reranker has meaningful latitude to promote items
/// RRF+recency mis-ranked — without blowing the reranker's context budget.
const RERANK_POOL: usize = 16;
/// Max wall-clock the reranker may consume before we fall back to RRF order.
/// Kept tight because this runs on every chat turn: 8s on a local model is a
/// lot of user-visible latency, so past this budget we prefer the baseline
/// ordering over making the user wait.
const RERANK_TIMEOUT: Duration = Duration::from_secs(8);

/// Skip the LLM reranker entirely for models that empirically can't score a
/// 16-candidate pool within [`RERANK_TIMEOUT`]. Observed during eval runs:
/// `gemma4:e2b` and similar sub-4B models consistently hit the timeout and
/// fall back to RRF order — we pay 8s of latency for zero reordering.
/// Matches common size tags: `:e2b`, `:1b`, `:2b`, `:3b`, `:4b`.
fn is_small_local_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains(":e2b")
        || m.contains(":1b")
        || m.contains(":2b")
        || m.contains(":3b")
        || m.contains(":4b")
        || m.contains("-1b")
        || m.contains("-2b")
        || m.contains("-3b")
        || m.contains("-4b")
}
/// Max wall-clock the query-rewrite/HyDE pre-step may consume. If it blows
/// the budget we fall back to the raw user query — retrieval should never be
/// gated on an optional enhancement step.
const QUERY_REWRITE_TIMEOUT: Duration = Duration::from_secs(6);
/// Minimum user-query length (in chars) before we bother with query rewriting.
/// Very short queries ("hoy", "factura?") don't benefit from expansion — the
/// LLM tends to invent unrelated context when given almost no signal.
const QUERY_REWRITE_MIN_CHARS: usize = 16;
/// Cap for per-ranker candidate lists before RRF fusion.
/// Trimmed from 30 → 20: with category pre-filtering (default: primary only)
/// the candidate pool is already more homogeneous, so fewer slots deliver
/// the same effective recall and cut vector/fetch latency.
const VEC_CANDIDATES: usize = 20;
const FTS_CANDIDATES: i32 = 30;
/// Number of top FTS hits always reserved in the final retrieval set.
/// Protects high-precision rare-term matches from being drowned by dense
/// vector noise.
const FTS_GUARANTEED_SLOTS: usize = 3;

/// Default category filter when the caller doesn't specify one. Primary only
/// keeps RAG signal dense — updates/promotions/social/forums pollute vector
/// neighborhoods and eat citation budget. Users can opt-in more categories
/// from the chat input dropdown.
pub const DEFAULT_RAG_CATEGORIES: &[&str] = &["primary"];

/// Recency bonus parameters. The decayed bonus is added to the fused RRF
/// score so that, among near-tied candidates, newer emails win. The weight
/// is calibrated against the max RRF score (≈ 0.034 summed across vec+fts
/// at rank 0) so a same-day email gets a meaningful but non-dominating
/// nudge.
const RECENCY_WEIGHT: f32 = 0.020;
/// Half-life in seconds for the recency decay. 60 days: a 2-month-old email
/// contributes half the bonus of today; a 6-month-old email contributes ~6%.
const RECENCY_HALF_LIFE_SECS: f64 = 60.0 * 86_400.0;
/// Budget for how long vector search may block before we give up and fall
/// back to FTS-only retrieval. CLAUDE.md notes vec_search on 47k emails took
/// 15s in a past version; the current 3-step implementation is ~100ms, but a
/// cold extension load or degraded DB can still stall it. 5s keeps chat
/// responsive in those cases.
const VEC_SEARCH_TIMEOUT: Duration = Duration::from_secs(5);
/// Max characters of each source body we feed the model (post HTML-strip).
/// Larger than a tight abstract: the model needs enough room to find specific
/// technical details buried mid-thread (e.g. env-var explanations, quoted
/// snippets). 4k chars × k=8 sources ≈ 32k chars ≈ ~8k tokens, comfortably
/// inside Ollama's default context window.
pub(crate) const MAX_SOURCE_BODY_CHARS: usize = 4000;
/// Characters to keep before the first query-term match when slicing a long
/// body around a hit. Gives the model context leading into the answer.
const SNIPPET_LEAD_CHARS: usize = 400;

// RRF weights match services/search.rs — keep in sync if that file changes.
const RRF_K: f32 = 60.0;
const VECTOR_WEIGHT: f32 = 0.55;
const FTS_WEIGHT: f32 = 0.50;
/// Extra score added to the top FTS_BOOST_TOP_N results on top of their
/// regular RRF contribution. Purpose: unique technical identifiers (e.g.
/// `ACME_API_KEY`) produce extremely strong BM25 scores but can
/// still end up mid-list after RRF because the vector side returns many
/// semi-relevant documents that collectively outweigh one exact hit.
/// This bonus lifts the top 1-2 exact-match emails to near the top of the
/// citation list so the model encounters them before it encounters noise.
const FTS_BOOST_TOP_N: usize = 2;
const FTS_BOOST_WEIGHT: f32 = 1.0;

// ── Public types ────────────────────────────────────────────────────────────

/// An email that retrieval picked for this turn, with the score and the
/// citation number the LLM should use for it.
#[derive(Debug, Clone)]
pub struct ScoredEmail {
    pub email: Email,
    pub body: String,
    pub score: f32,
    pub citation_number: i32,
}

// ── Retrieval ───────────────────────────────────────────────────────────────

/// Retrieve the top-K most relevant emails for `query`, scoped to `account_id`.
///
/// Strategy: vector KNN + FTS5 in parallel, then RRF fusion. If vector search
/// fails or times out we fall back to FTS-only and log a warning — the chat
/// still works, just with worse recall on semantic queries.
pub async fn retrieve_context(
    db: &Arc<Database>,
    provider: &dyn AIProvider,
    account_id: &str,
    query: &str,
    categories: &[String],
    k: usize,
) -> Result<Vec<ScoredEmail>> {
    let (sources, _trace) = retrieve_context_with_trace(db, provider, account_id, query, categories, k).await?;
    Ok(sources)
}

/// Resolve the category filter the DB should apply:
///   - empty slice → `None` (search every category)
///   - non-empty    → `Some(&categories)` passed straight to vec_search/fts_search
fn db_category_filter(categories: &[String]) -> Option<&[String]> {
    if categories.is_empty() {
        None
    } else {
        Some(categories)
    }
}

/// Like [`retrieve_context`] but also returns a [`RetrievalTrace`] describing
/// what happened (hit counts, timing, whether vector fell back). Used by
/// `run_chat_turn` so the reasoning trace can show retrieval stats.
pub async fn retrieve_context_with_trace(
    db: &Arc<Database>,
    provider: &dyn AIProvider,
    account_id: &str,
    query: &str,
    categories: &[String],
    k: usize,
) -> Result<(Vec<ScoredEmail>, RetrievalTrace)> {
    let t_total = std::time::Instant::now();
    let cat_filter = db_category_filter(categories);

    // ── 0. Query rewrite / HyDE (optional, bounded) ───────────────────────
    // A cheap LLM call broadens the embedding target by adding a rewritten
    // keyword-rich version + a hypothetical answer. Bounded by its own
    // timeout so retrieval never stalls on this optional step.
    //
    // Skipped for small local models (`:e2b`, `:1b`..`:4b`): they consistently
    // hit QUERY_REWRITE_TIMEOUT and the code falls back to the raw query
    // anyway, so we pay 5-6 s of every-turn latency on M1-class hardware for
    // zero retrieval benefit. Same reasoning as the rerank skip below.
    let t_qr = std::time::Instant::now();
    let expanded = if is_small_local_model(provider.model_name()) {
        None
    } else {
        // Fetch the user-editable rewrite template once per turn; falls back
        // to the registry default if the user has not customised it.
        let tpl = crate::services::prompts::get_template(db, "chat.query_rewrite")?;
        rewrite_query_hyde(provider, query, &tpl).await
    };
    let query_rewrite_ms: Option<i64> = expanded.as_ref().map(|_| t_qr.elapsed().as_millis() as i64);
    let effective_query: &str = expanded.as_deref().unwrap_or(query);
    let expanded_query_for_trace: String = if expanded.is_some() {
        effective_query.to_string()
    } else {
        String::new()
    };

    // ── 1. Vector scores (best-effort; tolerant to embedding/DB issues) ───
    // Times embedding vs vec_search separately so the reasoning panel can
    // show the user which one is actually slow. A single combined timeout
    // still protects the whole path.
    let t_vec = std::time::Instant::now();
    let mut vector_fallback = false;
    let mut embedding_ms: Option<i64> = None;
    let mut vec_search_ms: Option<i64> = None;
    let vec_fut = async {
        let t_emb = std::time::Instant::now();
        let emb = provider.embed(effective_query).await?.embedding;
        let emb_ms = t_emb.elapsed().as_millis() as i64;
        let t_vs = std::time::Instant::now();
        let scores = fetch_vector(
            db,
            VectorRequest {
                account_id,
                embedding: &emb,
                categories: cat_filter,
                limit: VEC_CANDIDATES,
            },
        )?;
        let vs_ms = t_vs.elapsed().as_millis() as i64;
        Ok::<(Vec<(String, f32)>, i64, i64), crate::models::error::AppError>((scores, emb_ms, vs_ms))
    };
    let vector_scores: Vec<(String, f32)> = match timeout(VEC_SEARCH_TIMEOUT, vec_fut).await {
        Ok(Ok((scores, emb_ms, vs_ms))) => {
            embedding_ms = Some(emb_ms);
            vec_search_ms = Some(vs_ms);
            scores
        }
        Ok(Err(e)) => {
            emit_log(
                "warn",
                &format!("vector search failed ({}); falling back to FTS-only", e),
            );
            vector_fallback = true;
            Vec::new()
        }
        Err(_) => {
            emit_log(
                "warn",
                &format!(
                    "vector search exceeded {}s; falling back to FTS-only",
                    VEC_SEARCH_TIMEOUT.as_secs()
                ),
            );
            vector_fallback = true;
            Vec::new()
        }
    };
    let vec_ms = t_vec.elapsed().as_secs_f64() * 1000.0;

    // ── 2. FTS scores ─────────────────────────────────────────────────────
    let t_fts = std::time::Instant::now();
    let fts_scores = fetch_fts(
        db,
        FtsRequest {
            account_id,
            query,
            categories: cat_filter,
            sender_email_eq: None,
            limit: FTS_CANDIDATES,
        },
    )?;
    let fts_ms = t_fts.elapsed().as_secs_f64() * 1000.0;
    let fts_ms_i64 = fts_ms.round() as i64;

    if vector_scores.is_empty() && fts_scores.is_empty() {
        emit_log("info", "no sources matched the question");
        let trace = RetrievalTrace {
            vector_hits: 0,
            fts_hits: 0,
            fused_top_k: 0,
            elapsed_ms: t_total.elapsed().as_millis() as i64,
            vector_fallback,
            categories: categories.to_vec(),
            thread_dedup_collapsed: 0,
            embedding_ms,
            vec_search_ms,
            fts_search_ms: fts_ms_i64,
            fetch_ms: 0,
            expansion_ms: 0,
            rerank_ms: None,
            rerank_timed_out: false,
            query_rewrite_ms,
            expanded_query: expanded_query_for_trace.clone(),
            invalid_citations: -1,
        };
        return Ok((Vec::new(), trace));
    }

    let vector_hits = vector_scores.len() as i32;
    let fts_hits = fts_scores.len() as i32;

    // ── 3. Reciprocal Rank Fusion ─────────────────────────────────────────
    // Three weighted rankings fused via `services::retrieval::fuse_rrf`:
    //   1. Vector (sorted by similarity desc) — weight VECTOR_WEIGHT
    //   2. FTS (sorted by bm25 asc; more negative = better) — weight FTS_WEIGHT
    //   3. FTS top-N — extra weight FTS_BOOST_WEIGHT so unique exact-term
    //      matches (e.g. `ACME_API_KEY`) lift above semi-relevant
    //      vector noise.
    // `fts_sorted` is kept around as a `Vec<&(String, f64)>` because the FTS
    // injection step (§7) and the trace need the BM25-ranked list.
    let mut vec_sorted: Vec<&(String, f32)> = vector_scores.iter().collect();
    vec_sorted.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut fts_sorted: Vec<&(String, f64)> = fts_scores.iter().collect();
    fts_sorted.sort_by(|a, b| a.1.total_cmp(&b.1));

    let vec_ids: Vec<String> = vec_sorted.iter().map(|(id, _)| id.clone()).collect();
    let fts_ids: Vec<String> = fts_sorted.iter().map(|(id, _)| id.clone()).collect();
    let fts_top_n: Vec<String> = fts_ids.iter().take(FTS_BOOST_TOP_N).cloned().collect();

    let fused_vec = fuse_rrf(
        &[
            Ranking {
                ids_in_order: &vec_ids,
                weight: VECTOR_WEIGHT,
            },
            Ranking {
                ids_in_order: &fts_ids,
                weight: FTS_WEIGHT,
            },
            Ranking {
                ids_in_order: &fts_top_n,
                weight: FTS_BOOST_WEIGHT,
            },
        ],
        RRF_K,
    );
    let mut fused: std::collections::HashMap<String, f32> = fused_vec.into_iter().collect();

    // ── 4. Load candidate metadata (single batched query, no bodies) ─────
    // We fetch metadata for every fused candidate — not just top-k — so
    // recency and thread-dedup can work across the full pool. Bodies are
    // fetched separately below for the final top-k only.
    let t_fetch = std::time::Instant::now();
    let candidate_ids: Vec<String> = fused.keys().cloned().collect();
    let candidate_emails = db.get_emails_by_ids(&candidate_ids)?;
    let mut emails_by_id: std::collections::HashMap<String, Email> =
        candidate_emails.into_iter().map(|e| (e.id.clone(), e)).collect();
    let fetch_ms = t_fetch.elapsed().as_secs_f64() * 1000.0;

    // ── 5. Recency bonus ─────────────────────────────────────────────────
    // Add an exponentially-decaying bonus keyed on email age. Calibrated
    // against the RRF score scale so newer emails win near-ties without
    // dominating the ranking. See RECENCY_WEIGHT / RECENCY_HALF_LIFE_SECS.
    let now_secs = Utc::now().timestamp() as f64;
    for (email_id, score) in fused.iter_mut() {
        if let Some(email) = emails_by_id.get(email_id) {
            let age_secs = (now_secs - email.timestamp as f64).max(0.0);
            let decay = (-age_secs / RECENCY_HALF_LIFE_SECS).exp() as f32;
            *score += RECENCY_WEIGHT * decay;
        }
    }

    // ── 6. Thread-dedup ──────────────────────────────────────────────────
    // Collapse candidates to the highest-scoring email per thread. Before
    // this, an active 5-email thread could eat 5 of the 8 citation slots
    // with near-identical content (the "shipping update" problem). After
    // dedup each thread contributes at most one candidate to the top-k,
    // and thread-expansion (step 8) still adds the latest sibling so the
    // model sees the most recent reply.
    let candidates_before_dedup = fused.len();
    // Sort first so dedup_by_thread (which keeps the first occurrence at the
    // best score) walks candidates highest-first → each thread's winning
    // email is the one we keep. Orphaned ids (metadata missing) become their
    // own pseudo-thread inside the closure, matching the previous behavior.
    let mut sorted_for_dedup: Vec<(String, f32)> = fused.into_iter().collect();
    sorted_for_dedup.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut ranked: Vec<(String, f32)> =
        dedup_by_thread(sorted_for_dedup, |id| emails_by_id.get(id).map(|e| e.thread_id.clone()));
    let thread_dedup_collapsed = candidates_before_dedup.saturating_sub(ranked.len());
    // Keep a larger pool than `k` so the reranker (below) has room to reorder.
    // For small local models the reranker always times out and contributes
    // nothing, so collapse the pool to `k` up-front and skip rerank entirely.
    let rerank_disabled = is_small_local_model(provider.model_name());
    let pool_size = if rerank_disabled { k } else { k.max(RERANK_POOL) };
    ranked.truncate(pool_size);

    // ── 7. FTS injection (thread-scoped) ─────────────────────────────────
    // Guarantee the top `FTS_GUARANTEED_SLOTS` FTS hits are represented in
    // the final list, even if thread-dedup/RRF pushed them out. We compare
    // by thread_id so an FTS hit is considered "already represented" if
    // another email from its thread made it in. This protects unique-token
    // matches (e.g. `ACME_API_KEY`) while respecting thread-dedup.
    if !fts_sorted.is_empty() {
        let existing_threads: std::collections::HashSet<String> = ranked
            .iter()
            .map(|(id, _)| {
                emails_by_id
                    .get(id)
                    .map(|e| e.thread_id.clone())
                    .unwrap_or_else(|| id.clone())
            })
            .collect();
        let existing_ids: std::collections::HashSet<String> = ranked.iter().map(|(id, _)| id.clone()).collect();

        let mut injections: Vec<(String, f32)> = Vec::new();
        for (fts_rank, (email_id, _)) in fts_sorted.iter().enumerate() {
            if injections.len() >= FTS_GUARANTEED_SLOTS {
                break;
            }
            if existing_ids.contains(email_id.as_str()) {
                continue;
            }
            let thread_key = emails_by_id
                .get(email_id.as_str())
                .map(|e| e.thread_id.clone())
                .unwrap_or_else(|| (*email_id).clone());
            if existing_threads.contains(&thread_key) {
                continue;
            }
            let injection_score = FTS_WEIGHT / (RRF_K + fts_rank as f32 + 1.0);
            injections.push(((*email_id).clone(), injection_score));
        }
        if !injections.is_empty() {
            let drop_count = injections.len().min(ranked.len());
            ranked.truncate(ranked.len().saturating_sub(drop_count));
            ranked.extend(injections);
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
            ranked.truncate(pool_size);
        }
    }

    // ── 8. Build ScoredEmail pool (pre-rerank) ────────────────────────────
    let mut pool: Vec<ScoredEmail> = Vec::with_capacity(ranked.len());
    for (citation_idx, (email_id, score)) in ranked.into_iter().enumerate() {
        if let Some(email) = emails_by_id.remove(&email_id) {
            let body = db.get_email_body(&email_id).unwrap_or_default();
            pool.push(ScoredEmail {
                email,
                body,
                score,
                citation_number: (citation_idx + 1) as i32,
            });
        }
    }

    // ── 8b. LLM reranker ─────────────────────────────────────────────────
    // Rescore the pool on a 0-10 relevance scale using the local LLM; on
    // timeout/parse-failure fall back to RRF order. Only runs when we have
    // more than `k` candidates — otherwise reranking is a no-op that still
    // costs an LLM call.
    let t_rerank = std::time::Instant::now();
    let (mut results, rerank_timed_out) = if pool.len() > k {
        let tpl = crate::services::prompts::get_template(db, "chat.rerank")?;
        rerank_candidates(provider, query, pool, &tpl).await
    } else {
        (pool, false)
    };
    let rerank_ms: Option<i64> = if results.is_empty() {
        None
    } else {
        Some(t_rerank.elapsed().as_millis() as i64)
    };
    results.truncate(k);
    // Citations may have shifted after reranking; re-number now.
    for (i, c) in results.iter_mut().enumerate() {
        c.citation_number = (i + 1) as i32;
    }

    // ── 5. Thread expansion ───────────────────────────────────────────────
    // For each thread that already appears in the results, also include the
    // most-recent email from that thread that wasn't retrieved via RRF. This
    // handles the common case where RAG finds the *question* email in a thread
    // (because it mentions the key terms) but the *answer* lives in a later
    // reply. Capped at THREAD_EXPANSION_LIMIT new emails so context stays
    // manageable.
    const THREAD_EXPANSION_LIMIT: usize = 3;
    let t_expansion = std::time::Instant::now();
    let retrieved_ids: std::collections::HashSet<String> = results.iter().map(|r| r.email.id.clone()).collect();
    // Unique thread IDs in retrieval order (most-relevant threads first).
    let unique_threads: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        results
            .iter()
            .filter_map(|r| {
                if seen.insert(r.email.thread_id.clone()) {
                    Some(r.email.thread_id.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    let mut expansion_count = 0;
    for thread_id in &unique_threads {
        if expansion_count >= THREAD_EXPANSION_LIMIT {
            break;
        }
        if let Ok(thread_emails) = db.get_thread(account_id, thread_id) {
            // get_thread returns emails ASC by timestamp; iterate in reverse to
            // find the most-recent sibling not already in results.
            if let Some(latest) = thread_emails.iter().rev().find(|e| !retrieved_ids.contains(&e.id)) {
                let body = db.get_email_body(&latest.id).unwrap_or_default();
                // Score it just below the weakest existing hit from this thread.
                let thread_min_score = results
                    .iter()
                    .filter(|r| &r.email.thread_id == thread_id)
                    .map(|r| r.score)
                    .fold(f32::INFINITY, f32::min);
                let expansion_score = thread_min_score * 0.9;
                results.push(ScoredEmail {
                    email: latest.clone(),
                    body,
                    score: expansion_score,
                    citation_number: 0, // re-assigned below
                });
                expansion_count += 1;
            }
        }
    }

    if expansion_count > 0 {
        // Re-sort so expansions land in score order, then re-number citations.
        results.sort_by(|a, b| b.score.total_cmp(&a.score));
        for (i, src) in results.iter_mut().enumerate() {
            src.citation_number = (i + 1) as i32;
        }
        emit_log(
            "debug",
            &format!("thread expansion: added {} sibling email(s)", expansion_count),
        );
    }

    let cats_label = if categories.is_empty() {
        "all".to_string()
    } else {
        categories.join(",")
    };
    emit_log("debug",
        &format!(
            "retrieval: vec={:.0}ms fts={:.0}ms fetch={:.0}ms total={:.0}ms -> {} sources ({} expanded, {} dedup) cats={}",
            vec_ms,
            fts_ms,
            fetch_ms,
            t_total.elapsed().as_secs_f64() * 1000.0,
            results.len(),
            expansion_count,
            thread_dedup_collapsed,
            cats_label,
        ),
    );

    let expansion_ms = t_expansion.elapsed().as_millis() as i64;

    let trace = RetrievalTrace {
        vector_hits,
        fts_hits,
        fused_top_k: results.len() as i32,
        elapsed_ms: t_total.elapsed().as_millis() as i64,
        vector_fallback,
        categories: categories.to_vec(),
        thread_dedup_collapsed: thread_dedup_collapsed as i32,
        embedding_ms,
        vec_search_ms,
        fts_search_ms: fts_ms_i64,
        fetch_ms: fetch_ms.round() as i64,
        expansion_ms,
        rerank_ms,
        rerank_timed_out,
        query_rewrite_ms,
        expanded_query: expanded_query_for_trace,
        invalid_citations: -1,
    };

    Ok((results, trace))
}

// ── Query rewrite / HyDE ────────────────────────────────────────────────────

/// Ask the local LLM to expand the user's query for retrieval.
///
/// Returns a string that concatenates: (a) the original query, (b) a rewritten
/// keyword-rich version, (c) a short hypothetical answer (HyDE). Embedding
/// this concatenation gives the vector side a broader semantic target than
/// the raw question alone — particularly helpful for bilingual (ES/EN)
/// queries where `nomic-embed-text` has weaker cross-lingual recall.
///
/// Bounded by [`QUERY_REWRITE_TIMEOUT`]: if the local model is slow we fall
/// back to the original query rather than making the user wait. Returns
/// `Ok(None)` on skip (too short / empty response).
async fn rewrite_query_hyde(provider: &dyn AIProvider, user_question: &str, template: &str) -> Option<String> {
    if user_question.chars().count() < QUERY_REWRITE_MIN_CHARS {
        return None;
    }

    // Terse prompt (template loaded from the user-editable prompt registry by
    // the caller). We expect the model to emit TWO lines — a reformulation
    // and a plausible answer — anything else is noise filtered below.
    let mut vars = std::collections::HashMap::new();
    vars.insert("user_question", user_question.to_string());
    let prompt = crate::services::prompts::render(template, &vars);

    let fut = provider.complete(&prompt, Default::default());
    let raw = match timeout(QUERY_REWRITE_TIMEOUT, fut).await {
        Ok(Ok(result)) => result.text,
        Ok(Err(_)) | Err(_) => return None,
    };

    let cleaned: Vec<&str> = raw.lines().map(str::trim).filter(|l| !l.is_empty()).take(2).collect();
    if cleaned.is_empty() {
        return None;
    }
    // Combine original + rewrites. The original stays so we don't lose exact
    // tokens the user typed (important for BM25-style FTS too, even though
    // FTS uses the raw query path).
    let expanded = format!("{}\n{}", user_question, cleaned.join("\n"));
    Some(expanded)
}

// ── LLM reranker ────────────────────────────────────────────────────────────

/// Ask the local LLM to rescore retrieval candidates on a 0-10 relevance
/// scale, then reorder them by the new scores. Falls back silently to the
/// input order on timeout or parse failure.
///
/// Input contract: `candidates` is the fused-top-N list, already deduped by
/// thread and anchored on smart snippets. We feed the model a compact
/// `[id] subject :: snippet` per row and expect back a list of `id=score`
/// tokens. Parsing is tolerant — any unparseable entry keeps its baseline
/// score, so a partial response still helps.
async fn rerank_candidates(
    provider: &dyn AIProvider,
    user_question: &str,
    mut candidates: Vec<ScoredEmail>,
    template: &str,
) -> (Vec<ScoredEmail>, bool) {
    if candidates.len() <= 1 {
        return (candidates, false);
    }

    // Build a compact rescoring prompt. Use the citation_number as the id the
    // model echoes back so we don't expose raw email IDs.
    let mut body = String::with_capacity(candidates.len() * 200);
    for (i, c) in candidates.iter().enumerate() {
        let subject = truncate_chars(&c.email.subject, 120);
        let raw = strip_html_for_fts(&c.body);
        let snippet = smart_body_slice(&raw, user_question, 400);
        let snippet = truncate_chars(&snippet, 400).replace('\n', " ");
        body.push_str(&format!("[{}] {} :: {}\n", i + 1, subject, snippet));
    }

    // Template loaded from the user-editable prompt registry by the caller.
    let mut vars = std::collections::HashMap::new();
    vars.insert("user_question", user_question.to_string());
    vars.insert("candidates", body);
    let prompt = crate::services::prompts::render(template, &vars);

    let fut = provider.complete(&prompt, Default::default());
    let raw = match timeout(RERANK_TIMEOUT, fut).await {
        Ok(Ok(result)) => result.text,
        Ok(Err(e)) => {
            emit_log("debug", &format!("rerank failed ({}); keeping baseline order", e));
            return (candidates, false);
        }
        Err(_) => {
            emit_log(
                "debug",
                &format!("rerank exceeded {}s; keeping baseline order", RERANK_TIMEOUT.as_secs()),
            );
            return (candidates, true);
        }
    };

    // Parse lines of the form `<id>=<score>` or `[<id>] <score>`. Tolerate
    // extra whitespace / punctuation so a slightly off-format model still
    // contributes useful signal.
    let mut scores: std::collections::HashMap<usize, f32> = std::collections::HashMap::new();
    for line in raw.lines() {
        let line = line.trim().trim_start_matches('[').trim_start_matches(['-', '*']);
        // Find digits before '=' or ']', then digits after '=' or whitespace.
        let mut sep = None;
        for (i, ch) in line.char_indices() {
            if ch == '=' || ch == ']' || ch == ':' {
                sep = Some(i);
                break;
            }
        }
        if let Some(pos) = sep {
            let id_part: String = line[..pos].chars().filter(|c| c.is_ascii_digit()).collect();
            let id = id_part.parse::<usize>().ok();
            let rest = line[pos + 1..].trim();
            if let (Some(id_val), Some(score_val)) =
                (id, rest.split_whitespace().next().and_then(|t| t.parse::<f32>().ok()))
            {
                scores.insert(id_val, score_val.clamp(0.0, 10.0));
            }
        }
    }

    if scores.is_empty() {
        emit_log("debug", "rerank: could not parse any scores; keeping baseline order");
        return (candidates, false);
    }

    // Apply scores: rewrite `score` as (rerank + tiny baseline tiebreak).
    // Keep the RRF score as a small additive tiebreak so ties fall back to
    // the hybrid-retrieval order rather than list order.
    const BASELINE_WEIGHT: f32 = 0.01;
    for (i, cand) in candidates.iter_mut().enumerate() {
        let id = i + 1;
        if let Some(rerank) = scores.get(&id) {
            cand.score = *rerank + cand.score * BASELINE_WEIGHT;
        }
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    // Re-number citations to match the new order.
    for (i, c) in candidates.iter_mut().enumerate() {
        c.citation_number = (i + 1) as i32;
    }
    (candidates, false)
}

/// Outcome of [`smart_body_slice_indexed`]: the sliced text plus the char
/// offset (within the returned string) of the query-term anchor, when an
/// anchor was found. `None` means head-truncation was used and the snippet
/// is not anchored on a match.
#[derive(Debug, Clone)]
pub(crate) struct SlicedBody {
    pub text: String,
    /// Char offset inside `text` where the first matching query token
    /// appears. None = head-truncation fallback.
    pub anchor_char: Option<usize>,
}

/// Extract up to `max_chars` of `body` as a context snippet for the LLM.
///
/// If `body` fits within the budget, return it as-is. Otherwise find the
/// earliest occurrence of any content-bearing token from `query` and return
/// a window around that hit (with leading context of `SNIPPET_LEAD_CHARS`).
/// Falls back to head truncation if no query token matches.
///
/// This matters for long threads where the relevant sentence lives mid-body
/// — head-only truncation would strip the exact paragraph the model needs
/// to answer.
pub(crate) fn smart_body_slice(body: &str, query: &str, max_chars: usize) -> String {
    smart_body_slice_indexed(body, query, max_chars).text
}

/// Like [`smart_body_slice`] but also returns the anchor offset so callers
/// can insert a visible "relevant region" marker at the right spot when the
/// snippet was pulled around a query-term match. Used by `build_prompt` to
/// give the LLM a chunk-level pointer into the exact paragraph retrieval
/// matched on.
pub(crate) fn smart_body_slice_indexed(body: &str, query: &str, max_chars: usize) -> SlicedBody {
    let body_trim = body.trim();
    let total_chars = body_trim.chars().count();
    if total_chars <= max_chars {
        return SlicedBody {
            text: body_trim.to_string(),
            anchor_char: None,
        };
    }

    // Content-bearing tokens: ≥4 chars, alphanumeric / underscore
    // (matches identifier-style tokens like ACME_API_KEY).
    // Sorted longest-first: a long token like `ACME_API_KEY` is more
    // specific than a short one like `idhub` and should anchor the snippet
    // window. Without this, a generic early mention (e.g. `Acme` in a
    // quoted question) can pull the slice away from the actual answer
    // paragraph that contains the specific identifier.
    let mut tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.chars().count() >= 4)
        .map(str::to_lowercase)
        .collect();
    tokens.sort_by_key(|b| std::cmp::Reverse(b.len()));
    tokens.dedup();

    if tokens.is_empty() {
        return SlicedBody {
            text: truncate_chars(body_trim, max_chars),
            anchor_char: None,
        };
    }

    let lower = body_trim.to_lowercase();
    // Find the first token (longest-first) that appears in the body and use
    // its position as the anchor. We do NOT take `min()` across all tokens —
    // that would pick the earliest generic match, which often lands in
    // boilerplate or the quoted previous message rather than the answer.
    let anchor_byte_pos: Option<usize> = tokens.iter().find_map(|t| lower.find(t.as_str()));

    let Some(byte_pos) = anchor_byte_pos else {
        return SlicedBody {
            text: truncate_chars(body_trim, max_chars),
            anchor_char: None,
        };
    };

    let char_pos = body_trim[..byte_pos].chars().count();
    let start_char = char_pos.saturating_sub(SNIPPET_LEAD_CHARS);

    let window: String = body_trim.chars().skip(start_char).take(max_chars).collect();
    let window_chars = window.chars().count();

    let prefix = if start_char > 0 { "…" } else { "" };
    let suffix = if start_char + window_chars < total_chars {
        "…"
    } else {
        ""
    };
    // Anchor offset inside the returned text, accounting for the optional
    // leading "…" we prepend above.
    let anchor_in_window = char_pos - start_char;
    let anchor_in_text = anchor_in_window + prefix.chars().count();

    SlicedBody {
        text: format!("{}{}{}", prefix, window, suffix),
        anchor_char: Some(anchor_in_text),
    }
}

/// Insert a visible chunk-level marker at the anchor position so the LLM can
/// focus attention on the exact paragraph that matched retrieval.
///
/// Wraps 1 line above and below the anchor to form a minimal "answer window"
/// without inflating context cost. Used in [`build_prompt`].
pub(crate) fn mark_relevant_region(sliced: &SlicedBody) -> String {
    let Some(anchor) = sliced.anchor_char else {
        // No anchor → nothing to highlight, return as-is.
        return sliced.text.clone();
    };
    // Find the start of the line containing the anchor and the end of the
    // line after it (generous enough to cover a multi-line answer without
    // running the marker into an entire paragraph).
    let chars: Vec<char> = sliced.text.chars().collect();
    if anchor >= chars.len() {
        return sliced.text.clone();
    }
    let start = chars[..anchor]
        .iter()
        .rposition(|c| *c == '\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    // Look for the 2nd newline after the anchor so a short answer paragraph
    // is fully inside the region.
    let mut end = chars.len();
    let mut seen = 0;
    for (i, ch) in chars.iter().enumerate().skip(anchor) {
        if *ch == '\n' {
            seen += 1;
            if seen >= 2 {
                end = i;
                break;
            }
        }
    }
    // Cap the highlighted region so we never drown the marker in a wall of
    // text (e.g. a long quoted thread with no newlines).
    const MAX_REGION_CHARS: usize = 600;
    let end = end.min(start + MAX_REGION_CHARS).min(chars.len());

    let before: String = chars[..start].iter().collect();
    let region: String = chars[start..end].iter().collect();
    let after: String = chars[end..].iter().collect();
    format!("{before}\n>>> RELEVANT REGION (answer likely here) >>>\n{region}\n<<< END RELEVANT REGION <<<\n{after}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_slice_indexed_returns_anchor_for_matched_token() {
        let body = "x".repeat(500) + " ACME_API_KEY is the app token. " + &"y".repeat(3000);
        let sliced = smart_body_slice_indexed(&body, "ACME_API_KEY", 600);
        assert!(sliced.anchor_char.is_some(), "expected an anchor on token match");
        assert!(sliced.text.contains("ACME_API_KEY"));
    }

    #[test]
    fn mark_relevant_region_inserts_markers_when_anchored() {
        let sliced = SlicedBody {
            text: "line A\nline B with answer\nline C\n".to_string(),
            anchor_char: Some(8), // inside "line B"
        };
        let out = mark_relevant_region(&sliced);
        assert!(out.contains(">>> RELEVANT REGION"));
        assert!(out.contains("<<< END RELEVANT REGION"));
    }

    #[test]
    fn mark_relevant_region_passthrough_when_no_anchor() {
        let sliced = SlicedBody {
            text: "plain head-truncated text".to_string(),
            anchor_char: None,
        };
        let out = mark_relevant_region(&sliced);
        assert_eq!(out, "plain head-truncated text");
    }
}
