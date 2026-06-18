//! Markdown → terminal rendering for human-facing (pretty) CLI output.
//!
//! The chat assistant emits markdown — tables, bullet/numbered lists, **bold**,
//! `[n]` citation markers, and `[label](email://id)` navigation links (all
//! mandated by the chat system prompt). Streaming that raw to stdout leaves
//! tables unaligned and link syntax bare, which is the wall-of-text the terminal
//! shows today. Here we re-render the *finished* answer (already persisted and
//! read back by the caller) through [`termimad`], which aligns tables and styles
//! prose.
//!
//! All styling lives behind [`RenderStyle`](super::RenderStyle): only `Rich`
//! gets a colored skin; `Plain` (piped output / `NO_COLOR`) renders the same
//! layout with **zero ANSI**; `Json` never reaches here. So agents (`--json`)
//! and captured output never pay extra tokens for styling.
//!
//! The pure functions here (`strip_internal_links`, `render_answer`,
//! `count_visual_rows`) are unit-tested; the actual cursor control for the live
//! preview / redraw lives in [`super::output`] and is validated manually.

use std::sync::OnceLock;

use regex::Regex;
use termimad::MadSkin;

/// Wrap width used when the terminal size can't be detected (e.g. piped output).
pub(crate) const DEFAULT_WIDTH: usize = 100;

/// Detected terminal `(width, height)` in cells — width clamped to a readable
/// band so tables don't sprawl on ultra-wide terminals, height kept as-is for
/// the preview-clear viewport check. Falls back to [`DEFAULT_WIDTH`]×24 when
/// stdout isn't a measurable terminal (piped output). Impure (reads the
/// terminal); the rendering math it feeds stays pure. `termimad` re-exports the
/// `crossterm` already in the dependency tree, so this adds no new dep.
pub(crate) fn term_size() -> (usize, usize) {
    match termimad::crossterm::terminal::size() {
        Ok((w, h)) => ((w as usize).clamp(40, 120), (h as usize).max(1)),
        Err(_) => (DEFAULT_WIDTH, 24),
    }
}

/// Rewrite markdown links whose target is an app-internal `email://` / `draft://`
/// scheme down to just their label text: `[Subject](email://eml-a)` → `Subject`.
/// Those URLs are in-app navigation chips — noise in a terminal. External
/// `http(s)` / `mailto` links are left untouched so termimad can style them, and
/// `[n]` citation markers (which are not links) are preserved. Pure so the
/// rewrite is unit-testable.
pub(crate) fn strip_internal_links(md: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // [label](email://… | draft://…): label has no ']', target no ')'.
        #[allow(clippy::unwrap_used)] // literal pattern, checked at build time
        Regex::new(r"\[([^\]]*)\]\((?:email|draft)://[^)]*\)").unwrap()
    });
    re.replace_all(md, "$1").into_owned()
}

/// Render a finished chat answer (markdown) to a terminal string. `color`
/// selects a colored skin vs a plain (no-ANSI) one; `width` is the wrap width,
/// injected so the function stays pure / unit-testable. Internal nav links are
/// collapsed to their labels first; `[n]` citation markers survive untouched.
pub(crate) fn render_answer(answer: &str, color: bool, width: usize) -> String {
    let md = strip_internal_links(answer);
    let skin = if color {
        MadSkin::default_dark()
    } else {
        // Documented by termimad as the "piped to a file" skin: no color, no
        // attributes — so Plain mode emits zero escape codes.
        MadSkin::no_style()
    };
    skin.text(&md, Some(width)).to_string()
}

