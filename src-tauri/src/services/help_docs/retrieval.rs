//! Look up the guide sections relevant to a chat question.
//!
//! Executor: FTS5 + vector KNN over `help_doc_chunks`, fused with the same
//! RRF the mailbox retrieval uses. Planner ([`plan_help_sources`], pure):
//! collapse hits to one per section, gate them on vector similarity so a
//! mailbox question does not drag a guide into the prompt, and serve every
//! kept section in the language the answer will be written in — a hit on the
//! French page is swapped for its Spanish sibling (same page, same section
//! index) when the user chats in Spanish.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;

use crate::ai::provider::AIProvider;
use crate::db::Database;
use crate::models::error::Result;
use crate::models::{HelpChunk, HelpTrace};
use crate::services::retrieval::{fuse_rrf, Ranking, DEFAULT_RRF_K};

use super::index::embedding_model_label;

/// Cosine similarity a section must reach (against the question) to ride in
/// the prompt. Below it the question is about the mailbox, not the app.
/// Calibrated on the app_help eval with nomic-embed-text v1.5: questions
/// about the app scored 0.67–0.83 on their best section, a mailbox question
/// about a demo thread scored 0.57. Overridable with the
/// `chat.help_min_similarity` preference.
pub const HELP_MIN_SIMILARITY: f32 = 0.60;
/// Sections handed to the model per turn. Two keep the block under ~900
/// tokens next to the mailbox sources.
pub const HELP_TOP_K: usize = 2;
/// Candidates fetched from each ranker before fusion.
const HELP_CANDIDATES: usize = 12;
/// Wall-clock cap for the whole lookup (embedding the query included). The
/// help block is optional: past this the turn proceeds without it.
const HELP_LOOKUP_TIMEOUT: Duration = Duration::from_secs(4);

/// A guide section as the prompt and the navigation planner see it.
#[derive(Debug, Clone, PartialEq)]
pub struct HelpSource {
    pub chunk_id: String,
    pub lang: String,
    pub page: String,
    pub section_index: i32,
    pub anchor: String,
    pub page_title: String,
    pub heading: String,
    pub content: String,
    pub nav_target: Option<String>,
    /// Vector similarity when available, otherwise the fused RRF score.
    pub score: f32,
    /// `help://<lang>/<page>#<anchor>` — the link the answer cites, which
    /// the UI turns into the public docs URL.
    pub link: String,
}

impl HelpSource {
    pub fn from_chunk(c: HelpChunk, score: f32) -> Self {
        let link = help_link(&c.lang, &c.page, &c.anchor);
        Self {
            chunk_id: c.chunk_id,
            lang: c.lang,
            page: c.page,
            section_index: c.section_index,
            anchor: c.anchor,
            page_title: c.page_title,
            heading: c.heading,
            content: c.content,
            nav_target: c.nav_target,
            score,
            link,
        }
    }

    /// "Page › Heading", or just the page title for an intro chunk.
    pub fn title(&self) -> String {
        if self.heading == self.page_title || self.heading.is_empty() {
            self.page_title.clone()
        } else {
            format!("{} › {}", self.page_title, self.heading)
        }
    }
}

/// The `help://` link for a section. No fragment for the page intro.
pub fn help_link(lang: &str, page: &str, anchor: &str) -> String {
    if anchor.is_empty() {
        format!("help://{lang}/{page}")
    } else {
        format!("help://{lang}/{page}#{anchor}")
    }
}

/// One chunk that came back from FTS and/or vector search.
#[derive(Debug, Clone, PartialEq)]
pub struct HelpCandidate {
    pub chunk: HelpChunk,
    pub vec_similarity: Option<f32>,
    /// 0-based rank in the FTS list, when FTS matched it.
    pub fts_rank: Option<usize>,
    pub fused: f32,
}

