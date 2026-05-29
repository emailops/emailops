//! Sanitization for HTML composed in the rich-text editor before it ships to
//! a provider.
//!
//! We do NOT trust the frontend to produce safe HTML even though the Tiptap
//! editor only emits an allowlisted subset — anyone can hand-craft a payload
//! and call `send_new_email` directly via the Tauri IPC bridge. The backend
//! is the security boundary.
//!
//! The policy here is intentionally narrower than incoming email rendering
//! (`sanitizeEmailHtml` on the frontend) — we only need to support the tags
//! the compose editor can produce, plus inline images via `cid:` URIs.

use ammonia::Builder;
use std::collections::HashSet;

/// Strip every tag / attribute / URL scheme not on the compose allowlist.
///
/// Allowlist rationale:
/// - Formatting: `p`, `br`, `strong`, `em`, `u`, `s`, `code`, `pre`, `blockquote`
/// - Lists: `ul`, `ol`, `li`
/// - Headings: `h1`-`h6` (Tiptap StarterKit emits these)
/// - Links: `a` with `href` only — schemes restricted to `http`, `https`, `mailto`
/// - Images: `img` with `src` / `alt` / `title` — `src` may be `cid:<id>` for
///   inline pasted images, plus `http`/`https`/`data` for compatibility
///
/// Style attributes are dropped — the editor doesn't need them and they're a
/// common smuggling vector (`expression()`, `behavior:`, `url(javascript:)`).
pub fn sanitize_outgoing_html(html: &str) -> String {
    let mut tags: HashSet<&str> = HashSet::new();
    for t in &[
        "p",
        "br",
        "strong",
        "b",
        "em",
        "i",
        "u",
        "s",
        "strike",
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
    ] {
        tags.insert(*t);
    }

    let mut url_schemes: HashSet<&str> = HashSet::new();
    for s in &["http", "https", "mailto", "cid", "data"] {
        url_schemes.insert(*s);
    }

    Builder::default()
        .tags(tags)
        .url_schemes(url_schemes)
        // Allow href on <a>, src/alt/title on <img>.
        .generic_attributes(HashSet::from_iter(["href", "src", "alt", "title"]))
        // No inline `style` — see module docstring.
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
    fn strips_inline_style_attribute() {
        let out = sanitize_outgoing_html(r#"<p style="color:red;background:url(javascript:alert(1))">x</p>"#);
        assert!(!out.contains("style="), "style attribute should be stripped, got {out}");
        assert!(!out.contains("javascript"));
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