/// Count how many terminal rows `text` occupies when printed at `width` columns,
/// accounting for hard newlines and soft wrapping. Used to move the cursor back
/// over the dim live preview before redrawing the clean answer.
///
/// Models the common terminal *deferred-wrap* behaviour: a line whose length is
/// exactly `width` still occupies a single row (the cursor parks in the last
/// column and only wraps when the next character arrives). Counting unicode
/// scalar values, not display width, is close enough for clearing a transient
/// preview and never over-counts a full-width line. Pure / unit-testable.
pub(crate) fn count_visual_rows(text: &str, width: usize) -> usize {
    let width = width.max(1);
    let mut rows = 1usize;
    let mut col = 0usize; // next write column (0-indexed)
    for ch in text.chars() {
        if ch == '\n' {
            rows += 1;
            col = 0;
        } else {
            if col == width {
                // Deferred wrap: this char spills onto a new row.
                rows += 1;
                col = 0;
            }
            col += 1;
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal ANSI CSI stripper for assertions (drops `ESC [ … <final>`
    /// sequences). Enough for termimad's SGR color/attribute codes; avoids
    /// pulling a crate just for tests.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    // Consume params/intermediates until a final byte (0x40–0x7E).
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            break;
                        }
                    }
                }
                continue;
            }
            out.push(c);
        }
        out
    }

    #[test]
    fn strip_internal_links_collapses_email_and_draft_links_to_labels() {
        let md = "See [Propuesta de llamada](email://eml-a) and [the draft](draft://d1).";
        let out = strip_internal_links(md);
        assert_eq!(out, "See Propuesta de llamada and the draft.");
    }

    #[test]
    fn strip_internal_links_leaves_external_links_and_citations() {
        // http/mailto links survive (termimad styles them); [1] is not a link.
        let md = "Site [docs](https://example.com) — paid on March 3rd [1].";
        let out = strip_internal_links(md);
        assert_eq!(out, "Site [docs](https://example.com) — paid on March 3rd [1].");
    }

    #[test]
    fn render_answer_plain_emits_no_ansi_escape_codes() {
        let md = "# Heading\n\nSome **bold** text and a list:\n\n- one\n- two\n";
        let out = render_answer(md, false, 80);
        assert!(!out.contains('\u{1b}'), "plain render must not contain ANSI: {out:?}");
        assert!(out.contains("Heading"));
        assert!(out.contains("one") && out.contains("two"));
    }

    #[test]
    fn render_answer_aligns_table_columns() {
        // A ragged markdown table should render with the pipe separators aligned
        // to a consistent column once termimad lays it out.
        let md = "\
| Week | Downloads |
|------|-----------|
| Jun 13 | 636 |
| May 9 | 458 |
";
        let out = strip_ansi(&render_answer(md, false, 80));
        let table_lines: Vec<&str> = out.lines().filter(|l| l.contains('│') || l.contains('|')).collect();
        assert!(table_lines.len() >= 2, "expected table rows, got: {out:?}");
        // termimad pads cells so the vertical borders line up: every rendered
        // table row has the same display length.
        let widths: Vec<usize> = table_lines.iter().map(|l| l.chars().count()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "table rows are not aligned to equal width: {widths:?} in {out:?}"
        );
    }

    #[test]
    fn render_answer_color_emits_ansi() {
        let md = "# Heading\n\ntext";
        let out = render_answer(md, true, 80);
        assert!(out.contains('\u{1b}'), "colored render should contain ANSI escapes");
    }

    #[test]
    fn render_answer_preserves_citation_markers() {
        let md = "The kickoff was March 3rd [1], confirmed by finance [2].";
        let out = strip_ansi(&render_answer(md, false, 100));
        assert!(out.contains("[1]"), "citation [1] dropped: {out:?}");
        assert!(out.contains("[2]"), "citation [2] dropped: {out:?}");
    }

    #[test]
    fn count_visual_rows_counts_hard_newlines() {
        assert_eq!(count_visual_rows("a\nb\nc", 80), 3);
        assert_eq!(count_visual_rows("single line", 80), 1);
        assert_eq!(count_visual_rows("", 80), 1);
    }

    #[test]
    fn count_visual_rows_handles_soft_wrap_with_deferred_boundary() {
        // Exactly `width` chars → still one row (deferred wrap).
        assert_eq!(count_visual_rows("abcd", 4), 1);
        // One past the boundary → two rows.
        assert_eq!(count_visual_rows("abcde", 4), 2);
        // Newline after a full-width line doesn't double-count.
        assert_eq!(count_visual_rows("abcd\n", 4), 2);
    }
}
