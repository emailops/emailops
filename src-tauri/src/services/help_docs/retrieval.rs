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
/// Calibrated on the app_help eval and `help_docs_rank_probe` with
/// nomic-embed-text v1.5 (see [`plan_help_sources`] for the numbers and
/// the recall-over-precision trade-off). Overridable with the
/// `chat.help_min_similarity` preference.
pub const HELP_MIN_SIMILARITY: f32 = 0.60;
/// Sections handed to the model per turn. Two keep the block under ~900
/// tokens next to the mailbox sources.
pub const HELP_TOP_K: usize = 2;
/// Candidates fetched from each ranker before fusion.
const HELP_CANDIDATES: usize = 12;
/// RRF weights. FTS decides the order (see [`plan_help_sources`]); vectors
/// still break ties and keep a section both rankers agree on ahead.
const FTS_FUSION_WEIGHT: f32 = 2.0;
const VECTOR_FUSION_WEIGHT: f32 = 1.0;
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
    /// The guide page the query planner picked. `Some` skips the similarity
    /// gate (the planner already judged the question to be about the app),
    /// keeps only that page's sections and serves its intro first.
    pub page: Option<&'a str>,
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

/// Pure: which sections ride in the prompt, in which language, in what order.
///
/// Two signals with two jobs, measured on the app_help eval against the
/// bundled nomic model (`help_docs_rank_probe`):
///   - **Vector similarity says whether the question is about the app.**
///     The best cosine over all candidates is 0.67–0.83 for questions about
///     EmailOps; mailbox questions score 0.47–0.57, with two overlaps
///     ("summarize today's emails" 0.66, "list all my pending tasks" 0.60).
///     The gate at 0.60 chooses recall: a false positive only rides the
///     block into a turn whose prompt tells the model to ignore it (the
///     mirror eval case shows it does) and can never move the UI, because
///     navigation follows the answer's citation, not the gate; a false
///     negative is an app question answered from the mailbox. Similarity
///     does not rank the sections either — the section a human would point
///     at landed between rank 16 and 57 — so the gate is global (the best
///     candidate decides for the turn), never per section.
///   - **BM25 says which section.** The guides are small and curated, with a
///     heading per topic, and once the app's own name is dropped bm25 puts
///     the right section first or second. Sections are served in bm25
///     order; the fused RRF score (FTS weighted 2:1) only breaks ties and
///     orders sections FTS never matched.
///
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

    let mut kept: Vec<SectionGroup> = if let Some(page) = input.page {
        groups.into_values().filter(|g| g.page == page).collect()
    } else {
        let about_the_app = if input.vector_available {
            groups
                .values()
                .filter_map(|g| g.max_similarity)
                .any(|s| s >= input.min_similarity)
        } else {
            groups.values().any(|g| g.best_fts_rank == Some(0))
        };
        if !about_the_app {
            return Vec::new();
        }
        groups
            .into_values()
            .filter(|g| input.vector_available || g.best_fts_rank == Some(0))
            .collect()
    };
    // BM25 order first (rank 0 best; sections FTS never matched go last),
    // fused RRF as the tie-break. Fusion alone rewards agreement between
    // rankers, and with vectors this weak that promoted "present in both"
    // sections over the one bm25 had first (Lenses lost to "Turning it all
    // off" on the eval).
    // On a picked page the intro goes first: it carries the page outline.
    let intro_first = input.page.is_some();
    kept.sort_by(|a, b| {
        let ia = intro_first && a.section_index == 0;
        let ib = intro_first && b.section_index == 0;
        let fa = a.best_fts_rank.unwrap_or(usize::MAX);
        let fb = b.best_fts_rank.unwrap_or(usize::MAX);
        ib.cmp(&ia)
            .then_with(|| fa.cmp(&fb))
            .then_with(|| b.best_fused.total_cmp(&a.best_fused))
    });
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

/// Sections served from the page the planner picked: its intro (the page
/// outline) and its best section.
const HELP_PAGE_K: usize = 2;

/// Sections from the rest of the guides that join a picked page.
const HELP_ELSEWHERE_K: usize = 2;

/// Pure: the page's sources first, then the best sections from other pages.
/// The planner picks the wrong page now and then, and the global ranking is
/// what answered such a question before pages existed — the right section
/// is not always its first hit ("add an account" was the second), so two
/// ride along.
pub fn merge_page_and_global(on_page: Vec<HelpSource>, global: Vec<HelpSource>) -> Vec<HelpSource> {
    let page = on_page.first().map(|s| s.page.clone());
    let elsewhere: Vec<HelpSource> = global
        .into_iter()
        .filter(|g| Some(&g.page) != page.as_ref() && !on_page.iter().any(|s| s.chunk_id == g.chunk_id))
        .take(HELP_ELSEWHERE_K)
        .collect();
    on_page.into_iter().chain(elsewhere).collect()
}

