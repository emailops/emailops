//! Where the app should open after an answer that cites a guide section.
//!
//! A section's `nav:` target is a plain string in the docs front matter
//! (`settings/ai`, `view/calendar`). This module is the one place that
//! knows which tabs and views exist, so a typo in the docs fails a unit test
//! instead of a silent no-op in the UI — and the pure planner that decides
//! whether a finished answer should navigate at all.

use super::retrieval::HelpSource;

/// Tabs of the Settings dialog. Mirrors `SettingsTab` in
/// `src/components/Settings/SettingsDialog.tsx`; keep the two in sync.
pub const SETTINGS_TABS: &[&str] = &[
    "appearance",
    "calendar",
    "ai",
    "classification",
    "junk",
    "tasks",
    "memory",
    "lenses",
    "aidrafts",
    "aitranslation",
    "aisearch",
    "privacy",
];

/// Main-view modes the effect may switch to. Mirrors the fixed members of
/// `ViewMode` in `src/components/Sidebar/Sidebar.tsx` (the `folder:` family
/// needs an id and is deliberately not navigable from a guide).
pub const VIEWS: &[&str] = &[
    "inbox",
    "attachments",
    "contacts",
    "drafts",
    "sent",
    "spam",
    "deleted",
    "calendar",
    "chat",
    "tasks",
    "memory",
    "lenses",
    "tagboard",
    "dashboard",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavTarget {
    Settings(String),
    View(String),
}

impl NavTarget {
    /// The wire form (`settings/ai`, `view/calendar`) — what the docs write
    /// and what the frontend effect handler receives.
    pub fn as_wire(&self) -> String {
        match self {
            NavTarget::Settings(tab) => format!("settings/{tab}"),
            NavTarget::View(view) => format!("view/{view}"),
        }
    }
}

/// Parse a `nav:` target. `None` for anything that is not a known tab or
/// view, so a docs typo can never reach the UI.
pub fn parse_nav_target(raw: &str) -> Option<NavTarget> {
    let (kind, name) = raw.trim().split_once('/')?;
    match kind {
        "settings" if SETTINGS_TABS.contains(&name) => Some(NavTarget::Settings(name.to_string())),
        "view" if VIEWS.contains(&name) => Some(NavTarget::View(name.to_string())),
        _ => None,
    }
}

/// Decide whether a finished answer should move the UI, and where.
///
/// The rule is the model's own citation: the first help source, in citation
/// order, whose `help://` link appears in the answer and which carries a
/// navigable target wins. An answer that did not cite the guides (the
/// question was about the mailbox after all) never navigates, whatever the
/// lookup offered — the user's screen must not change on a false positive
/// of the similarity gate.
pub fn plan_help_navigation<'a>(answer: &str, sources: &'a [HelpSource]) -> Option<(NavTarget, &'a HelpSource)> {
    sources.iter().find_map(|s| {
        if !answer.contains(&s.link) {
            return None;
        }
        let target = parse_nav_target(s.nav_target.as_deref()?)?;
        Some((target, s))
    })
}

/// Deterministic citation fallback for an answer that came from the help
/// block but forgot the link. Small local models drop the trailing
/// `[title](help://…)` link on some runs (seen on 2 of 6 eval cases), and
/// without it neither the docs chip nor the navigation can fire.
///
/// The rule is conservative: the turn offered help sources, ran no tool,
/// and the answer cites nothing else — no `help://`, no `email://`, no
/// `[n]` marker. An answer with any mailbox grounding is left alone, so a
/// gate false positive can never make a mailbox answer navigate.
pub fn plan_help_link_fallback<'a>(
    answer: &str,
    sources: &'a [HelpSource],
    tool_calls: usize,
) -> Option<&'a HelpSource> {
    let top = sources.first()?;
    if tool_calls > 0 || answer.trim().is_empty() {
        return None;
    }
    if answer.contains("help://") || answer.contains("email://") || answer.contains("draft://") {
        return None;
    }
    if has_numeric_citation(answer) {
        return None;
    }
    Some(top)
}

