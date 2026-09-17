//! The bundled user guides as a retrieval corpus.
//!
//! `docs/site/<lang>/<page>.md` — the same markdown the public docs site is
//! built from — is embedded in the binary with `include_str!` and split into
//! one chunk per heading section (long sections into several parts). Pure:
//! no I/O beyond the compile-time embed, so every rule here is unit-tested
//! against real pages.
//!
//! Why sections and not pages: a page is 1–2k words, far more than a chat
//! turn can afford in its prompt, while a section is the unit a reader would
//! be pointed at ("Settings → AI Backend & Models", "Where your data is
//! stored") and the unit the docs site anchors.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::models::HelpChunk;

/// Site languages, in the order `docs/site/README.md` lists them.
pub const LANGS: &[&str] = &["en", "es", "fr", "de"];

/// Pages indexed, by filename. `_index.md` is the landing blurb and carries
/// no headings, so it is left out.
pub const PAGES: &[&str] = &[
    "getting-started",
    "features",
    "ai-features",
    "privacy-security",
    "installation",
    "troubleshooting",
    "cli",
];

/// Sections longer than this (in chars) are split into parts at paragraph
/// boundaries. ~450 tokens: two of them fit in a turn without crowding the
/// mailbox sources (`TOP_K_SOURCES` × `MAX_SOURCE_BODY_CHARS`).
pub const MAX_CHUNK_CHARS: usize = 1800;

macro_rules! page {
    ($lang:literal, $page:literal) => {
        (
            $lang,
            $page,
            include_str!(concat!("../../../../docs/site/", $lang, "/", $page, ".md")),
        )
    };
}

/// Every `(lang, page, markdown)` the binary carries. Kept as a literal table
/// (not a loop over `LANGS × PAGES`) because `include_str!` needs literals.
fn raw_pages() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        page!("en", "getting-started"),
        page!("en", "features"),
        page!("en", "ai-features"),
        page!("en", "privacy-security"),
        page!("en", "installation"),
        page!("en", "troubleshooting"),
        page!("en", "cli"),
        page!("es", "getting-started"),
        page!("es", "features"),
        page!("es", "ai-features"),
        page!("es", "privacy-security"),
        page!("es", "installation"),
        page!("es", "troubleshooting"),
        page!("es", "cli"),
        page!("fr", "getting-started"),
        page!("fr", "features"),
        page!("fr", "ai-features"),
        page!("fr", "privacy-security"),
        page!("fr", "installation"),
        page!("fr", "troubleshooting"),
        page!("fr", "cli"),
        page!("de", "getting-started"),
        page!("de", "features"),
        page!("de", "ai-features"),
        page!("de", "privacy-security"),
        page!("de", "installation"),
        page!("de", "troubleshooting"),
        page!("de", "cli"),
    ]
}

/// The whole corpus, parsed once per process.
pub fn corpus() -> &'static [HelpChunk] {
    static CORPUS: OnceLock<Vec<HelpChunk>> = OnceLock::new();
    CORPUS.get_or_init(|| {
        raw_pages()
            .iter()
            .flat_map(|(lang, page, md)| parse_page(lang, page, md))
            .collect()
    })
}

/// Fingerprint of the corpus as compiled in: chunk ids, text and nav
/// targets. Stored next to the index so a rebuild happens exactly when the
/// guides shipped with the binary changed.
pub fn corpus_hash() -> String {
    let mut hasher = Sha256::new();
    for c in corpus() {
        hasher.update(c.chunk_id.as_bytes());
        hasher.update([0]);
        hasher.update(c.heading.as_bytes());
        hasher.update([0]);
        hasher.update(c.content.as_bytes());
        hasher.update([0]);
        hasher.update(c.nav_target.as_deref().unwrap_or("").as_bytes());
        hasher.update([0]);
    }
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The text a chunk is embedded and full-text indexed as: the page and
/// heading first, so a query that names the feature ("chat", "Ollama") lands
/// on the section even when its body never repeats the word.
pub fn embedding_text(chunk: &HelpChunk) -> String {
    if chunk.heading == chunk.page_title {
        format!("{}\n{}", chunk.page_title, chunk.content)
    } else {
        format!("{} › {}\n{}", chunk.page_title, chunk.heading, chunk.content)
    }
}

/// Parsed front matter: the title and the `nav:` map (anchor → target).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FrontMatter {
    pub title: String,
    pub nav: BTreeMap<String, String>,
}

