//! What the user is looking at, as one line of per-turn context.
//!
//! The chat panel is docked next to the app, so "esto", "aquí" and "añade una
//! columna" usually refer to whatever view is on screen. This module turns the
//! frontend's view token into a short, validated context line.
//!
//! Two invariants, both load-bearing:
//!
//! 1. **The line goes in the final user message, never the system prompt.**
//!    It changes every time the user navigates, so a system-prompt placement
//!    would change the cached anchor on every turn (`ColdPrefill` in
//!    `ai::llama_cpp::planner::plan_cached_prefix`) and throw away the
//!    KV-prefix cache. Same rule the Sources and OPEN EMAIL blocks follow.
//! 2. **An unknown token yields `None`.** The frontend is the only producer,
//!    but the value crosses the IPC boundary, so it is validated against the
//!    same typed allowlists the guides' `nav:` targets use — a token that is
//!    not a real view, settings tab or registered form never reaches a prompt.
//!
//! Pure: no I/O, no DB.

use crate::services::forms::registry as forms;
use crate::services::help_docs::nav::{parse_nav_target, NavTarget};

/// Where the user is, once validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewContext {
    /// A main view: `view/lenses`, `view/calendar`, …
    View(String),
    /// A Settings tab: `settings/ai`, …
    Settings(String),
    /// A fillable form is open on screen. Carries the registered form id.
    Form(&'static str),
}

impl ViewContext {
    /// The form the user has open, if any. This is what lets "añade una
    /// columna de IVA" resolve to a form without the user naming it.
    pub fn open_form_id(&self) -> Option<&'static str> {
        match self {
            ViewContext::Form(id) => Some(id),
            _ => None,
        }
    }
}

/// Parse the frontend's view token. `None` for anything unrecognised, so a
/// stale or hand-crafted token is simply ignored rather than prompted.
pub fn parse_view_context(raw: &str) -> Option<ViewContext> {
    let raw = raw.trim();
    if let Some(form_id) = raw.strip_prefix("form/") {
        // Resolve against the registry so the id in the prompt is always one
        // the filler can actually look up.
        return forms::lookup(form_id).map(|f| ViewContext::Form(f.id));
    }
    match parse_nav_target(raw)? {
        NavTarget::View(v) => Some(ViewContext::View(v)),
        NavTarget::Settings(t) => Some(ViewContext::Settings(t)),
    }
}

/// The one line prepended to the final user message.
///
/// Deliberately terse — it is paid for on every turn the user has something
/// open. It states the fact and the contract (use it only when the question is
/// about it), mirroring how the OPEN EMAIL block states both halves.
pub fn view_context_line(ctx: &ViewContext) -> String {
    match ctx {
        ViewContext::View(v) => format!(
            "CURRENT VIEW: the user is looking at the {v} view. Use this only to resolve \
             vague references like \"this\" or \"here\"; ignore it otherwise."
        ),
        ViewContext::Settings(tab) => format!(
            "CURRENT VIEW: the user has Settings › {tab} open. Use this only to resolve \
             vague references like \"this\" or \"here\"; ignore it otherwise."
        ),
        ViewContext::Form(id) => format!(
            "CURRENT VIEW: the user has the \"{id}\" form open on screen. A request to add, \
             change or remove a field refers to THIS form."
        ),
    }
}

/// Which form a fill turn should target.
///
/// The planner decides *whether* this is a form turn; the form on screen only
/// decides *which* one. Letting an open form turn every question into a fill
/// would hijack "qué correos tengo hoy" asked with the Create Lens dialog up —
/// the same trap `docs/DECISIONS.md` (2026-09-14) records for routing: context
/// is a hint, never a gate.
pub fn resolve_target_form(
    planner_pick: Option<&'static str>,
    open_form: Option<&'static str>,
) -> Option<&'static str> {
    planner_pick.map(|pick| open_form.unwrap_or(pick))
}

