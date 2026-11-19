/// Strip HTML tags, style/script blocks, and entities for FTS indexing.
/// Produces clean text content from email HTML bodies so that HTML tag names
/// (e.g. "table", "div", "style") don't pollute search results.
pub fn strip_html_for_fts(html: &str) -> String {
    if !html.contains('<') {
        return decode_html_entities(html);
    }

    // Byte-level scan: tag names are ASCII, so `eq_ignore_ascii_case` on a
    // byte slice avoids allocating a lowercased String per tag.  Content chars
    // are re-emitted via str slicing at UTF-8 boundaries.
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    let mut in_tag = false;
    let mut skip_until: Option<&'static [u8]> = None;

    while i < bytes.len() {
        if let Some(close_tag) = skip_until {
            if i + close_tag.len() <= bytes.len() && bytes[i..i + close_tag.len()].eq_ignore_ascii_case(close_tag) {
                skip_until = None;
                in_tag = false;
                i += close_tag.len();
                continue;
            }
            i += 1;
            continue;
        }
        let b = bytes[i];
        if b == b'<' {
            // `Name <addr@host>` in plain-text mail is not a tag: keep the
            // address. Real tags carry spaces or `=` before any `@`.
            if let Some(addr) = angle_bracket_address(&html[i + 1..]) {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(addr);
                out.push(' ');
                i += addr.len() + 2;
                continue;
            }
            if bytes.len() - i >= 6 && bytes[i..i + 6].eq_ignore_ascii_case(b"<style") {
                skip_until = Some(b"</style>");
            } else if bytes.len() - i >= 7 && bytes[i..i + 7].eq_ignore_ascii_case(b"<script") {
                skip_until = Some(b"</script>");
            }
            in_tag = true;
            i += 1;
            continue;
        }
        if b == b'>' {
            in_tag = false;
            if !out.ends_with(' ') {
                out.push(' ');
            }
            i += 1;
            continue;
        }
        if !in_tag {
            // Copy one UTF-8 char via str slicing (safe at char boundaries).
            // UTF-8 leading byte encodes length: 0xxxxxxx=1, 110xxxxx=2,
            // 1110xxxx=3, 11110xxx=4. `b < 0xC0` covers ASCII and stray
            // continuation bytes (shouldn't land here on valid str input).
            let ch_len = if b < 0xC0 {
                1
            } else if b < 0xE0 {
                2
            } else if b < 0xF0 {
                3
            } else {
                4
            };
            let end = (i + ch_len).min(bytes.len());
            out.push_str(&html[i..end]);
            i = end;
            continue;
        }
        i += 1;
    }

    decode_html_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `rest` starts right after a `<`: the address it wraps when it reads
/// `addr@host>`, e.g. the `<ana@site.example>` of `Ana <ana@site.example>`.
fn angle_bracket_address(rest: &str) -> Option<&str> {
    let end = rest.find('>')?;
    let inner = &rest[..end];
    let looks_like_address = inner.contains('@')
        && !inner.starts_with('/')
        && !inner
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '=' | '<'));
    looks_like_address.then_some(inner)
}

/// Named references decoded besides the numeric ones: the five of HTML
/// escaping, the non-breaking space (as a plain space), the zero-width
/// non-joiner newsletters pad previews with, and the accented letters and
/// marks of Spanish text.
const NAMED_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", ' '),
    ("zwnj", '\u{200C}'),
    ("aacute", 'á'),
    ("eacute", 'é'),
    ("iacute", 'í'),
    ("oacute", 'ó'),
    ("uacute", 'ú'),
    ("Aacute", 'Á'),
    ("Eacute", 'É'),
    ("Iacute", 'Í'),
    ("Oacute", 'Ó'),
    ("Uacute", 'Ú'),
    ("ntilde", 'ñ'),
    ("Ntilde", 'Ñ'),
    ("uuml", 'ü'),
    ("Uuml", 'Ü'),
    ("iexcl", '¡'),
    ("iquest", '¿'),
];

/// The longest reference between `&` and `;`: `#x` and eight hex digits.
/// Bounds the look-ahead, so a stray `&` never scans the rest of the text.
const MAX_ENTITY_LEN: usize = 10;

/// Decode HTML character references: decimal (`&#NNN;`) and hex (`&#xHH;`)
/// ones — newsletters encode emoji and invisible spacers that way — and the
/// named ones in `NAMED_ENTITIES`. One pass, so text escaped twice (Gmail
/// sends a typed `&lt;` as `&amp;lt;`) comes out escaped once. Unknown or
/// malformed references are left as they are.
pub fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after
            .bytes()
            .take(MAX_ENTITY_LEN + 1)
            .position(|b| b == b';')
            .and_then(|end| entity_char(&after[..end]).map(|ch| (ch, end)));
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The character a reference names (`amp`, `#39`, `#x1F468`), if any.
fn entity_char(name: &str) -> Option<char> {
    let Some(number) = name.strip_prefix('#') else {
        return NAMED_ENTITIES.iter().find(|(n, _)| *n == name).map(|&(_, ch)| ch);
    };
    let code = match number.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => number.parse::<u32>().ok(),
    };
    code.and_then(char::from_u32)
}