/// Everything the planner needs; assembled by [`lookup_help`].
#[derive(Debug)]
pub struct HelpPlanInput<'a> {
    pub candidates: &'a [HelpCandidate],
    /// Chunks in `ui_lang` for every `(page, section_index)` among the
    /// candidates, so a foreign-language hit can be served in the answer's
    /// language.
    pub siblings: &'a [HelpChunk],
    pub ui_lang: &'a str,
    /// False when no chunk is embedded for the active model (FTS-only).
    pub vector_available: bool,
    pub min_similarity: f32,
    pub k: usize,
}

#[derive(Debug)]
struct SectionGroup {
    page: String,
    section_index: i32,
    best_fused: f32,
    max_similarity: Option<f32>,
    best_fts_rank: Option<usize>,
    /// Index into `candidates` of the best-scoring chunk.
    best: usize,
}

/// Pure: which sections ride in the prompt, in which language, in what order
/// (best vector similarity first).
///
/// Gate: with vectors, a section needs `max_similarity ≥ min_similarity`.
/// Without vectors (corpus not embedded yet) only the top FTS hit passes,
/// because bm25 alone cannot tell "how do I connect Ollama" from a mail
/// that mentions Ollama.
pub fn plan_help_sources(input: HelpPlanInput<'_>) -> Vec<HelpSource> {
    let mut groups: BTreeMap<(String, i32), SectionGroup> = BTreeMap::new();
    for (i, c) in input.candidates.iter().enumerate() {
        let key = (c.chunk.page.clone(), c.chunk.section_index);
        let g = groups.entry(key).or_insert_with(|| SectionGroup {
            page: c.chunk.page.clone(),
            section_index: c.chunk.section_index,
            best_fused: f32::MIN,
            max_similarity: None,
            best_fts_rank: None,
            best: i,
        });
        if c.fused > g.best_fused {
            g.best_fused = c.fused;
            g.best = i;
        }
        g.max_similarity = match (g.max_similarity, c.vec_similarity) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        g.best_fts_rank = match (g.best_fts_rank, c.fts_rank) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    let mut kept: Vec<SectionGroup> = groups
        .into_values()
        .filter(|g| {
            if input.vector_available {
                g.max_similarity.is_some_and(|s| s >= input.min_similarity)
            } else {
                g.best_fts_rank == Some(0)
            }
        })
        .collect();
    // Order by the semantic signal. RRF fusion decides which sections are
    // candidates at all, but ranking by it let an FTS-only hit on a section
    // that merely repeats a query word outrank the section the question is
    // about; vector similarity is what the gate trusts, so it orders too.
    let rank = |g: &SectionGroup| g.max_similarity.unwrap_or(g.best_fused);
    kept.sort_by(|a, b| rank(b).total_cmp(&rank(a)));
    kept.truncate(input.k);

    kept.into_iter()
        .map(|g| {
            let best = &input.candidates[g.best];
            let score = g.max_similarity.unwrap_or(g.best_fused);
            // Serve the answer's language: a candidate already in it wins,
            // then the stored sibling (part 0 first), then the hit itself.
            let in_lang = input
                .candidates
                .iter()
                .filter(|c| c.chunk.page == g.page && c.chunk.section_index == g.section_index)
                .filter(|c| c.chunk.lang == input.ui_lang)
                .max_by(|a, b| a.fused.total_cmp(&b.fused))
                .map(|c| c.chunk.clone());
            let chunk = in_lang
                .or_else(|| {
                    input
                        .siblings
                        .iter()
                        .find(|s| {
                            s.lang == input.ui_lang
                                && s.page == g.page
                                && s.section_index == g.section_index
                                && s.part == best.chunk.part
                        })
                        .or_else(|| {
                            input.siblings.iter().find(|s| {
                                s.lang == input.ui_lang && s.page == g.page && s.section_index == g.section_index
                            })
                        })
                        .cloned()
                })
                .unwrap_or_else(|| best.chunk.clone());
            HelpSource::from_chunk(chunk, score)
        })
        .collect()
}

/// Similarity gate: the `chat.help_min_similarity` preference when it parses
/// as a number in `(0, 1]`, else [`HELP_MIN_SIMILARITY`].
pub fn min_similarity(db: &Database) -> f32 {
    db.get_preference("chat.help_min_similarity")
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| *v > 0.0 && *v <= 1.0)
        .unwrap_or(HELP_MIN_SIMILARITY)
}

