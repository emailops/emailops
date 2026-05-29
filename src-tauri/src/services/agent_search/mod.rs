// Agentic email search.
//
// Given a natural-language query (e.g. "emails en los que envío facturas")
// returns a ranked list of relevant emails. Supports several "modes" so that
// the eval harness in `crate::evals::agent_search` can A/B them against each
// other on the same query.
//
// Modes:
//   Baseline       — delegates to `services::search::search_emails` (FTS5 only).
//                    This is what the user gets today; the reference point.
//   Hybrid         — FTS5 ∪ vec_search, fused with Reciprocal Rank Fusion.
//                    No LLM. Cheap and language-agnostic on the retrieval side.
//   Smart          — Hybrid + an LLM "query understanding" step that pulls
//                    keywords, direction (sent-by-me / received-by-me) and an
//                    embedding query from the question, then a per-result LLM
//                    relevance gate that drops irrelevant pool members. This
//                    is the "agentic-lite" version: one round of planning,
//                    retrieval, judging, in <= 3 LLM calls.
//
// The full multi-round tool loop ("Agent") is intentionally out of scope for
// this iteration — we want a measurable baseline before adding complexity.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::ai::provider::CompletionOptions;
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Email;
use crate::services::ai::AiService;
use crate::services::retrieval::{
    fetch_fts, fetch_vector, fuse_rrf, FtsRequest, Ranking, VectorRequest, DEFAULT_RRF_K,
};
use crate::util::html::strip_html_for_fts;

const DEFAULT_TOP_K: usize = 15;
const HYBRID_POOL_SIZE: usize = 60;
const RELEVANCE_BODY_CHARS: usize = 600;

// ── Public API ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSearchMode {
    Baseline,
    Hybrid,
    Smart,
}