/// Split `md` into its YAML front matter and body. The front matter is the
/// block between the leading `---` line and the next `---` line; a page
/// without one yields an empty [`FrontMatter`] and the whole text as body.
pub(crate) fn parse_front_matter(md: &str) -> (FrontMatter, &str) {
    let Some(rest) = md.strip_prefix("---\n").or_else(|| md.strip_prefix("---\r\n")) else {
        return (FrontMatter::default(), md);
    };
    let Some(end) = rest.find("\n---") else {
        return (FrontMatter::default(), md);
    };
    let block = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']);

    let mut fm = FrontMatter::default();
    let mut in_nav = false;
    for line in block.lines() {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if in_nav && indented {
            if let Some((k, v)) = line.trim().split_once(':') {
                let k = k.trim();
                let v = strip_quotes(v.trim());
                if !k.is_empty() && !v.is_empty() {
                    fm.nav.insert(k.to_string(), v.to_string());
                }
            }
            continue;
        }
        in_nav = false;
        if indented {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        match k.trim() {
            "title" => fm.title = strip_quotes(v.trim()).to_string(),
            "nav" => in_nav = v.trim().is_empty(),
            _ => {}
        }
    }
    (fm, body)
}

fn strip_quotes(v: &str) -> &str {
    let v = v.trim();
    if v.len() >= 2 && ((v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"'))) {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

/// A heading-delimited section of a page body, before part splitting.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RawSection {
    pub heading: String,
    /// `Some` when the heading carried an explicit `{#id}`.
    pub explicit_anchor: Option<String>,
    pub content: String,
}

/// Split a page body at `##`/`###`/`####` headings. The text before the
/// first heading becomes section 0 with an empty heading. Headings inside
/// fenced code blocks are text, not sections.
pub(crate) fn split_sections(body: &str) -> Vec<RawSection> {
    let mut sections: Vec<RawSection> = vec![RawSection {
        heading: String::new(),
        explicit_anchor: None,
        content: String::new(),
    }];
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if !in_fence {
            if let Some((heading, anchor)) = parse_heading(line) {
                sections.push(RawSection {
                    heading,
                    explicit_anchor: anchor,
                    content: String::new(),
                });
                continue;
            }
        }
        if let Some(last) = sections.last_mut() {
            last.content.push_str(line);
            last.content.push('\n');
        }
    }
    for s in &mut sections {
        s.content = s.content.trim().to_string();
    }
    sections
}

/// `## Heading text {#anchor}` → `("Heading text", Some("anchor"))`. Only
/// levels 2–4 count: the pages have no `#` title (it comes from the front
/// matter) and deeper levels are not used.
fn parse_heading(line: &str) -> Option<(String, Option<String>)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if !(2..=4).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    let mut text = rest.trim().to_string();
    let mut anchor = None;
    if let Some(open) = text.rfind("{#") {
        if text.ends_with('}') {
            anchor = Some(text[open + 2..text.len() - 1].trim().to_string());
            text = text[..open].trim_end().to_string();
        }
    }
    Some((text, anchor))
}

/// Hugo-style heading id: lowercase, non-alphanumerics dropped, whitespace
/// and hyphens collapsed to one `-`. Letters outside ASCII are kept, as Hugo
/// keeps them ("fonctionnalités").
pub(crate) fn slugify(heading: &str) -> String {
    let mut out = String::with_capacity(heading.len());
    let mut pending_dash = false;
    for ch in heading.chars() {
        if ch.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else if ch.is_whitespace() || ch == '-' || ch == '_' {
            pending_dash = true;
        }
    }
    out
}

/// Split `content` into pieces of at most `max_chars` characters at blank
/// lines, never inside a paragraph. A paragraph longer than the cap stays
/// whole (a table is one paragraph) — the cap is a budget, not a guarantee.
pub(crate) fn split_parts(content: &str, max_chars: usize) -> Vec<String> {
    if content.chars().count() <= max_chars {
        return vec![content.to_string()];
    }
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for para in content.split("\n\n") {
        let para = para.trim_end();
        if para.is_empty() {
            continue;
        }
        let len = para.chars().count();
        if !current.is_empty() && current_len + 2 + len > max_chars {
            parts.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if !current.is_empty() {
            current.push_str("\n\n");
            current_len += 2;
        }
        current.push_str(para);
        current_len += len;
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Parse one page into chunks. Section 0 (the intro) is emitted only when
/// it has text, but always consumes index 0 so numbering matches across
/// languages.
pub(crate) fn parse_page(lang: &str, page: &str, md: &str) -> Vec<HelpChunk> {
    let (fm, body) = parse_front_matter(md);
    let mut chunks = Vec::new();
    for (section_index, section) in split_sections(body).into_iter().enumerate() {
        if section.content.is_empty() {
            continue;
        }
        let is_intro = section_index == 0;
        let anchor = match (&section.explicit_anchor, is_intro) {
            (Some(a), _) => a.clone(),
            (None, true) => String::new(),
            (None, false) => slugify(&section.heading),
        };
        let heading = if is_intro {
            fm.title.clone()
        } else {
            section.heading.clone()
        };
        let nav_target = section.explicit_anchor.as_ref().and_then(|a| fm.nav.get(a)).cloned();
        for (part, text) in split_parts(&section.content, MAX_CHUNK_CHARS).into_iter().enumerate() {
            chunks.push(HelpChunk {
                chunk_id: format!("{lang}/{page}#{section_index}.{part}"),
                lang: lang.to_string(),
                page: page.to_string(),
                section_index: section_index as i32,
                part: part as i32,
                anchor: anchor.clone(),
                page_title: fm.title.clone(),
                heading: heading.clone(),
                content: text,
                nav_target: nav_target.clone(),
            });
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashSet};

    const SAMPLE: &str = "---\ntitle: 'AI features'\ndescription: 'x'\nweight: 40\nnav:\n  choosing-a-backend: settings/ai\n  tag-board: view/tagboard\n---\n\nIntro paragraph.\n\n## Choosing a backend {#choosing-a-backend}\n\nSettings → AI.\n\n```bash\n## not a heading\n```\n\n### The model catalog {#the-model-catalog}\n\nA table.\n\n## Tag Board {#tag-board}\n\nBoard text.\n\n## Turning it all off\n\nMaster switch.\n";

    #[test]
    fn front_matter_yields_title_and_nav_map() {
        let (fm, body) = parse_front_matter(SAMPLE);
        assert_eq!(fm.title, "AI features");
        assert_eq!(
            fm.nav.get("choosing-a-backend").map(String::as_str),
            Some("settings/ai")
        );
        assert_eq!(fm.nav.get("tag-board").map(String::as_str), Some("view/tagboard"));
        assert_eq!(fm.nav.len(), 2);
        assert!(body.starts_with("Intro paragraph."), "body={body:?}");
    }

    #[test]
    fn front_matter_without_nav_block_is_empty_map() {
        let (fm, _) = parse_front_matter("---\ntitle: \"Docs\"\nweight: 1\n---\nhello");
        assert_eq!(fm.title, "Docs");
        assert!(fm.nav.is_empty());
    }

    #[test]
    fn page_without_front_matter_is_all_body() {
        let (fm, body) = parse_front_matter("just text");
        assert_eq!(fm, FrontMatter::default());
        assert_eq!(body, "just text");
    }

    #[test]
    fn sections_split_on_headings_and_keep_intro_as_zero() {
        let (_, body) = parse_front_matter(SAMPLE);
        let sections = split_sections(body);
        assert_eq!(sections.len(), 5);
        assert_eq!(sections[0].heading, "");
        assert_eq!(sections[0].content, "Intro paragraph.");
        assert_eq!(sections[1].heading, "Choosing a backend");
        assert_eq!(sections[1].explicit_anchor.as_deref(), Some("choosing-a-backend"));
        assert_eq!(sections[4].heading, "Turning it all off");
        assert_eq!(sections[4].explicit_anchor, None);
    }

    #[test]
    fn headings_inside_code_fences_are_text() {
        let (_, body) = parse_front_matter(SAMPLE);
        let sections = split_sections(body);
        assert!(sections[1].content.contains("## not a heading"));
        assert!(!sections.iter().any(|s| s.heading == "not a heading"));
    }

    #[test]
    fn slugify_matches_hugo_anchor_style() {
        assert_eq!(slugify("Elegir un backend"), "elegir-un-backend");
        assert_eq!(slugify("1. AI on or off"), "1-ai-on-or-off");
        assert_eq!(slugify("Command line (emailops-cli)"), "command-line-emailops-cli");
        assert_eq!(slugify("Fonctionnalités  IA"), "fonctionnalités-ia");
        assert_eq!(slugify("  Turning it all off "), "turning-it-all-off");
    }

    #[test]
    fn parse_page_numbers_sections_and_applies_nav_targets() {
        let chunks = parse_page("en", "ai-features", SAMPLE);
        let ids: Vec<&str> = chunks.iter().map(|c| c.chunk_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "en/ai-features#0.0",
                "en/ai-features#1.0",
                "en/ai-features#2.0",
                "en/ai-features#3.0",
                "en/ai-features#4.0"
            ]
        );
        assert_eq!(chunks[0].heading, "AI features");
        assert_eq!(chunks[0].anchor, "");
        assert_eq!(chunks[1].nav_target.as_deref(), Some("settings/ai"));
        assert_eq!(chunks[2].nav_target, None);
        assert_eq!(chunks[3].nav_target.as_deref(), Some("view/tagboard"));
        assert_eq!(chunks[4].anchor, "turning-it-all-off");
        assert_eq!(chunks[4].nav_target, None);
        assert_eq!(chunks[4].content, "Master switch.");
    }

    #[test]
    fn empty_intro_still_consumes_index_zero() {
        let chunks = parse_page("es", "p", "---\ntitle: 'T'\n---\n\n## A\n\ntext\n");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_id, "es/p#1.0");
    }

    #[test]
    fn long_sections_split_into_parts_at_paragraphs() {
        let para = "x".repeat(700);
        let content = format!("{para}\n\n{para}\n\n{para}");
        let parts = split_parts(&content, 1500);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].chars().count(), 700 * 2 + 2);
        assert_eq!(parts[1].chars().count(), 700);
        assert_eq!(split_parts("short", 1500), vec!["short".to_string()]);
    }

    #[test]
    fn parts_share_section_anchor_and_get_own_ids() {
        let big = "y".repeat(MAX_CHUNK_CHARS - 10);
        let md = format!("---\ntitle: 'T'\n---\n\n## Long {{#long}}\n\n{big}\n\n{big}\n");
        let chunks = parse_page("en", "p", &md);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chunk_id, "en/p#1.0");
        assert_eq!(chunks[1].chunk_id, "en/p#1.1");
        assert!(chunks.iter().all(|c| c.anchor == "long" && c.section_index == 1));
    }

    #[test]
    fn embedding_text_prefixes_page_and_heading() {
        let chunks = parse_page("en", "ai-features", SAMPLE);
        assert!(embedding_text(&chunks[1]).starts_with("AI features › Choosing a backend\n"));
        assert!(embedding_text(&chunks[0]).starts_with("AI features\n"));
    }

    // ── The real corpus ─────────────────────────────────────────────────────

    #[test]
    fn corpus_covers_every_language_and_page() {
        let have: BTreeSet<(String, String)> = corpus().iter().map(|c| (c.lang.clone(), c.page.clone())).collect();
        for lang in LANGS {
            for page in PAGES {
                assert!(
                    have.contains(&(lang.to_string(), page.to_string())),
                    "missing {lang}/{page}"
                );
            }
        }
        assert_eq!(have.len(), LANGS.len() * PAGES.len());
    }

    #[test]
    fn corpus_chunk_ids_are_unique_and_non_empty() {
        let mut seen = HashSet::new();
        for c in corpus() {
            assert!(seen.insert(c.chunk_id.clone()), "duplicate {}", c.chunk_id);
            assert!(!c.content.trim().is_empty(), "{} is empty", c.chunk_id);
            assert!(!c.page_title.is_empty(), "{} has no page title", c.chunk_id);
        }
    }

    /// The language swap relies on section N meaning the same thing in every
    /// language. `scripts/check-docs-parity.sh` guards the anchors; this
    /// guards the count, on the compiled-in text.
    #[test]
    fn section_numbering_is_identical_across_languages() {
        for page in PAGES {
            let per_lang: Vec<BTreeSet<i32>> = LANGS
                .iter()
                .map(|lang| {
                    corpus()
                        .iter()
                        .filter(|c| c.lang == *lang && c.page == *page)
                        .map(|c| c.section_index)
                        .collect()
                })
                .collect();
            for (i, set) in per_lang.iter().enumerate() {
                assert_eq!(
                    set, &per_lang[0],
                    "{page}: section set differs between en and {}",
                    LANGS[i]
                );
            }
        }
    }

    /// Explicit anchors are language-invariant by the docs' own rule, so the
    /// nav map keyed on them must be too: every language navigates the same
    /// section to the same place.
    #[test]
    fn nav_targets_are_identical_across_languages() {
        for page in PAGES {
            let per_lang: Vec<BTreeMap<(i32, String), String>> = LANGS
                .iter()
                .map(|lang| {
                    corpus()
                        .iter()
                        .filter(|c| c.lang == *lang && c.page == *page && c.part == 0)
                        .filter_map(|c| c.nav_target.clone().map(|t| ((c.section_index, c.anchor.clone()), t)))
                        .collect()
                })
                .collect();
            for (i, map) in per_lang.iter().enumerate() {
                assert_eq!(
                    map, &per_lang[0],
                    "{page}: nav targets differ between en and {}",
                    LANGS[i]
                );
            }
        }
    }

    #[test]
    fn corpus_has_navigable_sections() {
        let navigable = corpus().iter().filter(|c| c.nav_target.is_some()).count();
        assert!(navigable > 0, "no section carries a nav: target");
    }

    #[test]
    fn corpus_hash_is_stable_within_a_process() {
        assert_eq!(corpus_hash(), corpus_hash());
        assert_eq!(corpus_hash().len(), 64);
    }
}
