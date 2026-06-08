//! Route classification for a chat turn: decide RAG-first vs tools-first from
//! the user's `chat.routing_mode` preference and a cheap keyword/date
//! heuristic.

use std::sync::Arc;

use crate::db::Database;
use crate::models::{RouteDecision, RouteMode};

// ── Routing (RAG vs tools-first) ────────────────────────────────────────────

/// Keywords that suggest the user wants a specific email / recent item /
/// count-style answer. These are cheap deterministic signals — when any match
/// we skip RAG and let the tool loop drive the query. Mixed EN/ES because the
/// user works in both.
const TOOLS_FIRST_KEYWORDS: &[&str] = &[
    // Draft / compose / reply intent. The user is asking us to WRITE something,
    // which must run the tool loop so `generate_email_draft` fires (saving a
    // real draft + opening the composer) instead of RAG free-writing the body.
    // Substrings are chosen to catch verb families: "contesta" matches
    // "contestar"/"contestarle", "responde" matches "responder", "redacta"
    // matches "redactar". Routing ToolsFirst on a false positive is cheap (the
    // model still has every tool), so we err toward catching the intent.
    // EN:
    "draft",
    "compose",
    "reply to",
    "respond to",
    "write a reply",
    "write an email",
    "write a response",
    // ES:
    "borrador",
    "redacta",
    "contesta",
    "responde",
    // Recency / specificity (EN)
    "latest",
    "most recent",
    "last email",
    "last message",
    "most recent email",
    "last invoice",
    "recent invoice",
    "show me the",
    "find the email",
    "open thread",
    "get thread",
    // Listings / counts (EN)
    "how many",
    "count of",
    "list all",
    "show all",
    "give me all",
    "pass me all",
    // Recency / specificity (ES)
    "último",
    "ultimo",
    "última",
    "ultima",
    "más reciente",
    "mas reciente",
    "muéstrame",
    "muestrame",
    "enséñame",
    "ensename",
    // Listings / counts (ES)
    "cuántos",
    "cuantos",
    "cuántas",
    "cuantas",
    "pásame",
    "pasame",
    "todas las",
    "todos los",
    "dame todos",
    "dame todas",
    // Time-bounded — current period (EN)
    "today",
    "yesterday",
    "this week",
    "this month",
    "this year",
    "last week",
    "last month",
    "last year",
    "this quarter",
    "last quarter",
    // Time-bounded — current period (ES)
    "hoy",
    "ayer",
    "anteayer",
    "esta semana",
    "semana pasada",
    "este mes",
    "mes pasado",
    "este año",
    "este ano",
    "año pasado",
    "ano pasado",
    "este trimestre",
    "último trimestre",
    "ultimo trimestre",
    "trimestre pasado",
    // Month names — Spanish. The risk of false positives ("hola Mayo") is
    // low and the cost of routing to ToolsFirst when RAG would have worked
    // is also low (the model still has tools to find the email). We omit
    // the unaccented "ano" form of "año" — too noisy.
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "setiembre",
    "octubre",
    "noviembre",
    "diciembre",
    // Month names — English.
    "january",
    "february",
    "march",
    "april",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Regex matching explicit date signals the keyword list can't catch as flat
/// substrings: 4-digit years in the plausible 1990-2099 range, ISO dates
/// (YYYY-MM-DD), and quarter shorthand (`Q1`..`Q4`). Compiled once.
static DATE_PATTERNS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
fn date_patterns() -> &'static regex::Regex {
    DATE_PATTERNS.get_or_init(|| {
        // \b avoids matching "Q5" inside a sentence or 2018 inside a phone
        // number like "2018-5550". ISO date pattern anchored separately so a
        // 4-digit year on its own still routes correctly.
        // Input is lowercased before matching, so quarter pattern uses `q`.
        // Hard-coded regex literal — Regex::new can only fail on invalid syntax,
        // which is checked at compile-time by every build.
        #[allow(clippy::unwrap_used)]
        let r = regex::Regex::new(r"\b(19|20)\d{2}\b|\b\d{4}-\d{2}-\d{2}\b|\bq[1-4]\b").unwrap();
        r
    })
}

/// Heuristic for detecting tools-first queries. Flat keyword substring match
/// (recency / aggregation / time-period words) plus a regex pass that catches
/// explicit years ("2018"), ISO dates ("2024-03-15"), and quarter shorthand
/// ("Q1"). Date-bound questions go to ToolsFirst because RAG over an
/// embeddings index can't reliably filter by date — search_emails with
/// `since`/`until` is the only correct path.
pub(super) fn heuristic_route(user_question: &str) -> Option<(RouteMode, Vec<String>)> {
    let q = user_question.to_lowercase();
    let mut matched = Vec::new();
    for kw in TOOLS_FIRST_KEYWORDS {
        if q.contains(kw) {
            matched.push((*kw).to_string());
        }
    }
    // Regex matches (years, ISO dates, quarters) — push the matched text so the
    // reasoning panel can show *which* date signal triggered the route.
    for m in date_patterns().find_iter(&q) {
        matched.push(m.as_str().to_string());
    }
    if !matched.is_empty() {
        Some((RouteMode::ToolsFirst, matched))
    } else {
        None
    }
}