/// Fetch, fuse and plan the help sources for `query`. Best-effort and
/// bounded: an error or timeout inside yields no sources plus a trace that
/// says so, never a failed turn. `query_embedding` is reused when the
/// mailbox retrieval already embedded the question this turn. `page` is the
/// guide page the query planner picked: its intro and best section ride
/// first, joined by the best section from the rest of the guides (see
/// [`merge_page_and_global`]).
pub async fn lookup_help(
    db: &Arc<Database>,
    provider: &dyn AIProvider,
    query: &str,
    query_embedding: Option<&[f32]>,
    ui_lang: &str,
    k: usize,
    page: Option<&str>,
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

    let embedding: Option<Vec<f32>> = if vector_available {
        match query_embedding {
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
        }
    } else {
        None
    };
    let ranker = SectionRanker {
        db,
        query,
        embedding: embedding.as_deref(),
        model: &model,
        ui_lang,
    };

    let sources = match page {
        None => ranker.rank(k, None, &mut trace)?,
        Some(p) => {
            let on_page = ranker.rank(HELP_PAGE_K, Some(p), &mut trace)?;
            // Enough that hits on the picked page, which are skipped, still
            // leave HELP_ELSEWHERE_K from other pages.
            let global = ranker.rank(HELP_PAGE_K + HELP_ELSEWHERE_K, None, &mut trace)?;
            merge_page_and_global(on_page, global)
        }
    };
    trace.included = sources.len() as i32;
    trace.chunk_ids = sources.iter().map(|s| s.chunk_id.clone()).collect();
    trace.elapsed_ms = t0.elapsed().as_millis() as i64;
    Ok((sources, trace))
}

/// One ranking pass over the corpus (or over one page of it): FTS + KNN,
/// fused, hydrated and handed to [`plan_help_sources`].
struct SectionRanker<'a> {
    db: &'a Arc<Database>,
    query: &'a str,
    embedding: Option<&'a [f32]>,
    model: &'a str,
    ui_lang: &'a str,
}