fn has_numeric_citation(answer: &str) -> bool {
    let b = answer.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'[' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < b.len() && b[j] == b']' {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// The line appended by the fallback: the same shape the prompt asks for.
pub fn help_link_line(source: &HelpSource) -> String {
    format!("[{}]({})", source.title(), source.link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::HelpChunk;

    fn source(link: &str, nav: Option<&str>) -> HelpSource {
        HelpSource::from_chunk(
            HelpChunk {
                chunk_id: link.trim_start_matches("help://").to_string(),
                lang: "en".into(),
                page: "ai-features".into(),
                section_index: 1,
                part: 0,
                anchor: link.rsplit('#').next().unwrap_or("").to_string(),
                page_title: "AI features".into(),
                heading: "Choosing a backend".into(),
                content: "text".into(),
                nav_target: nav.map(str::to_string),
            },
            0.9,
        )
    }

    #[test]
    fn parses_known_settings_tabs_and_views() {
        assert_eq!(parse_nav_target("settings/ai"), Some(NavTarget::Settings("ai".into())));
        assert_eq!(
            parse_nav_target("view/calendar"),
            Some(NavTarget::View("calendar".into()))
        );
        assert_eq!(
            parse_nav_target(" view/tagboard "),
            Some(NavTarget::View("tagboard".into()))
        );
    }

    #[test]
    fn rejects_unknown_targets() {
        assert_eq!(parse_nav_target("settings/nope"), None);
        assert_eq!(parse_nav_target("view/folder:x"), None);
        assert_eq!(parse_nav_target("dialog/ai"), None);
        assert_eq!(parse_nav_target("settings"), None);
        assert_eq!(parse_nav_target(""), None);
    }

    #[test]
    fn wire_form_round_trips() {
        for raw in ["settings/privacy", "view/inbox"] {
            assert_eq!(parse_nav_target(raw).map(|t| t.as_wire()), Some(raw.to_string()));
        }
    }

    #[test]
    fn every_nav_target_in_the_corpus_is_valid() {
        for c in super::super::corpus::corpus() {
            if let Some(t) = &c.nav_target {
                assert!(parse_nav_target(t).is_some(), "{}: bad nav target {t:?}", c.chunk_id);
            }
        }
    }

    #[test]
    fn navigates_to_the_first_cited_source_with_a_target() {
        let sources = vec![
            source("help://en/ai-features#the-model-catalog", None),
            source("help://en/ai-features#choosing-a-backend", Some("settings/ai")),
            source("help://en/features#calendar", Some("view/calendar")),
        ];
        let answer = "Open Settings, see [Choosing a backend](help://en/ai-features#choosing-a-backend) \
                      and [Calendar](help://en/features#calendar).";
        let (target, src) = plan_help_navigation(answer, &sources).expect("navigates");
        assert_eq!(target, NavTarget::Settings("ai".into()));
        assert_eq!(src.link, "help://en/ai-features#choosing-a-backend");
    }

    #[test]
    fn does_not_navigate_when_the_answer_cites_nothing() {
        let sources = vec![source("help://en/ai-features#choosing-a-backend", Some("settings/ai"))];
        assert!(plan_help_navigation("Marisol wrote on Tuesday [1].", &sources).is_none());
    }

    #[test]
    fn does_not_navigate_for_a_cited_source_without_target() {
        let sources = vec![source("help://en/ai-features#the-model-catalog", None)];
        let answer = "See [the catalog](help://en/ai-features#the-model-catalog).";
        assert!(plan_help_navigation(answer, &sources).is_none());
    }

    #[test]
    fn ignores_an_invalid_target_even_when_cited() {
        let sources = vec![source(
            "help://en/ai-features#choosing-a-backend",
            Some("settings/bogus"),
        )];
        let answer = "See [x](help://en/ai-features#choosing-a-backend).";
        assert!(plan_help_navigation(answer, &sources).is_none());
    }

    #[test]
    fn fallback_links_the_top_source_when_nothing_is_cited() {
        let sources = vec![
            source("help://en/ai-features#choosing-a-backend", Some("settings/ai")),
            source("help://en/troubleshooting#chat-is-slow", None),
        ];
        let picked = plan_help_link_fallback("Open Settings → AI Backend & Models and pick Ollama.", &sources, 0)
            .expect("fallback");
        assert_eq!(picked.link, "help://en/ai-features#choosing-a-backend");
        assert_eq!(
            help_link_line(picked),
            "[AI features › Choosing a backend](help://en/ai-features#choosing-a-backend)"
        );
    }

    #[test]
    fn fallback_stays_out_when_the_answer_is_grounded_elsewhere() {
        let sources = vec![source("help://en/ai-features#choosing-a-backend", Some("settings/ai"))];
        assert!(plan_help_link_fallback("Marisol wrote on Tuesday [1].", &sources, 0).is_none());
        assert!(plan_help_link_fallback("See [her mail](email://eml-1).", &sources, 0).is_none());
        assert!(plan_help_link_fallback("Draft saved [Re: x](draft://d-1).", &sources, 0).is_none());
        assert!(plan_help_link_fallback("Already [linked](help://en/cli).", &sources, 0).is_none());
        assert!(
            plan_help_link_fallback("Found 3 emails.", &sources, 1).is_none(),
            "a tool ran"
        );
        assert!(plan_help_link_fallback("   ", &sources, 0).is_none());
        assert!(plan_help_link_fallback("anything", &[], 0).is_none(), "no help offered");
    }
}