/// Pick a retrieval strategy for this turn based on the user's configured
/// `chat.routing_mode` preference and a cheap keyword heuristic.
///
/// Modes:
///   - `always_rag`  → always `RagFirst` (pre-Fix-B behavior; useful for A/B).
///   - `always_tools` → always `ToolsFirst` (debug / eval comparison).
///   - `auto`        → heuristic-driven: `ToolsFirst` when a keyword matches,
///     otherwise `RagFirst`. Keeps RAG for open-ended questions.
///     This is the default when the preference is unset.
pub(super) fn classify_route(db: &Arc<Database>, user_question: &str) -> RouteDecision {
    let mode_pref = db
        .get_preference("chat.routing_mode")
        .ok()
        .flatten()
        .unwrap_or_else(|| "auto".to_string());

    match mode_pref.as_str() {
        "always_tools" => RouteDecision {
            mode: RouteMode::ToolsFirst,
            reason: "forced by preference chat.routing_mode=always_tools".to_string(),
            matched_keywords: Vec::new(),
            classifier: "forced".to_string(),
        },
        "auto" => {
            if let Some((mode, matched)) = heuristic_route(user_question) {
                RouteDecision {
                    mode,
                    reason: format!("heuristic matched: {}", matched.join(", ")),
                    matched_keywords: matched,
                    classifier: "heuristic".to_string(),
                }
            } else {
                RouteDecision {
                    mode: RouteMode::RagFirst,
                    reason: "no tool-first keywords matched; RAG first".to_string(),
                    matched_keywords: Vec::new(),
                    classifier: "heuristic".to_string(),
                }
            }
        }
        _ => RouteDecision {
            mode: RouteMode::RagFirst,
            reason: "default: RAG first (set chat.routing_mode=auto to enable tool routing)".to_string(),
            matched_keywords: Vec::new(),
            classifier: "forced".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Routing heuristic ───────────────────────────────────────────────

    #[test]
    fn heuristic_routes_recency_queries_to_tools() {
        let (mode, matched) = heuristic_route("¿cuándo fue el último email de alice?").unwrap();
        assert_eq!(mode, RouteMode::ToolsFirst);
        assert!(matched.iter().any(|k| k == "último"));
    }

    #[test]
    fn heuristic_routes_listings_to_tools() {
        let (mode, _) = heuristic_route("pasame todas las facturas").unwrap();
        assert_eq!(mode, RouteMode::ToolsFirst);
    }

    #[test]
    fn heuristic_routes_counts_to_tools() {
        let (mode, _) = heuristic_route("cuantas propuestas a clientes he enviado en 2025").unwrap();
        assert_eq!(mode, RouteMode::ToolsFirst);
    }

    #[test]
    fn heuristic_routes_english_latest_to_tools() {
        let (mode, matched) = heuristic_route("show me the latest invoice").unwrap();
        assert_eq!(mode, RouteMode::ToolsFirst);
        assert!(matched.iter().any(|k| k == "latest"));
    }

    #[test]
    fn heuristic_returns_none_for_open_ended_questions() {
        assert!(heuristic_route("what does the team think about the proposal?").is_none());
        assert!(heuristic_route("resumen del kickoff").is_none());
    }

    #[test]
    fn heuristic_routes_draft_intent_to_tools() {
        // Writing/replying intent must run the tool loop so generate_email_draft
        // fires (saving a real draft + opening the composer) instead of RAG
        // free-writing the body. EN + ES, covering the reported regression.
        for q in [
            "escribe un borrador para contestar a dani de apple, pidiendo info del pedido",
            "redacta una respuesta para maria",
            "draft a reply to John about the invoice",
            "compose an email to billing@acme.com",
            "write a reply to the support thread",
        ] {
            let res = heuristic_route(q);
            assert!(res.is_some(), "expected ToolsFirst for: {}", q);
            assert_eq!(res.unwrap().0, RouteMode::ToolsFirst, "query: {}", q);
        }
    }

    #[test]
    fn heuristic_routes_year_mention_to_tools() {
        let (mode, matched) = heuristic_route("que entrevistas hice en 2018").unwrap();
        assert_eq!(mode, RouteMode::ToolsFirst);
        assert!(matched.iter().any(|k| k == "2018"));
    }

    #[test]
    fn heuristic_routes_iso_date_to_tools() {
        let (mode, _) = heuristic_route("emails del 2025-03-14").unwrap();
        assert_eq!(mode, RouteMode::ToolsFirst);
    }

    #[test]
    fn heuristic_routes_quarter_to_tools() {
        let (mode, _) = heuristic_route("facturas de Q3").unwrap();
        assert_eq!(mode, RouteMode::ToolsFirst);
    }

    #[test]
    fn heuristic_routes_relative_date_words_to_tools() {
        for q in [
            "emails de ayer",
            "lo que me llegó la semana pasada",
            "mes pasado de emailops",
            "emails from yesterday",
            "last week's invoices",
        ] {
            let res = heuristic_route(q);
            assert!(res.is_some(), "expected ToolsFirst for: {}", q);
            assert_eq!(res.unwrap().0, RouteMode::ToolsFirst, "query: {}", q);
        }
    }

    #[test]
    fn heuristic_routes_month_name_to_tools() {
        let (mode, _) = heuristic_route("propuestas de enero").unwrap();
        assert_eq!(mode, RouteMode::ToolsFirst);
        let (mode, _) = heuristic_route("invoices in december").unwrap();
        assert_eq!(mode, RouteMode::ToolsFirst);
    }
}