impl SectionRanker<'_> {
    fn rank(&self, k: usize, page: Option<&str>, trace: &mut HelpTrace) -> Result<Vec<HelpSource>> {
        let (db, ui_lang) = (self.db, self.ui_lang);
        let vec_hits: Vec<(i64, f32)> = match self.embedding {
            Some(e) => db.vec_search_help_docs(e, self.model, HELP_CANDIDATES, page)?,
            None => Vec::new(),
        };
        let fts_hits = db.fts_search_help_docs(self.query, HELP_CANDIDATES as i32, page)?;

        // On a picked page the intro rides even when neither ranker matched it.
        let page_intro: Vec<HelpChunk> = match page {
            Some(p) => db.get_help_chunk_siblings(&[(p.to_string(), 0)], ui_lang)?,
            None => Vec::new(),
        };

        if vec_hits.is_empty() && fts_hits.is_empty() && page_intro.is_empty() {
            return Ok(Vec::new());
        }

        // ── Fuse + hydrate ─────────────────────────────────────────────────
        let vec_ids: Vec<String> = vec_hits.iter().map(|(r, _)| r.to_string()).collect();
        let fts_ids: Vec<String> = fts_hits.iter().map(|(r, _)| r.to_string()).collect();
        let fused: BTreeMap<String, f32> = fuse_rrf(
            &[
                Ranking {
                    ids_in_order: &vec_ids,
                    weight: VECTOR_FUSION_WEIGHT,
                },
                Ranking {
                    ids_in_order: &fts_ids,
                    weight: FTS_FUSION_WEIGHT,
                },
            ],
            DEFAULT_RRF_K,
        )
        .into_iter()
        .collect();
        let rowids: Vec<i64> = fused.keys().filter_map(|k| k.parse().ok()).collect();
        let rows = db.get_help_chunks_by_rowids(&rowids)?;
        let mut candidates: Vec<HelpCandidate> = rows
            .into_iter()
            .map(|(rowid, chunk)| HelpCandidate {
                vec_similarity: vec_hits.iter().find(|(r, _)| *r == rowid).map(|(_, s)| *s),
                fts_rank: fts_hits.iter().position(|(r, _)| *r == rowid),
                fused: fused.get(&rowid.to_string()).copied().unwrap_or(0.0),
                chunk,
            })
            .collect();
        candidates.extend(page_intro.into_iter().map(|chunk| HelpCandidate {
            chunk,
            vec_similarity: None,
            fts_rank: None,
            fused: 0.0,
        }));
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

        trace.candidates += sections.len() as i32;
        let top = candidates.iter().filter_map(|c| c.vec_similarity).reduce(f32::max);
        trace.top_similarity = match (trace.top_similarity, top) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };

        Ok(plan_help_sources(HelpPlanInput {
            candidates: &candidates,
            siblings: &siblings,
            ui_lang,
            vector_available: !vec_hits.is_empty(),
            page,
            min_similarity: min_similarity(db),
            k,
        }))
    }
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
        plan_on_page(cands, sibs, lang, vector, None)
    }

    fn plan_on_page(
        cands: &[HelpCandidate],
        sibs: &[HelpChunk],
        lang: &str,
        vector: bool,
        page: Option<&str>,
    ) -> Vec<HelpSource> {
        plan_help_sources(HelpPlanInput {
            candidates: cands,
            siblings: sibs,
            ui_lang: lang,
            vector_available: vector,
            page,
            min_similarity: HELP_MIN_SIMILARITY,
            k: HELP_TOP_K,
        })
    }

    // ── Page picked by the planner ───────────────────────────────────────
    // The query planner names the guide page; bm25 and vectors only choose
    // sections inside it. The page intro carries the page outline, so it goes
    // first: it answers "what is on this page?" and frames the section after it.

    #[test]
    fn a_picked_page_serves_its_intro_first_then_its_best_section() {
        let cands = vec![
            cand(
                chunk("es", "installation", 2, 0, "Con IA local"),
                Some(0.62),
                Some(0),
                0.05,
            ),
            cand(chunk("es", "ai-features", 12, 0, "Lentes"), Some(0.55), Some(1), 0.04),
            cand(chunk("es", "ai-features", 0, 0, "Funciones de IA"), None, None, 0.0),
            cand(
                chunk("es", "ai-features", 13, 0, "Apagarlo todo"),
                Some(0.50),
                Some(2),
                0.03,
            ),
        ];
        let out = plan_on_page(&cands, &[], "es", true, Some("ai-features"));
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(ids, ["es/ai-features#0.0", "es/ai-features#12.0"]);
    }

    // The planner picks the wrong page now and then ("add an account" went to
    // installation), so the picked page is a preference, not a filter: its
    // intro and best section ride first, then the best section from anywhere
    // else — which is what answered "add an account" before pages existed.

    fn source(page: &str, section: i32) -> HelpSource {
        HelpSource::from_chunk(chunk("en", page, section, 0, &format!("{page} {section}")), 0.5)
    }

    #[test]
    fn a_picked_page_is_joined_by_the_two_best_sections_from_other_pages() {
        // "como añado una nueva cuenta": the planner picked installation, and
        // the right section was the SECOND global hit (the first was
        // "Something else"), so one global slot was not enough.
        let on_page = vec![source("installation", 0), source("installation", 12)];
        let global = vec![
            source("troubleshooting", 8),
            source("getting-started", 4),
            source("features", 1),
        ];
        let out = merge_page_and_global(on_page, global);
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "en/installation#0.0",
                "en/installation#12.0",
                "en/troubleshooting#8.0",
                "en/getting-started#4.0"
            ]
        );
    }

    #[test]
    fn the_global_pick_skips_the_picked_page() {
        // A global hit already on the page is not repeated and does not use
        // up a slot.
        let on_page = vec![source("ai-features", 0), source("ai-features", 1)];
        let global = vec![source("ai-features", 1), source("getting-started", 2)];
        let out = merge_page_and_global(on_page, global);
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(
            ids,
            ["en/ai-features#0.0", "en/ai-features#1.0", "en/getting-started#2.0"]
        );
    }

    #[test]
    fn nothing_global_leaves_the_page_sources() {
        let on_page = vec![source("features", 0), source("features", 6)];
        assert_eq!(merge_page_and_global(on_page.clone(), Vec::new()), on_page);
    }

    #[test]
    fn a_picked_page_skips_the_similarity_gate() {
        // The planner already said the question is about the app.
        let cands = vec![cand(
            chunk("en", "features", 6, 0, "Attachments view"),
            Some(0.31),
            Some(0),
            0.03,
        )];
        let out = plan_on_page(&cands, &[], "en", true, Some("features"));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_picked_page_keeps_its_sections_without_vectors() {
        let cands = vec![
            cand(chunk("en", "features", 0, 0, "Features"), None, None, 0.0),
            cand(chunk("en", "features", 6, 0, "Attachments view"), None, Some(1), 0.02),
        ];
        let out = plan_on_page(&cands, &[], "en", false, Some("features"));
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(ids, ["en/features#0.0", "en/features#6.0"]);
    }

    /// The gate is global: one candidate above the threshold means the
    /// question is about the app, and then the fused order decides which
    /// sections ride — a section's own similarity is not a filter (the right
    /// section often scores below the threshold, see the probe).
    #[test]
    fn gate_is_global_and_order_is_fused() {
        let cands = vec![
            cand(
                chunk("en", "ai-features", 1, 0, "Choosing a backend"),
                Some(0.71),
                Some(1),
                0.03,
            ),
            cand(
                chunk("en", "ai-features", 2, 0, "The model catalog"),
                Some(0.40),
                Some(0),
                0.05,
            ),
        ];
        let out = plan(&cands, &[], "en", true);
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(ids, ["en/ai-features#2.0", "en/ai-features#1.0"]);
        assert!((out[1].score - 0.71).abs() < 1e-6);
        assert!(
            (out[0].score - 0.40).abs() < 1e-6,
            "score reports the section's own similarity"
        );
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

    /// Fusion rewards agreement: a section on both lists outscores the one
    /// bm25 ranked first when that one is missing from the vector list. The
    /// bm25 order wins; fusion only breaks ties.
    #[test]
    fn bm25_order_beats_fused_agreement() {
        let cands = vec![
            cand(
                chunk("en", "ai-features", 13, 0, "Turning it all off"),
                Some(0.66),
                Some(1),
                0.048,
            ),
            cand(chunk("en", "ai-features", 12, 0, "Lenses"), None, Some(0), 0.033),
            cand(
                chunk("en", "getting-started", 1, 0, "AI on or off"),
                Some(0.63),
                Some(2),
                0.045,
            ),
        ];
        let out = plan(&cands, &[], "en", true);
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(ids, ["en/ai-features#12.0", "en/ai-features#13.0"]);
    }

    #[test]
    fn keeps_at_most_k_sections_by_fused_score() {
        let cands = vec![
            cand(chunk("en", "ai-features", 1, 0, "A"), Some(0.6), None, 0.01),
            cand(chunk("en", "ai-features", 2, 0, "B"), Some(0.9), Some(0), 0.05),
            cand(chunk("en", "ai-features", 3, 0, "C"), Some(0.7), Some(1), 0.03),
        ];
        let out = plan(&cands, &[], "en", true);
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(ids, ["en/ai-features#2.0", "en/ai-features#3.0"]);
    }

    /// From the ranking probe: "how do I turn on Lenses" scores 0.59 on the
    /// Lenses section (rank 16 by vector) and 0.66 on "Turning it all off";
    /// bm25 has Lenses first. The turn is about the app (0.66 ≥ 0.60), and
    /// the fused order — FTS weighted up — serves Lenses first.
    #[test]
    fn a_low_similarity_section_that_fts_ranks_first_is_served_first() {
        let cands = vec![
            cand(
                chunk("en", "ai-features", 13, 0, "Turning it all off"),
                Some(0.66),
                Some(1),
                0.030,
            ),
            cand(chunk("en", "ai-features", 12, 0, "Lenses"), Some(0.59), Some(0), 0.033),
            cand(
                chunk("en", "installation", 8, 0, "Direct download"),
                Some(0.67),
                None,
                0.016,
            ),
        ];
        let out = plan(&cands, &[], "en", true);
        let ids: Vec<&str> = out.iter().map(|s| s.chunk_id.as_str()).collect();
        assert_eq!(ids, ["en/ai-features#12.0", "en/ai-features#13.0"]);
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
            "summarize today's emails",
            "who do I know at Faro Logistics?",
            "send me Bahía Studio's May invoice",
            "list all my pending tasks",
            "what is my next meeting?",
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
                // Where the section a human would point at actually lands.
                let expected: Vec<String> = [
                    "en/ai-features#1.0",
                    "en/ai-features#2.0",
                    "en/privacy-security#1.0",
                    "en/troubleshooting#2.0",
                    "en/ai-features#12.0",
                ]
                .iter()
                .filter_map(|want| {
                    scored
                        .iter()
                        .position(|(_, id)| id == want)
                        .map(|pos| format!("{want}@{} {:.2}", pos + 1, scored[pos].0))
                })
                .collect();
                println!("{q}\n    {}\n    expected: {}", top.join(" | "), expected.join(" | "));
            }
        }
    }
}
