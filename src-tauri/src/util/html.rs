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

/// Decode the most common HTML entities. Covers a handful of named entities
/// plus all decimal (`&#NNN;`) and hex (`&#xHH;`) numeric character references.
/// Numeric refs matter because newsletters encode emoji and invisible spacer
/// characters that way; left undecoded they show up as literal `&#…;` text.
pub fn decode_html_entities(s: &str) -> String {
    let named = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&nbsp;", " ");
    decode_numeric_entities(&named)
}

/// Replace `&#NNN;` / `&#xHH;` numeric character references with the code
/// point they name. Invalid or out-of-range references are left untouched.
fn decode_numeric_entities(s: &str) -> String {
    if !s.contains("&#") {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' && i + 2 < bytes.len() && bytes[i + 1] == b'#' {
            let mut j = i + 2;
            let hex = bytes[j] == b'x' || bytes[j] == b'X';
            if hex {
                j += 1;
            }
            let start = j;
            // Bound the digit run so a stray `&#` can't scan unboundedly.
            while j < bytes.len() && bytes[j] != b';' && j - start < 8 {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b';' && j > start {
                let digits = &s[start..j];
                let code = if hex {
                    u32::from_str_radix(digits, 16).ok()
                } else {
                    digits.parse::<u32>().ok()
                };
                if let Some(ch) = code.and_then(char::from_u32) {
                    out.push(ch);
                    i = j + 1;
                    continue;
                }
            }
        }
        // Copy one UTF-8 char. Leading byte encodes length: 0xxxxxxx=1,
        // 110xxxxx=2, 1110xxxx=3, 11110xxx=4.
        let b = bytes[i];
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
        out.push_str(&s[i..end]);
        i = end;
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn leaves_invalid_numeric_entity_untouched() {
        // Out-of-range code point — char::from_u32 fails, keep literal.
        assert_eq!(decode_html_entities("&#999999999;"), "&#999999999;");
        // No terminating semicolon.
        assert_eq!(decode_html_entities("&#128104 plain"), "&#128104 plain");
        // Non-numeric body.
        assert_eq!(decode_html_entities("&#abc;"), "&#abc;");
    }
}