/// Strip HTML tags only — no style/script block handling, no entity decoding.
/// Cheap single-pass scan suited for embedding-chunk preparation where the
/// downstream model handles entities and odd whitespace anyway.
pub fn strip_html_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Split a provider's draft body into `(plain_text, body_html)`.
///
/// Provider `list_drafts` responses hand us the message body as HTML (Outlook
/// always, Gmail after `parse_message` reassembles it). The drafts table and
/// the composer expect the split shape: `body` = plain text (previews, search)
/// and `body_html` = the rich source the editor renders. Storing HTML in `body`
/// with `body_html = None` makes the composer fall back to escaping the HTML as
/// literal text — the raw `<html><head>…` bug.
///
/// A body with no `<` is already plain text: return it as-is with `None` so the
/// composer wraps it itself (preserving newlines via its own text→HTML path).
pub fn split_draft_body(body: &str) -> (String, Option<String>) {
    if body.contains('<') {
        (strip_html_for_fts(body), Some(body.to_string()))
    } else {
        (body.to_string(), None)
    }
}

/// Largest body that inlining `cid:` images may produce.
pub const MAX_INLINED_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Replace every `cid:{content_id}` reference in `html` with `replacement`.
///
/// A reference only matches when the id ends there — at a quote, paren, `>`,
/// whitespace or the end of the text. A plain substring replace let `cid:img1`
/// also rewrite the front of `cid:img10`, corrupting that image.
///
/// Each reference gets a full copy of the image, so a sender can repeat one
/// reference until the body is gigabytes. Once inlining would take the body
/// past [`MAX_INLINED_BODY_BYTES`], the remaining references stay links.
pub fn replace_cid_reference(html: &str, content_id: &str, replacement: &str) -> String {
    let needle = format!("cid:{content_id}");
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    // Length of the result if every reference left from here stays a link.
    let mut projected = html.len();
    let mut skipped = 0usize;
    while let Some(pos) = rest.find(&needle) {
        let after = &rest[pos + needle.len()..];
        let terminated = after
            .chars()
            .next()
            .is_none_or(|c| matches!(c, '"' | '\'' | ')' | '>') || c.is_whitespace());
        let inlined = projected - needle.len() + replacement.len();
        let fits = inlined <= MAX_INLINED_BODY_BYTES;
        out.push_str(&rest[..pos]);
        if terminated && fits {
            out.push_str(replacement);
            projected = inlined;
        } else {
            out.push_str(&needle);
            skipped += usize::from(terminated);
        }
        rest = after;
    }
    out.push_str(rest);
    if skipped > 0 {
        crate::services::logger::log(
            "debug",
            "sync",
            format!(
                "{skipped} inline image reference(s) left as links: the body would pass {MAX_INLINED_BODY_BYTES} bytes"
            ),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn cid_replacement_does_not_touch_a_longer_cid_sharing_the_prefix() {
        let html = r#"<img src="cid:img1"><img src="cid:img10">"#;
        let out = super::replace_cid_reference(html, "img1", "data:image/png;base64,AAA");
        assert_eq!(out, r#"<img src="data:image/png;base64,AAA"><img src="cid:img10">"#);
    }

    #[test]
    fn cid_replacement_matches_every_terminator() {
        let html = "<img src='cid:a'> url(cid:a) cid:a\n<img src=cid:a>cid:a";
        let out = super::replace_cid_reference(html, "a", "X");
        assert_eq!(out, "<img src='X'> url(X) X\n<img src=X>X");
    }

    fn data_uri_of(bytes: usize) -> String {
        format!("data:image/png;base64,{}", "A".repeat(bytes))
    }

    /// A few KB of HTML repeating one `cid:` reference used to inline the
    /// image once per occurrence: a body of gigabytes that SQLite refuses.
    #[test]
    fn many_references_to_one_image_stay_under_the_inlining_cap() {
        let html = r#"<img src="cid:a">"#.repeat(10_000);
        let out = super::replace_cid_reference(&html, "a", &data_uri_of(1024 * 1024));
        assert!(out.len() <= super::MAX_INLINED_BODY_BYTES, "{} bytes", out.len());
    }

    #[test]
    fn references_past_the_inlining_cap_stay_links() {
        let html = r#"<img src="cid:a">"#.repeat(40);
        let out = super::replace_cid_reference(&html, "a", &data_uri_of(1024 * 1024));
        assert!(out.len() <= super::MAX_INLINED_BODY_BYTES, "{} bytes", out.len());
        assert!(
            out.starts_with(r#"<img src="data:image/png;base64,"#),
            "the first ones still inline"
        );
        assert!(out.ends_with(r#"<img src="cid:a">"#), "the last ones stay links");
    }

    #[test]
    fn distinct_images_that_fit_under_the_cap_are_all_inlined() {
        let mut html = r#"<img src="cid:a"><img src="cid:b"><img src="cid:c">"#.to_string();
        for cid in ["a", "b", "c"] {
            html = super::replace_cid_reference(&html, cid, &data_uri_of(10 * 1024 * 1024));
        }
        assert!(!html.contains("cid:"), "every image inlined");
    }
    use super::*;

    #[test]
    fn strip_keeps_an_address_written_in_angle_brackets() {
        // Plain-text mail names people as `Name <addr>`. Read as a tag, the
        // address vanished from the search index and from the text Lens
        // extraction sends the model.
        let got = strip_html_for_fts("From: Sam Lee <sam.lee@mail.example>\nSubject: Hi");
        assert_eq!(got, "From: Sam Lee sam.lee@mail.example Subject: Hi");
    }

    #[test]
    fn strip_still_removes_real_tags_around_addresses() {
        let got = strip_html_for_fts(r#"<p>Write to <a href="mailto:ana@site.example">ana@site.example</a></p>"#);
        assert_eq!(got, "Write to ana@site.example");
    }

    #[test]
    fn split_draft_body_keeps_html_and_derives_plain() {
        let html = "<html><head><style>.x{color:red}</style></head><body><div>Hola</div><div>Mundo</div></body></html>";
        let (plain, body_html) = split_draft_body(html);
        // Rich source preserved verbatim for the editor.
        assert_eq!(body_html.as_deref(), Some(html));
        // Plain rendering drops tags + style blocks, so previews aren't HTML soup.
        assert_eq!(plain, "Hola Mundo");
        assert!(!plain.contains('<'));
    }

    #[test]
    fn split_draft_body_leaves_plain_text_alone() {
        let (plain, body_html) = split_draft_body("just plain text");
        assert_eq!(plain, "just plain text");
        assert_eq!(body_html, None);
    }

    #[test]
    fn decodes_named_entities() {
        assert_eq!(
            decode_html_entities("a &amp; b &lt;c&gt; &quot;d&quot;"),
            "a & b <c> \"d\""
        );
    }

    #[test]
    fn decodes_decimal_numeric_entity() {
        // U+1F468 MAN — substack newsletters encode emoji as numeric refs.
        assert_eq!(decode_html_entities("&#128104;"), "👨");
    }

    #[test]
    fn decodes_hex_numeric_entity() {
        assert_eq!(decode_html_entities("&#x1F468;"), "👨");
        assert_eq!(decode_html_entities("&#X1f468;"), "👨");
    }

    #[test]
    fn decodes_decimal_entity_in_running_text() {
        // 8364 = € EURO SIGN
        assert_eq!(decode_html_entities("price: &#8364;10"), "price: €10");
    }

    #[test]
    fn decodes_spanish_accents_and_inverted_marks() {
        assert_eq!(
            decode_html_entities("&iquest;S&iacute;? &iexcl;Canci&oacute;n de caf&eacute;!"),
            "¿Sí? ¡Canción de café!"
        );
    }

    #[test]
    fn decodes_apos_and_the_zero_width_non_joiner() {
        assert_eq!(decode_html_entities("it&apos;s&zwnj;"), "it's\u{200C}");
    }

    #[test]
    fn decodes_text_escaped_twice_only_once() {
        // Gmail sends a literal `&lt;` typed in a message as `&amp;lt;`.
        assert_eq!(
            decode_html_entities("write &amp;lt;b&amp;gt; for bold"),
            "write &lt;b&gt; for bold"
        );
    }

    #[test]
    fn leaves_an_unknown_named_entity_and_a_bare_ampersand_untouched() {
        assert_eq!(decode_html_entities("a &bogus; b & c"), "a &bogus; b & c");
    }

    #[test]
    fn leaves_invalid_numeric_entity_untouched() {
        // Out-of-range code point — char::from_u32 fails, keep literal.
        assert_eq!(decode_html_entities("&#999999999;"), "&#999999999;");
        // No terminating semicolon.
        assert_eq!(decode_html_entities("&#128104 plain"), "&#128104 plain");
        // Non-numeric body.
        assert_eq!(decode_html_entities("&#abc;"), "&#abc;");
    }
}