/// Fetch, fuse and plan the help sources for `query`. Best-effort and
/// bounded: an error or timeout inside yields no sources plus a trace that
/// says so, never a failed turn. `query_embedding` is reused when the
/// mailbox retrieval already embedded the question this turn.
pub async fn lookup_help(
    db: &Arc<Database>,
    provider: &dyn AIProvider,
    query: &str,
    query_embedding: Option<&[f32]>,
    ui_lang: &str,
    k: usize,
) -> Result<(Vec<HelpSource>, HelpTrace)> {
    let t0 = std::time::Instant::now();
    let mut trace = HelpTrace {
        lang: ui_lang.to_string(),
        ..HelpTrace::default()
    };
    if db.count_help_doc_chunks()? == 0 {
        trace.elapsed_ms = t0.elapsed().as_millis() as i64;
        return Ok((Vec::new(), trace));
    }
    let model = embedding_model_label(db);
    let vector_available = db.count_help_chunks_embedded_with(&model)? > 0;
    trace.vector_available = vector_available;

    // ── Vector candidates ──────────────────────────────────────────────
    let vec_hits: Vec<(i64, f32)> = if vector_available {
        let owned: Option<Vec<f32>> = match query_embedding {
            Some(e) => Some(e.to_vec()),
            None => match timeout(HELP_LOOKUP_TIMEOUT, provider.embed(query)).await {
                Ok(Ok(r)) => Some(r.embedding),
                Ok(Err(e)) => {
                    log("warn", format!("help lookup: embedding failed ({e}); FTS only"));
                    None
                }
                Err(_) => {
                    log("warn", "help lookup: embedding timed out; FTS only");
                    None
                }
            },
        };
        match owned {
            Some(e) => db.vec_search_help_docs(&e, &model, HELP_CANDIDATES)?,
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    // ── FTS candidates ─────────────────────────────────────────────────
    let fts_hits = db.fts_search_help_docs(query, HELP_CANDIDATES as i32)?;

    if vec_hits.is_empty() && fts_hits.is_empty() {
        trace.elapsed_ms = t0.elapsed().as_millis() as i64;
        return Ok((Vec::new(), trace));
    }

    // ── Fuse + hydrate ─────────────────────────────────────────────────
    let vec_ids: Vec<String> = vec_hits.iter().map(|(r, _)| r.to_string()).collect();
    let fts_ids: Vec<String> = fts_hits.iter().map(|(r, _)| r.to_string()).collect();
    let fused: BTreeMap<String, f32> = fuse_rrf(
        &[
            Ranking {
                ids_in_order: &vec_ids,
                weight: 1.0,
            },
            Ranking {
                ids_in_order: &fts_ids,
                weight: 1.0,
            },
        ],
        DEFAULT_RRF_K,
    )
    .into_iter()
    .collect();
    let rowids: Vec<i64> = fused.keys().filter_map(|k| k.parse().ok()).collect();
    let rows = db.get_help_chunks_by_rowids(&rowids)?;
    let candidates: Vec<HelpCandidate> = rows
        .into_iter()
        .map(|(rowid, chunk)| HelpCandidate {
            vec_similarity: vec_hits.iter().find(|(r, _)| *r == rowid).map(|(_, s)| *s),
            fts_rank: fts_hits.iter().position(|(r, _)| *r == rowid),
            fused: fused.get(&rowid.to_string()).copied().unwrap_or(0.0),
            chunk,
        })
        .collect();
    let sections: Vec<(String, i32)> = {
        let mut s: Vec<(String, i32)> = candidates
            .iter()
            .map(|c| (c.chunk.page.clone(), c.chunk.section_index))
            .collect();
        s.sort();
        s.dedup();
        s
    };
    let siblings = db.get_help_chunk_siblings(&sections, ui_lang)?;

    trace.candidates = sections.len() as i32;
    trace.top_similarity = candidates.iter().filter_map(|c| c.vec_similarity).reduce(f32::max);

    let sources = plan_help_sources(HelpPlanInput {
        candidates: &candidates,
        siblings: &siblings,
        ui_lang,
        vector_available: vector_available && !vec_hits.is_empty(),
        min_similarity: min_similarity(db),
        k,
    });
    trace.included = sources.len() as i32;
    trace.chunk_ids = sources.iter().map(|s| s.chunk_id.clone()).collect();
    trace.elapsed_ms = t0.elapsed().as_millis() as i64;
    Ok((sources, trace))
}

fn log(level: &str, message: impl Into<String>) {
    crate::services::logger::log(level, "ai", message);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(lang: &str, page: &str, section: i32, part: i32, heading: &str) -> HelpChunk {
        HelpChunk {
            chunk_id: format!("{lang}/{page}#{section}.{part}"),
            lang: lang.into(),
            page: page.into(),
            section_index: section,
            part,
            anchor: heading.to_lowercase().replace(' ', "-"),
            page_title: "AI features".into(),
            heading: heading.into(),
            content: format!("{heading} body in {lang}"),
            nav_target: None,
        }
    }

    fn cand(chunk: HelpChunk, sim: Option<f32>, fts: Option<usize>, fused: f32) -> HelpCandidate {
        HelpCandidate {
            chunk,
            vec_similarity: sim,
            fts_rank: fts,
            fused,
        }
    }

    fn plan(cands: &[HelpCandidate], sibs: &[HelpChunk], lang: &str, vector: bool) -> Vec<HelpSource> {
        plan_help_sources(HelpPlanInput {
            candidates: cands,
            siblings: sibs,
            ui_lang: lang,
            vector_available: vector,
            min_similarity: HELP_MIN_SIMILARITY,
            k: HELP_TOP_K,
        })
    }

    #[test]
    fn gate_drops_sections_below_similarity() {
        let cands = vec![
            cand(
                chunk("en", "ai-features", 1, 0, "Choosing a backend"),
                Some(0.71),
                Some(0),
                0.03,
            ),
            cand(
                chunk("en", "ai-features", 2, 0, "The model catalog"),
                Some(0.40),
                Some(1),
                0.02,
            ),
        ];
        let out = plan(&cands, &[], "en", true);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].chunk_id, "en/ai-features#1.0");
        assert!((out[0].score - 0.71).abs() < 1e-6);
    }

    #[test]
    fn mailbox_question_yields_nothing() {
        let cands = vec![cand(
            chunk("en", "features", 5, 0, "Calendar"),
            Some(0.31),
            Some(0),
            0.03,
        )];
        assert!(plan(&cands, &[], "en", true).is_empty());
    }

    #[test]
    fn without_vectors_only_the_top_fts_hit_passes() {
        let cands = vec![
            cand(
                chunk("en", "ai-features", 2, 0, "The model catalog"),
                None,
                Some(1),
                0.015,
            ),
            cand(
                chunk("en", "ai-features", 1, 0, "Choosing a backend"),
                None,
                Some(0),
                0.016,
            ),
        ];
        let out = plan(&cands, &[], "en", false);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].chunk_id, "en/ai-features#1.0");
    }

    #[test]
    fn a_foreign_hit_is_served_in_the_ui_language_via_sibling() {
        let cands = vec![cand(
            chunk("fr", "ai-features", 1, 0, "Choisir un backend"),
            Some(0.8),
            None,
            0.03,
        )];
        let sibs = vec![chunk("es", "ai-features", 1, 0, "Elegir un backend")];
        let out = plan(&cands, &sibs, "es", true);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].lang, "es");
        assert_eq!(out[0].link, "help://es/ai-features#elegir-un-backend");
        assert!(
            (out[0].score - 0.8).abs() < 1e-6,
            "score comes from the hit, not the sibling"
        );
    }

    #[test]
    fn same_section_in_several_languages_collapses_to_one_source() {
        let cands = vec![
            cand(
                chunk("fr", "ai-features", 1, 0, "Choisir un backend"),
                Some(0.8),
                None,
                0.03,
            ),
            cand(
                chunk("en", "ai-features", 1, 0, "Choosing a backend"),
                Some(0.78),
                Some(0),
                0.05,
            ),
            cand(
                chunk("de", "ai-features", 1, 0, "Backend wählen"),
                Some(0.7),
                None,
                0.02,
            ),
        ];
        let out = plan(&cands, &[], "en", true);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].lang, "en", "the candidate already in the UI language wins");
        assert!((out[0].score - 0.8).abs() < 1e-6, "best similarity across languages");
    }

    #[test]
    fn falls_back_to_the_hit_itself_when_no_sibling_exists() {
        let cands = vec![cand(chunk("fr", "cli", 1, 0, "Installer"), Some(0.9), None, 0.03)];
        let out = plan(&cands, &[], "de", true);
        assert_eq!(out[0].lang, "fr");
    }

    #[test]
    fn keeps_at_most_k_sections() {
        let cands = vec![
            cand(chunk("en", "ai-features", 1, 0, "A"), Some(0.6), None, 0.01),
            cand(chunk("en", "ai-features", 2, 0, "B"), Some(0.9), Some(0), 0.05),
            cand(chunk("en", "ai-features", 3, 0, "C"), Some(0.7), Some(1), 0.03),
        ];
        let out = plan(&cands, &[], "en", true);
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(ids, ["en/ai-features#2.0", "en/ai-features#3.0"]);
    }

    /// Regression from the first eval run: with the sections ordered by the
    /// fused RRF score, an FTS-only hit on a section that merely repeats the
    /// app's name outranked the section the question was about (0.9 cosine).
    /// Vector similarity is the semantic signal; FTS only supplies candidates.
    #[test]
    fn sections_are_ordered_by_similarity_not_fusion() {
        let cands = vec![
            cand(
                chunk("en", "privacy-security", 5, 0, "Local AI by default"),
                Some(0.62),
                Some(0),
                0.05,
            ),
            cand(
                chunk("en", "ai-features", 1, 0, "Choosing a backend"),
                Some(0.90),
                None,
                0.01,
            ),
            cand(
                chunk("en", "troubleshooting", 1, 0, "AI features are unavailable"),
                Some(0.75),
                Some(1),
                0.04,
            ),
        ];
        let out = plan(&cands, &[], "en", true);
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(ids, ["en/ai-features#1.0", "en/troubleshooting#1.0"]);
    }

    #[test]
    fn sibling_part_matches_the_hit_part() {
        let cands = vec![cand(chunk("en", "installation", 10, 1, "Linux"), Some(0.8), None, 0.03)];
        let sibs = vec![
            chunk("es", "installation", 10, 0, "Linux"),
            chunk("es", "installation", 10, 1, "Linux"),
        ];
        let out = plan(&cands, &sibs, "es", true);
        assert_eq!(out[0].chunk_id, "es/installation#10.1");
    }

    #[test]
    fn help_link_omits_fragment_for_intro() {
        assert_eq!(help_link("en", "cli", ""), "help://en/cli");
        assert_eq!(
            help_link("es", "ai-features", "chat-with-your-mailbox"),
            "help://es/ai-features#chat-with-your-mailbox"
        );
    }

    #[test]
    fn source_title_joins_page_and_heading() {
        let s = HelpSource::from_chunk(chunk("en", "ai-features", 1, 0, "Chat"), 0.5);
        assert_eq!(s.title(), "AI features › Chat");
        let mut intro = chunk("en", "ai-features", 0, 0, "AI features");
        intro.anchor.clear();
        let s = HelpSource::from_chunk(intro, 0.5);
        assert_eq!(s.title(), "AI features");
        assert_eq!(s.link, "help://en/ai-features");
    }

    #[test]
    fn min_similarity_pref_overrides_default_when_sane() {
        let db = Database::new_for_testing().unwrap();
        assert_eq!(min_similarity(&db), HELP_MIN_SIMILARITY);
        db.set_preference("chat.help_min_similarity", "0.7").unwrap();
        assert!((min_similarity(&db) - 0.7).abs() < 1e-6);
        db.set_preference("chat.help_min_similarity", "7").unwrap();
        assert_eq!(min_similarity(&db), HELP_MIN_SIMILARITY);
    }
}