impl AgentSearchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentSearchMode::Baseline => "baseline",
            AgentSearchMode::Hybrid => "hybrid",
            AgentSearchMode::Smart => "smart",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSearchHit {
    pub email_id: String,
    pub subject: String,
    pub sender: String,
    pub sender_email: String,
    pub recipients: Vec<String>,
    pub timestamp: i64,
    pub mailbox: String,
    /// Fused score (RRF) or LLM relevance score in [0, 1]. Higher is better.
    pub score: f32,
    /// Why this email was kept — useful in eval reports.
    pub reason: String,
    pub snippet: String,
    /// True when sender_email matches the account's own email (i.e. the user
    /// sent this message). Available even in baseline mode since it's a pure
    /// metadata flag from the database.
    pub sent_by_user: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSearchResult {
    pub query: String,
    pub mode: AgentSearchMode,
    pub hits: Vec<AgentSearchHit>,
    /// Diagnostics — populated for "smart" mode; empty otherwise.
    pub query_plan: Option<QueryPlan>,
    /// Total wall time spent inside the agent search call, in ms.
    pub elapsed_ms: i64,
    /// Counts per retrieval stage. Useful for debugging recall problems.
    pub stage_counts: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryPlan {
    pub keywords: Vec<String>,
    /// "sent_by_me" | "received_by_me" | null (no constraint).
    pub direction: Option<String>,
    pub semantic_query: String,
    /// Raw LLM output before parsing — kept for debugging when parsing fails.
    pub raw: String,
}

#[derive(Debug, Clone)]
pub struct AgentSearchOptions {
    pub mode: AgentSearchMode,
    pub top_k: usize,
    /// Categories filter applied to retrieval. None = all categories.
    pub categories: Option<Vec<String>>,
}

impl Default for AgentSearchOptions {
    fn default() -> Self {
        Self {
            mode: AgentSearchMode::Smart,
            top_k: DEFAULT_TOP_K,
            categories: None,
        }
    }
}

/// Top-level entry point.
///
/// Resolves the user's email address from the account row so direction
/// filters work, dispatches to the per-mode implementation, and assembles
/// `AgentSearchHit`s with body snippets.
pub async fn run_agent_search(
    db: Arc<Database>,
    ai: Option<&AiService>,
    account_id: &str,
    query: &str,
    opts: AgentSearchOptions,
) -> Result<AgentSearchResult> {
    let start = Instant::now();
    let user_email = resolve_user_email(&db, account_id)?;

    let (hits, plan, stage_counts) = match opts.mode {
        AgentSearchMode::Baseline => {
            let (h, sc) = run_baseline(&db, account_id, &user_email, query, &opts)?;
            (h, None, sc)
        }
        AgentSearchMode::Hybrid => {
            let (h, sc) = run_hybrid(&db, ai, account_id, &user_email, query, &opts, None).await?;
            (h, None, sc)
        }
        AgentSearchMode::Smart => {
            let ai_ref =
                ai.ok_or_else(|| AppError::AiError("smart mode requires an AiService instance".to_string()))?;
            let plan = analyze_query(ai_ref, query).await?;
            let (h, sc) = run_hybrid(&db, Some(ai_ref), account_id, &user_email, query, &opts, Some(&plan)).await?;
            let filtered = relevance_filter(ai_ref, query, &plan, h, opts.top_k).await?;
            (filtered, Some(plan), sc)
        }
    };

    Ok(AgentSearchResult {
        query: query.to_string(),
        mode: opts.mode,
        hits,
        query_plan: plan,
        elapsed_ms: start.elapsed().as_millis() as i64,
        stage_counts,
    })
}

// ── Mode: Baseline ───────────────────────────────────────────────────────────

fn run_baseline(
    db: &Arc<Database>,
    account_id: &str,
    user_email: &str,
    query: &str,
    opts: &AgentSearchOptions,
) -> Result<(Vec<AgentSearchHit>, HashMap<String, usize>)> {
    let cats: Option<&[String]> = opts.categories.as_deref();
    // Use `db.fts_search()` (OR-joined, stopword-filtered, bm25-ranked) instead
    // of `db.search_emails()` (AND-joined, used for the UI's precision-oriented
    // filter bar). On NL queries the AND join collapses recall to 0 because
    // every stopword becomes a required prefix.
    let ranked = fetch_fts(
        db,
        FtsRequest {
            account_id,
            query,
            categories: cats,
            sender_email_eq: None,
            limit: opts.top_k as i32,
        },
    )?;
    let mut stage_counts = HashMap::new();
    stage_counts.insert("fts".to_string(), ranked.len());

    let mut hits: Vec<AgentSearchHit> = Vec::with_capacity(ranked.len());
    for (i, (eid, _rank)) in ranked.into_iter().enumerate() {
        if let Some(email) = db.get_email_by_id(&eid)? {
            hits.push(make_hit(db, email, user_email, 1.0 / (i as f32 + 1.0), "FTS match"));
        }
    }

    Ok((hits, stage_counts))
}

// ── Mode: Hybrid (FTS5 ∪ vec_search, RRF) ────────────────────────────────────

async fn run_hybrid(
    db: &Arc<Database>,
    ai: Option<&AiService>,
    account_id: &str,
    user_email: &str,
    query: &str,
    opts: &AgentSearchOptions,
    plan: Option<&QueryPlan>,
) -> Result<(Vec<AgentSearchHit>, HashMap<String, usize>)> {
    let cats: Option<&[String]> = opts.categories.as_deref();
    let mut stage_counts: HashMap<String, usize> = HashMap::new();

    // 1) FTS5: use plan's keywords if available, else raw query. We go through
    // `db.fts_search()` so terms are OR-joined and stopword-filtered (the
    // production `db.search_emails()` is AND-joined and zeroes out NL queries).
    let fts_query = plan
        .map(|p| {
            if p.keywords.is_empty() {
                query.to_string()
            } else {
                p.keywords.join(" ")
            }
        })
        .unwrap_or_else(|| query.to_string());

    // Push direction=sent_by_me into the FTS query: bm25 with subject weight 3.0
    // otherwise buries longer sent-email subjects ("Re: ...") under short
    // received subjects ("🎉 Propuesta aceptada"), starving the pool of any
    // sent-by-user candidates. received_by_me stays as a post-filter since
    // received emails dominate most corpora anyway.
    let sender_filter: Option<&str> =
        plan.and_then(|p| p.direction.as_deref())
            .and_then(|d| if d == "sent_by_me" { Some(user_email) } else { None });
    let fts_ranked = fetch_fts(
        db,
        FtsRequest {
            account_id,
            query: &fts_query,
            categories: cats,
            sender_email_eq: sender_filter,
            limit: HYBRID_POOL_SIZE as i32,
        },
    )?;
    stage_counts.insert("fts".to_string(), fts_ranked.len());

    // 2) Vector: embed the semantic_query (or full query) and KNN search.
    let mut vec_email_ids: Vec<(String, f32)> = Vec::new();
    if let Some(ai_ref) = ai {
        let embed_text = plan
            .map(|p| {
                if p.semantic_query.is_empty() {
                    query.to_string()
                } else {
                    p.semantic_query.clone()
                }
            })
            .unwrap_or_else(|| query.to_string());

        match ai_ref.embed(&embed_text).await {
            Ok(vec) => {
                match fetch_vector(
                    db,
                    VectorRequest {
                        account_id,
                        embedding: &vec,
                        categories: cats,
                        limit: HYBRID_POOL_SIZE,
                    },
                ) {
                    Ok(hits) => {
                        vec_email_ids = hits;
                    }
                    Err(e) => {
                        crate::services::logger::log(
                            "error",
                            "search",
                            format!("agent_search: vec_search error: {e} — continuing with FTS only"),
                        );
                    }
                }
            }
            Err(e) => {
                crate::services::logger::log(
                    "error",
                    "search",
                    format!("agent_search: embed error: {e} — continuing with FTS only"),
                );
            }
        }
    }
    stage_counts.insert("vector".to_string(), vec_email_ids.len());

    // 3) RRF fusion via `services::retrieval`. Track per-id provenance in a
    //    side table so the `reason` string can still cite which list(s)
    //    contributed the hit — fuse_rrf returns just the fused scores.
    let fts_ids: Vec<String> = fts_ranked.iter().map(|(id, _)| id.clone()).collect();
    let vec_ids: Vec<String> = vec_email_ids.iter().map(|(id, _)| id.clone()).collect();
    let mut sources: HashMap<String, Vec<String>> = HashMap::new();
    for (rank, eid) in fts_ids.iter().enumerate() {
        sources
            .entry(eid.clone())
            .or_default()
            .push(format!("fts#{}", rank + 1));
    }
    for (rank, eid) in vec_ids.iter().enumerate() {
        sources
            .entry(eid.clone())
            .or_default()
            .push(format!("vec#{}", rank + 1));
    }
    let ranked = fuse_rrf(
        &[
            Ranking {
                ids_in_order: &fts_ids,
                weight: 1.0,
            },
            Ranking {
                ids_in_order: &vec_ids,
                weight: 1.0,
            },
        ],
        DEFAULT_RRF_K,
    );
    stage_counts.insert("pool".to_string(), ranked.len());

    // 5) Hydrate emails by ID. We use get_email_by_id but want them in ranked order.
    let take_n = (opts.top_k * 3).min(HYBRID_POOL_SIZE);
    let mut hits: Vec<AgentSearchHit> = Vec::with_capacity(take_n);
    for (eid, score) in ranked.iter().take(take_n) {
        let email = match db.get_email_by_id(eid)? {
            Some(e) => e,
            None => continue,
        };
        let reason = format!(
            "rrf={:.4} via {}",
            score,
            sources.get(eid).map(|v| v.join(",")).unwrap_or_default()
        );
        hits.push(make_hit(db, email, user_email, *score, &reason));
    }

    // 6) Direction filter from query plan (if smart mode).
    if let Some(p) = plan {
        if let Some(dir) = p.direction.as_deref() {
            let before = hits.len();
            match dir {
                "sent_by_me" => hits.retain(|h| h.sent_by_user),
                "received_by_me" => hits.retain(|h| !h.sent_by_user),
                _ => {}
            }
            stage_counts.insert("after_direction".to_string(), hits.len());
            crate::services::logger::log(
                "debug",
                "search",
                format!("agent_search: direction={dir} filter: {before} → {}", hits.len()),
            );
        }
    }

    // Hybrid mode (no LLM filter) trims to top_k here. Smart mode does it
    // after the LLM relevance pass.
    if plan.is_none() {
        hits.truncate(opts.top_k);
    }

    Ok((hits, stage_counts))
}

// ── Mode: Smart — query understanding + relevance gate ───────────────────────

async fn analyze_query(ai: &AiService, query: &str) -> Result<QueryPlan> {
    let prompt = format!(
        "You are converting a natural-language email search query into a structured search plan.\n\
The user's emails are in Spanish and English. The user is a freelance CTO who sends \
proposals/invoices to clients and receives invoices from vendors.\n\n\
QUERY: {query}\n\n\
Return STRICT JSON only (no prose, no code fences) with this exact shape:\n\
{{\n\
  \"keywords\": [\"...\"],     // short list of Spanish/English keywords for FTS5 search. Include synonyms (e.g. for 'propuesta' add 'proposal', 'oferta', 'cotización').\n\
  \"direction\": \"sent_by_me\" | \"received_by_me\" | null,  // null if not specified by the query\n\
  \"semantic_query\": \"...\"   // a rephrased English+Spanish sentence to embed for vector search\n\
}}\n\n\
Rules:\n\
- \"envío\", \"envié\", \"he enviado\", \"mando\" → direction=sent_by_me\n\
- \"recibo\", \"recibí\", \"me llegan\", \"de proveedores\", \"from <someone>\" → direction=received_by_me\n\
- If direction is unclear, return null. Do not guess.\n\
- Keep keywords to <=6 distinct terms. Stem aggressively (\"factura\", not \"facturas\")."
    );

    let raw = ai
        .complete(
            &prompt,
            "agent_search_plan",
            Some(CompletionOptions {
                temperature: Some(0.0),
                max_tokens: Some(300),
                think: Some(false),
            }),
        )
        .await
        .unwrap_or_else(|e| {
            crate::services::logger::log(
                "error",
                "search",
                format!("agent_search: plan LLM error: {e} — falling back to empty plan"),
            );
            String::new()
        });

    let parsed = parse_plan_json(&raw);
    Ok(QueryPlan {
        keywords: parsed.keywords,
        direction: parsed.direction,
        semantic_query: if parsed.semantic_query.is_empty() {
            query.to_string()
        } else {
            parsed.semantic_query
        },
        raw,
    })
}

#[derive(Default, Debug, Deserialize)]
struct ParsedPlan {
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    semantic_query: String,
}

fn parse_plan_json(raw: &str) -> ParsedPlan {
    let stripped = strip_code_fence(raw);
    // Find first { ... last } in case the model added prose.
    let start = stripped.find('{');
    let end = stripped.rfind('}');
    let json_slice = match (start, end) {
        (Some(s), Some(e)) if e > s => &stripped[s..=e],
        _ => return ParsedPlan::default(),
    };
    serde_json::from_str::<ParsedPlan>(json_slice).unwrap_or_else(|err| {
        crate::services::logger::log(
            "error",
            "search",
            format!(
                "agent_search: plan JSON parse failed: {err} — raw: {}",
                truncate(raw, 200)
            ),
        );
        ParsedPlan::default()
    })
}

/// LLM-graded relevance gate. For each candidate, asks the model to rate the
/// email's relevance to the query in {0, 1, 2}. Anything <1 is dropped. The
/// kept items are re-sorted by score (ties broken by RRF order).
async fn relevance_filter(
    ai: &AiService,
    query: &str,
    plan: &QueryPlan,
    candidates: Vec<AgentSearchHit>,
    top_k: usize,
) -> Result<Vec<AgentSearchHit>> {
    if candidates.is_empty() {
        return Ok(candidates);
    }

    // Batch in chunks of 8 to keep prompts small for a 4B local model.
    const BATCH: usize = 8;
    let mut out: Vec<AgentSearchHit> = Vec::new();
    for chunk in candidates.chunks(BATCH) {
        let prompt = build_relevance_prompt(query, plan, chunk);
        let raw = match ai
            .complete(
                &prompt,
                "agent_search_relevance",
                Some(CompletionOptions {
                    temperature: Some(0.0),
                    max_tokens: Some(400),
                    think: Some(false),
                }),
            )
            .await
        {
            Ok(t) => t,
            Err(e) => {
                crate::services::logger::log(
                    "error",
                    "search",
                    format!("agent_search: relevance LLM error: {e} — keeping batch as-is"),
                );
                // On failure, keep candidates with their existing scores so we
                // don't black-hole results when the model is flaky.
                for h in chunk {
                    out.push(h.clone());
                }
                continue;
            }
        };
        let scores = parse_relevance_json(&raw, chunk.len());
        for (mut hit, (score, why)) in chunk.iter().cloned().zip(scores) {
            if score >= 1 {
                hit.score = score as f32 / 2.0; // 0.5 or 1.0
                if !why.is_empty() {
                    hit.reason = format!("LLM[{score}/2] {why}");
                }
                out.push(hit);
            }
        }
    }

    // Re-sort by LLM score (desc), then by timestamp (desc) as a sensible
    // tiebreaker (recent matches usually preferred).
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.timestamp.cmp(&a.timestamp))
    });
    out.truncate(top_k);
    Ok(out)
}