/// The per-call hint the query planner gets about an open form, so "añade una
/// columna de IVA" is recognised as a form request at all. Empty when nothing
/// fillable is open.
///
/// Goes in the planner prompt's per-call tail (after `{{query}}`), never its
/// cached head — it varies per turn.
pub fn planner_form_hint(open_form: Option<&str>) -> String {
    match open_form {
        Some(id) => format!(
            "(The user has the \"{id}\" form open on screen. A request to add, change or \
             remove a field of it is {{\"form\": \"{id}\"}}.)"
        ),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_main_view() {
        assert_eq!(
            parse_view_context("view/lenses"),
            Some(ViewContext::View("lenses".into()))
        );
    }

    #[test]
    fn parses_a_settings_tab() {
        assert_eq!(
            parse_view_context("settings/ai"),
            Some(ViewContext::Settings("ai".into()))
        );
    }

    #[test]
    fn parses_an_open_form_into_its_registered_id() {
        assert_eq!(
            parse_view_context("form/lens.create"),
            Some(ViewContext::Form("lens.create"))
        );
    }

    #[test]
    fn rejects_a_form_id_that_is_not_registered() {
        assert_eq!(parse_view_context("form/lens.destroy"), None);
    }

    #[test]
    fn rejects_a_view_that_does_not_exist() {
        assert_eq!(parse_view_context("view/rm-rf"), None);
        assert_eq!(parse_view_context("settings/root"), None);
    }

    #[test]
    fn rejects_junk_without_panicking() {
        for junk in ["", "   ", "lenses", "view/", "/", "form/", "https://evil.test"] {
            assert_eq!(parse_view_context(junk), None, "accepted junk: {junk:?}");
        }
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        assert_eq!(
            parse_view_context("  view/calendar "),
            Some(ViewContext::View("calendar".into()))
        );
    }

    #[test]
    fn only_a_form_context_reports_an_open_form() {
        assert_eq!(ViewContext::Form("lens.create").open_form_id(), Some("lens.create"));
        assert_eq!(ViewContext::View("lenses".into()).open_form_id(), None);
        assert_eq!(ViewContext::Settings("ai".into()).open_form_id(), None);
    }

    #[test]
    fn an_open_form_line_claims_field_edits_for_that_form() {
        let line = view_context_line(&ViewContext::Form("lens.create"));
        assert!(line.contains("lens.create"));
        assert!(line.contains("THIS form"));
    }

    #[test]
    fn a_plain_view_line_tells_the_model_to_ignore_it_when_irrelevant() {
        let line = view_context_line(&ViewContext::View("calendar".into()));
        assert!(line.contains("calendar"));
        assert!(line.contains("ignore it otherwise"));
    }

    #[test]
    fn the_context_line_stays_short_enough_to_ride_on_every_turn() {
        // It is prepended to the user message on every turn the panel is open.
        // A line that grows past ~200 chars is a paragraph, not a hint.
        for ctx in [
            ViewContext::View("lenses".into()),
            ViewContext::Settings("ai".into()),
            ViewContext::Form("lens.create"),
        ] {
            let len = view_context_line(&ctx).len();
            assert!(len < 220, "{ctx:?} renders {len} chars");
        }
    }

    #[test]
    fn the_form_on_screen_wins_over_the_one_the_planner_guessed() {
        assert_eq!(
            resolve_target_form(Some("lens.create"), Some("lens.create")),
            Some("lens.create")
        );
    }

    #[test]
    fn the_planner_pick_applies_when_no_form_is_open() {
        assert_eq!(resolve_target_form(Some("lens.create"), None), Some("lens.create"));
    }

    #[test]
    fn an_open_form_never_turns_an_ordinary_question_into_a_fill() {
        // The regression this guards: asking "qué correos tengo hoy" with the
        // Create Lens dialog open must still be an ordinary mailbox turn.
        assert_eq!(resolve_target_form(None, Some("lens.create")), None);
    }

    #[test]
    fn nothing_open_and_nothing_picked_targets_no_form() {
        assert_eq!(resolve_target_form(None, None), None);
    }

    #[test]
    fn the_planner_hint_names_the_open_form_and_the_verdict_it_should_emit() {
        let hint = planner_form_hint(Some("lens.create"));
        assert!(hint.contains("lens.create"));
        assert!(hint.contains("\"form\""));
    }

    #[test]
    fn the_planner_hint_is_empty_when_no_form_is_open() {
        assert_eq!(planner_form_hint(None), "");
    }
}