/// Dev-only ranking probe: how well does the active embedding model rank
/// the guide sections for the app_help eval questions, with and without
/// nomic's task prefixes? Reads the demo data dir in `EMAILOPS_DATA_DIR`,
/// embeds the corpus twice in memory (no DB writes) and prints the top 3
/// per question. Run with:
///   EMAILOPS_DATA_DIR=$PWD/.emailops-demo-data cargo test --features cli,eval \
///     -- --ignored help_docs_rank_probe --nocapture
#[cfg(test)]
mod rank_probe {
    #[tokio::test]
    #[ignore = "needs the demo data dir with local models; run by hand"]
    async fn help_docs_rank_probe() {
        use crate::services::help_docs::corpus::{corpus, embedding_text};
        use std::sync::Arc;
        let Ok(dir) = std::env::var("EMAILOPS_DATA_DIR") else {
            eprintln!("EMAILOPS_DATA_DIR not set — skipping");
            return;
        };
        let db = Arc::new(crate::db::Database::new(std::path::PathBuf::from(dir)).unwrap());
        let provider = crate::services::ai::AiService::load_provider(&db).unwrap();
        let questions = [
            "how do I make EmailOps use my local Ollama instead of the built-in model?",
            "¿cómo cambio el modelo de IA que usa EmailOps?",
            "Wo speichert EmailOps meine Daten auf dem Rechner?",
            "pourquoi le chat d'EmailOps est-il si lent, et comment l'accélérer ?",
            "how do I turn on Lenses in EmailOps?",
            "what did Marisol say about the production bug?",
        ];
        let en: Vec<&crate::models::HelpChunk> = corpus().iter().filter(|c| c.lang == "en").collect();
        fn cos(a: &[f32], b: &[f32]) -> f32 {
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            dot / (na * nb)
        }
        for (doc_prefix, query_prefix) in [("", ""), ("search_document: ", "search_query: ")] {
            let texts: Vec<String> = en
                .iter()
                .map(|c| format!("{doc_prefix}{}", embedding_text(c)))
                .collect();
            let mut vecs = Vec::with_capacity(texts.len());
            for batch in texts.chunks(8) {
                for r in provider.embed_batch(batch).await.unwrap() {
                    vecs.push(r.embedding);
                }
            }
            println!("\n=== prefixes: doc={doc_prefix:?} query={query_prefix:?}");
            for q in questions {
                let qe = provider.embed(&format!("{query_prefix}{q}")).await.unwrap().embedding;
                let mut scored: Vec<(f32, &str)> = vecs
                    .iter()
                    .zip(en.iter())
                    .map(|(v, c)| (cos(&qe, v), c.chunk_id.as_str()))
                    .collect();
                scored.sort_by(|a, b| b.0.total_cmp(&a.0));
                let top: Vec<String> = scored.iter().take(3).map(|(s, id)| format!("{id} {s:.2}")).collect();
                println!("{q}\n    {}", top.join(" | "));
            }
        }
    }
}