fn build_relevance_prompt(query: &str, plan: &QueryPlan, batch: &[AgentSearchHit]) -> String {
    let mut s = String::new();
    s.push_str("You are rating how relevant each email is to a search query.\n\n");
    s.push_str(&format!("QUERY: {}\n", query));
    if let Some(d) = plan.direction.as_deref() {
        s.push_str(&format!("DIRECTION HINT: {}\n", d));
    }
    s.push_str("\nFor each email below, rate relevance:\n");
    s.push_str("  2 = email clearly matches the query (right topic AND right direction if specified)\n");
    s.push_str("  1 = email is plausibly relevant (right topic, direction ambiguous)\n");
    s.push_str("  0 = not relevant (wrong topic, wrong direction, or unrelated)\n\n");
    s.push_str("Return STRICT JSON only (no prose, no code fences):\n");
    s.push_str("{ \"ratings\": [ {\"i\": 0, \"score\": 0|1|2, \"why\": \"...\"}, ... ] }\n\n");
    s.push_str("EMAILS:\n");
    for (i, h) in batch.iter().enumerate() {
        let dir = if h.sent_by_user {
            "SENT BY USER"
        } else {
            "RECEIVED BY USER"
        };
        s.push_str(&format!(
            "[{}] {dir} | from: {} <{}> | subject: {} | snippet: {}\n",
            i,
            truncate(&h.sender, 60),
            truncate(&h.sender_email, 80),
            truncate(&h.subject, 100),
            truncate(&h.snippet.replace('\n', " "), 200),
        ));
    }
    s
}

