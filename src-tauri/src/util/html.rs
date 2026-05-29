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

/// Decode the most common HTML entities. Not exhaustive — only covers the
/// entities that show up in stripped email bodies often enough to matter.
pub fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
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
