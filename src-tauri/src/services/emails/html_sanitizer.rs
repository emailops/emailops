//! Sanitization for HTML composed in the rich-text editor before it ships to
//! a provider.
//!
//! We do NOT trust the frontend to produce safe HTML even though the Tiptap
//! editor only emits an allowlisted subset — anyone can hand-craft a payload
//! and call `send_new_email` directly via the Tauri IPC bridge. The backend
//! is the security boundary.
//!
//! The policy here is intentionally narrower than incoming email rendering
//! (`sanitizeEmailHtml` on the frontend): what the compose editor can produce,
//! inline images via `cid:` URIs, and the table/inline-style formatting a
//! draft written in the provider's own client carries, so sending that draft
//! from here does not flatten it.

use ammonia::Builder;
use std::borrow::Cow;
use std::collections::HashSet;

/// Inline-style properties that survive. None of them can take a `url()`, so
/// a kept declaration cannot load or run anything.
const STYLE_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-align",
    "text-decoration",
    "text-indent",
    "line-height",
    "letter-spacing",
    "white-space",
    "vertical-align",
    "width",
    "max-width",
    "height",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-color",
    "border-style",
    "border-width",
    "border-collapse",
    "border-spacing",
    "list-style-type",
];

/// Drop the declarations of a `style` attribute whose value could fetch or
/// execute something (`url(`, `expression(`, CSS escapes, at-rules), whatever
/// the property. The property allowlist runs on what is left.
fn drop_unsafe_style_declarations(style: &str) -> String {
    style
        .split(';')
        .filter(|declaration| {
            let lower = declaration.to_ascii_lowercase();
            !["url(", "expression(", "\\", "@", "javascript:"]
                .iter()
                .any(|needle| lower.contains(needle))
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Strip every tag / attribute / URL scheme not on the compose allowlist.
///
/// Allowlist rationale:
/// - Formatting: `p`, `br`, `strong`, `em`, `u`, `s`, `code`, `pre`, `blockquote`,
///   `sub`, `sup`, `small`, `font`, `center`
/// - Lists: `ul`, `ol`, `li`
/// - Headings: `h1`-`h6` (Tiptap StarterKit emits these)
/// - Links: `a` with `href` only — schemes restricted to `http`, `https`, `mailto`
/// - Images: `img` with `src` / `alt` / `title` — `src` may be `cid:<id>` for
///   inline pasted images, plus `http`/`https`/`data` for compatibility
/// - Tables: `table`, `thead`, `tbody`, `tfoot`, `tr`, `th`, `td`, `caption`,
///   `colgroup`, `col` with their layout attributes
/// - Inline `style`, reduced to [`STYLE_PROPERTIES`] and to declarations that
///   cannot load or run anything (`expression()`, `url(javascript:)` and the
///   like are dropped)
///
/// `<style>` blocks and the `background` attribute stay out: both can fetch a
/// remote resource when the recipient opens the message.
pub fn sanitize_outgoing_html(html: &str) -> String {
    let tags: HashSet<&str> = HashSet::from_iter([
        "p",
        "br",
        "strong",
        "b",
        "em",
        "i",
        "u",
        "s",
        "strike",
        "sub",
        "sup",
        "small",
        "font",
        "center",
        "code",
        "pre",
        "blockquote",
        "ul",
        "ol",
        "li",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "a",
        "img",
        "span",
        "div",
        "hr",
        "table",
        "thead",
        "tbody",
        "tfoot",
        "tr",
        "th",
        "td",
        "caption",
        "colgroup",
        "col",
    ]);

    let url_schemes: HashSet<&str> = HashSet::from_iter(["http", "https", "mailto", "cid", "data"]);

    Builder::default()
        .tags(tags)
        .url_schemes(url_schemes)
        // href on <a>, src/alt/title on <img>, layout attributes on tables and
        // legacy formatting tags. None of the added ones carries a URL.
        .generic_attributes(HashSet::from_iter([
            "href",
            "src",
            "alt",
            "title",
            "style",
            "align",
            "valign",
            "width",
            "height",
            "bgcolor",
            "color",
            "face",
            "size",
            "border",
            "cellpadding",
            "cellspacing",
            "colspan",
            "rowspan",
            "span",
            "dir",
        ]))
        .attribute_filter(|_element, attribute, value| {
            if attribute == "style" {
                Some(Cow::Owned(drop_unsafe_style_declarations(value)))
            } else {
                Some(Cow::Borrowed(value))
            }
        })
        .filter_style_properties(HashSet::from_iter(STYLE_PROPERTIES.iter().copied()))
        .strip_comments(true)
        .link_rel(Some("noopener noreferrer"))
        .clean(html)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_basic_formatting() {
        let out = sanitize_outgoing_html("<p>Hello <strong>world</strong> and <em>peace</em></p>");
        assert!(out.contains("<strong>world</strong>"));
        assert!(out.contains("<em>peace</em>"));
    }

    #[test]
    fn keeps_lists_and_blockquotes() {
        let html = "<ul><li>a</li><li>b</li></ul><blockquote>quoted</blockquote>";
        let out = sanitize_outgoing_html(html);
        assert!(out.contains("<ul>"));
        assert!(out.contains("<li>a</li>"));
        assert!(out.contains("<blockquote>quoted</blockquote>"));
    }

    #[test]
    fn keeps_cid_images_for_inline_pastes() {
        let out = sanitize_outgoing_html(r#"<p>see: <img src="cid:img1" alt="pic"></p>"#);
        assert!(
            out.contains(r#"src="cid:img1""#),
            "cid: src should survive — it's how inline pasted images reference MIME parts; got {out}"
        );
        assert!(out.contains(r#"alt="pic""#));
    }

    #[test]
    fn allows_http_https_data_image_sources() {
        for src in [
            "https://example.com/x.png",
            "http://example.com/x.png",
            "data:image/png;base64,AAAA",
        ] {
            let out = sanitize_outgoing_html(&format!(r#"<img src="{src}">"#));
            assert!(out.contains(src), "expected {src} to survive, got {out}");
        }
    }

    #[test]
    fn strips_script_tag_and_event_handlers() {
        let out = sanitize_outgoing_html(r#"<p onclick="alert(1)">hi</p><script>alert(2)</script>"#);
        assert!(!out.contains("script"));
        assert!(!out.contains("onclick"));
        assert!(!out.contains("alert"));
    }

    #[test]
    fn rejects_javascript_href() {
        let out = sanitize_outgoing_html(r#"<a href="javascript:alert(1)">x</a>"#);
        assert!(
            !out.contains("javascript:"),
            "javascript: scheme must be stripped, got {out}"
        );
    }

    #[test]
    fn rejects_file_and_unknown_schemes() {
        for href in ["file:///etc/passwd", "vbscript:msgbox(1)", "weird:thing"] {
            let out = sanitize_outgoing_html(&format!(r#"<a href="{href}">x</a>"#));
            assert!(!out.contains(href), "scheme should be stripped: {href} → {out}");
        }
    }

    #[test]
    fn keeps_tables_and_their_layout_attributes() {
        let html = r##"<table border="1" cellpadding="4" width="600"><thead><tr><th align="left">Item</th></tr></thead><tbody><tr><td colspan="2" bgcolor="#eeeeee" valign="top">x</td></tr></tbody></table>"##;
        let out = sanitize_outgoing_html(html);
        for kept in [
            "<table",
            "<thead>",
            "<tbody>",
            "<tr>",
            "<th",
            "<td",
            r#"border="1""#,
            r#"cellpadding="4""#,
            r#"width="600""#,
            r#"align="left""#,
            r#"colspan="2""#,
            r##"bgcolor="#eeeeee""##,
            r#"valign="top""#,
        ] {
            assert!(out.contains(kept), "expected {kept} to survive, got {out}");
        }
    }

    #[test]
    fn keeps_safe_inline_styles() {
        let out = sanitize_outgoing_html(
            r##"<p style="color: red; text-align: center; font-size: 14px">x</p><span style="background-color:#ff0">y</span>"##,
        );
        for kept in [
            "color:red",
            "text-align:center",
            "font-size:14px",
            "background-color:#ff0",
        ] {
            assert!(
                out.replace(' ', "").contains(kept),
                "expected {kept} to survive, got {out}"
            );
        }
    }

    #[test]
    fn drops_style_declarations_that_can_load_or_run_something() {
        let out = sanitize_outgoing_html(
            r#"<p style="color:red;background:url(javascript:alert(1));background-image:url(https://example.com/t.png);width:expression(alert(2));position:fixed">x</p>"#,
        );
        assert!(
            out.replace(' ', "").contains("color:red"),
            "safe declaration lost: {out}"
        );
        for gone in ["javascript", "url(", "expression", "position", "example.com"] {
            assert!(!out.contains(gone), "{gone} should be stripped, got {out}");
        }
    }

    #[test]
    fn strips_style_blocks_and_background_attributes() {
        let out = sanitize_outgoing_html(
            r#"<style>p { background: url(https://example.com/a.png) }</style><table><tr><td background="https://example.com/b.png">x</td></tr></table>"#,
        );
        assert!(!out.contains("<style"), "style block should be dropped, got {out}");
        assert!(!out.contains("example.com"), "remote background survived: {out}");
        assert!(out.contains("<td>x</td>"), "cell content should stay, got {out}");
    }

    #[test]
    fn strips_iframes_and_embeds() {
        for html in [
            "<iframe src='https://evil'></iframe>",
            "<object data='https://evil'></object>",
            "<embed src='https://evil'>",
            "<form action='https://evil'><input></form>",
        ] {
            let out = sanitize_outgoing_html(html);
            assert!(!out.contains("evil"), "embedded payload survived: {html} → {out}");
        }
    }

    #[test]
    fn strips_html_comments() {
        let out = sanitize_outgoing_html("<p>hi</p><!-- secret note -->");
        assert!(!out.contains("secret note"));
    }

    #[test]
    fn safe_http_link_gets_rel_noopener() {
        let out = sanitize_outgoing_html(r#"<a href="https://example.com">x</a>"#);
        assert!(out.contains(r#"href="https://example.com""#));
        assert!(out.contains("noopener"), "rel should add noopener, got {out}");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(sanitize_outgoing_html(""), "");
    }
}