#[derive(Deserialize)]
struct RelevanceWire {
    #[serde(default)]
    ratings: Vec<RelevanceItem>,
}

#[derive(Deserialize)]
struct RelevanceItem {
    #[serde(default)]
    i: usize,
    #[serde(default)]
    score: i32,
    #[serde(default)]
    why: String,
}

fn parse_relevance_json(raw: &str, batch_len: usize) -> Vec<(i32, String)> {
    let mut out: Vec<(i32, String)> = vec![(0, String::new()); batch_len];
    let stripped = strip_code_fence(raw);
    let start = stripped.find('{');
    let end = stripped.rfind('}');
    let slice = match (start, end) {
        (Some(s), Some(e)) if e > s => &stripped[s..=e],
        _ => return out,
    };
    match serde_json::from_str::<RelevanceWire>(slice) {
        Ok(w) => {
            for item in w.ratings {
                if item.i < batch_len {
                    out[item.i] = (item.score.clamp(0, 2), item.why);
                }
            }
        }
        Err(e) => {
            crate::services::logger::log(
                "error",
                "search",
                format!(
                    "agent_search: relevance JSON parse failed: {e} — raw: {}",
                    truncate(raw, 200)
                ),
            );
        }
    }
    out
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn resolve_user_email(db: &Database, account_id: &str) -> Result<String> {
    let accounts = db.list_accounts()?;
    accounts
        .into_iter()
        .find(|a| a.id == account_id)
        .map(|a| a.email)
        .ok_or_else(|| AppError::NotFound(format!("account {} not found", account_id)))
}

fn make_hit(db: &Database, email: Email, user_email: &str, score: f32, reason: &str) -> AgentSearchHit {
    let body = db.get_email_body(&email.id).unwrap_or_default();
    let plain = strip_html_for_fts(&body);
    let snippet = if plain.trim().is_empty() {
        email.snippet.clone()
    } else {
        truncate(&plain, RELEVANCE_BODY_CHARS)
    };
    let sent = email.sender_email.eq_ignore_ascii_case(user_email);
    AgentSearchHit {
        email_id: email.id,
        subject: email.subject,
        sender: email.sender,
        sender_email: email.sender_email,
        recipients: email.recipients,
        timestamp: email.timestamp,
        mailbox: email.mailbox,
        score,
        reason: reason.to_string(),
        snippet,
        sent_by_user: sent,
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```json") {
        return rest.trim_start_matches('\n').trim_end_matches("```").trim().to_string();
    }
    if let Some(rest) = t.strip_prefix("```") {
        return rest.trim_start_matches('\n').trim_end_matches("```").trim().to_string();
    }
    t.to_string()
}
