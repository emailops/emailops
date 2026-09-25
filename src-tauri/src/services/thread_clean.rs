//! Read an email body for a model without what its thread already said.
//!
//! A reply carries the conversation before it as quoted text, often with a
//! signature the thread has seen before. Re-reading that wastes the context
//! window. But a quote is not always history: in a one-message thread, a
//! forward, or a reply to mail that was never synced, the quote is the only
//! copy. So nothing is cut on a marker alone — **a block goes only when the
//! thread already contains it**:
//!
//!   1. [`normalize_body`]: HTML → text with nothing removed (`<blockquote>`
//!      content becomes `> ` lines, like a plain-text quote).
//!   2. [`segment`]: the markers ("On … wrote:", Outlook `From:/Sent:`
//!      headers, "Original/Forwarded message", `>` lines, the `-- ` signature
//!      delimiter) only split the text into own / quoted / signature blocks.
//!   3. [`new_text`]: a quoted or signature block is dropped when ~80% of its
//!      5-word runs appear in the earlier messages ([`History`]); an own
//!      paragraph when it repeats one of theirs. "Sent from my iPhone" stubs
//!      always go. Everything else stays.
//!
//! [`clean_email_body`] reads a body with no thread around it, so only stubs
//! and whitespace go; `thread_reader` supplies the history for a thread.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use crate::util::html::decode_html_entities;

/// Per-email ceiling. A single-message "chat about this email" shows the body
/// nearly whole instead of clipping it at the floor. Also the cap applied by
/// the `get_email_body` chat tool, so a long newsletter comes back whole rather
/// than sliced in half.
pub const MAX_CHARS_PER_EMAIL: usize = 16000;

/// Total budget shared across ALL emails preseeded into a multi-email summary
/// (e.g. the "resumen del día" shortcut). Deliberately far tighter than a
/// thread's budget: a summary table only needs a one-line gist per email, and a
/// weak local model fixates on — and gets derailed by — one long body if the
/// first row is allowed to consume the whole context window.
const SUMMARY_BODIES_BUDGET: usize = 6_000;

/// Per-email floor for a summary excerpt — enough for a one-line gist.
pub const MIN_SUMMARY_CHARS_PER_EMAIL: usize = 300;

/// Per-email ceiling for a summary excerpt. Well below [`MAX_CHARS_PER_EMAIL`]
/// so no single email can swallow the budget the way an 8000-char newsletter did.
pub const MAX_SUMMARY_CHARS_PER_EMAIL: usize = 1_500;

/// Pick the per-email excerpt cap when inlining N email bodies into a single
/// summary turn. Fair-shares [`SUMMARY_BODIES_BUDGET`] across the rows so the
/// first (often longest) email cannot crowd out the rest, clamped to
/// `[MIN_SUMMARY_CHARS_PER_EMAIL, MAX_SUMMARY_CHARS_PER_EMAIL]`.
pub fn summary_chars_per_email(num_emails: usize) -> usize {
    let n = num_emails.max(1);
    (SUMMARY_BODIES_BUDGET / n).clamp(MIN_SUMMARY_CHARS_PER_EMAIL, MAX_SUMMARY_CHARS_PER_EMAIL)
}

/// Clean a single email body. Accepts the raw body (HTML or plain text).
///
/// Unlike [`crate::util::html::strip_html_for_fts`], this function preserves
/// line breaks — they're load-bearing for the quote/signature heuristics that
/// run downstream.
pub fn clean_email_body(body: &str, max_chars: usize) -> String {
    truncate_chars(&new_text(&normalize_body(body), &History::default()), max_chars)
}

/// A body as plain text with nothing removed: HTML → text (quoted
/// `<blockquote>` content turned into `> ` lines), entities decoded,
/// invisible spacers dropped, `<addr>` without its brackets.
pub fn normalize_body(body: &str) -> String {
    let text = if looks_like_html(body) {
        prefix_blockquote_lines(&html_to_plain_text(&mark_blockquotes(body)))
    } else {
        strip_inline_addr_brackets(body)
    };
    strip_invisible_chars(&text)
}

/// Render an email body as readable plain text **without** dropping any
/// content. This is the full-fidelity sibling of [`clean_email_body`]: it
/// converts HTML to text, strips invisible spacer characters, and collapses
/// runs of blank lines — but it deliberately keeps quoted replies, signatures,
/// and the entire body length intact. Use it where the user asked to see the
/// whole message (e.g. the CLI `show` command), not a context-budgeted excerpt.
pub fn body_to_plain_text(body: &str) -> String {
    let text = to_plain_text(body);
    let visible = strip_invisible_chars(&text);
    collapse_whitespace(&visible)
}

/// Remove zero-width / invisible formatting characters that newsletters stuff
/// into the body (and especially the preheader) to pad the inbox preview.
/// These are noise that consumes the context budget and renders as nothing.
///
/// `U+200D` ZERO WIDTH JOINER is deliberately **kept** — it glues multi-codepoint
/// emoji sequences (e.g. 👨‍💻), so stripping it would corrupt real glyphs.
fn strip_invisible_chars(text: &str) -> String {
    if !text.chars().any(is_invisible_spacer) {
        return text.to_string();
    }
    text.chars().filter(|c| !is_invisible_spacer(*c)).collect()
}

fn is_invisible_spacer(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'   // soft hyphen
        | '\u{034F}' // combining grapheme joiner
        | '\u{200B}' // zero width space
        | '\u{2060}' // word joiner
        | '\u{FEFF}' // zero width no-break space / BOM
    )
}

// ── HTML / plain-text normalisation ────────────────────────────────────────

fn to_plain_text(body: &str) -> String {
    if looks_like_html(body) {
        html_to_plain_text(body)
    } else {
        strip_inline_addr_brackets(body)
    }
}

/// Heuristic: does this body look like HTML?  We look for any of a small set of
/// block-level tags. Plain-text emails containing `<alice@example.com>` or
/// `<https://…>` get a false-negative on purpose so their line breaks survive.
fn looks_like_html(body: &str) -> bool {
    static MARKERS: &[&str] = &[
        "<html", "<body", "<div", "<p>", "<p ", "<br", "<table", "<span", "</p>", "</div>",
        // Plain text some clients wrap in <pre> with its `<`/`>` escaped.
        "<pre",
    ];
    // Lowercase a bounded prefix — bodies can be megabytes.
    let prefix: String = body.chars().take(4096).collect::<String>().to_lowercase();
    MARKERS.iter().any(|m| prefix.contains(m))
}

// All regexes below use hard-coded literals — `Regex::new` only fails on
// invalid syntax, which is caught at build/test time. Allowed per-fn.

#[allow(clippy::expect_used)]
fn block_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?is)<\s*/?\s*(br|p|div|tr|li|h[1-6]|table|ul|ol|blockquote)([\s/][^>]*)?>").expect("valid regex")
    })
}

#[allow(clippy::expect_used)]
fn style_block_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").expect("valid regex"))
}

#[allow(clippy::expect_used)]
fn script_block_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("valid regex"))
}

#[allow(clippy::expect_used)]
fn any_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<[^>]+>").expect("valid regex"))
}

#[allow(clippy::expect_used)]
fn inline_addr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // <foo@bar.tld> and <https://...> tokens that show up in plain-text bodies
    // (mailto display form, RFC 3986 angle-bracket URIs). Group 1 is the
    // address itself, which is content: only the brackets go.
    RE.get_or_init(|| Regex::new(r"<\s*([^<>\s]+@[^<>\s]+|https?://[^<>\s]+)\s*>").expect("valid regex"))
}

// Private-use characters survive tag stripping and never occur in mail text.
const QUOTE_OPEN: char = '\u{E000}';
const QUOTE_CLOSE: char = '\u{E001}';

/// Put every `<blockquote>` / `</blockquote>` on a line of its own as a marker
/// character, so [`prefix_blockquote_lines`] can quote its text after the tags
/// are gone. Mail clients quote the previous message this way, often with no
/// attribution line around it.
fn mark_blockquotes(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while i < html.len() {
        let rest = &lower[i..];
        let open = rest.starts_with("<blockquote") && rest[11..].starts_with(|c: char| c == '>' || c.is_whitespace());
        if open || rest.starts_with("</blockquote") {
            out.push('\n');
            out.push(if open { QUOTE_OPEN } else { QUOTE_CLOSE });
            out.push('\n');
            i += rest.find('>').map_or(rest.len(), |p| p + 1);
            continue;
        }
        let Some(ch) = html[i..].chars().next() else { break };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Turn the lines between blockquote markers into `> ` lines.
fn prefix_blockquote_lines(text: &str) -> String {
    let mut depth = 0usize;
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        match line.trim() {
            t if t == QUOTE_OPEN.to_string() => depth += 1,
            t if t == QUOTE_CLOSE.to_string() => depth = depth.saturating_sub(1),
            t if depth > 0 && !t.is_empty() => out.push(format!("> {t}")),
            _ => out.push(line.to_string()),
        }
    }
    out.join("\n")
}

fn html_to_plain_text(html: &str) -> String {
    // 1. Drop style + script blocks entirely.
    let s = style_block_re().replace_all(html, "");
    let s = script_block_re().replace_all(&s, "").into_owned();
    // 2. Convert block-level tags to newlines BEFORE stripping all tags.
    let s = block_tag_re().replace_all(&s, "\n").into_owned();
    // 3. Remove anything else that still looks like a tag.
    let s = any_tag_re().replace_all(&s, "").into_owned();
    // 4. Decode entities.
    decode_html_entities(&s)
}

fn strip_inline_addr_brackets(text: &str) -> String {
    inline_addr_re().replace_all(text, "$1").into_owned()
}

// ── Thread history ─────────────────────────────────────────────────────────

/// Words per run when comparing a block with the thread: long enough that
/// common phrases do not match by accident, short enough to survive a client
/// re-wrapping lines.
const SHINGLE_WORDS: usize = 5;
/// Share of a block's runs the thread must already contain for it to go.
const KNOWN_PERCENT: usize = 80;
/// Own paragraphs shorter than this are never treated as repeats: "Thanks!",
/// "Best," and one-word answers recur legitimately.
const MIN_REPEAT_CHARS: usize = 40;

/// What the earlier messages of a thread said, as [`match_key`] text and its
/// 5-word runs. Built from each message's whole normalized body, quotes
/// included, so a later reply quoting any of it is recognised.
#[derive(Debug, Default, Clone)]
pub struct History {
    text: String,
    shingles: HashSet<String>,
}

impl History {
    /// Add one message (the output of [`normalize_body`]).
    pub fn add(&mut self, normalized: &str) {
        let key = match_key(normalized);
        self.shingles.extend(shingles(&key));
        self.text.push_str(&key);
        self.text.push('\n');
    }

    /// Does the thread already contain `text`? Short text must appear whole;
    /// longer text when [`KNOWN_PERCENT`] of its runs do. Empty text is known.
    fn knows(&self, text: &str) -> bool {
        let key = match_key(text);
        let words = key.split_whitespace().count();
        if words == 0 {
            return true;
        }
        if words < SHINGLE_WORDS {
            return self.text.contains(&key);
        }
        let runs = shingles(&key);
        let known = runs.iter().filter(|r| self.shingles.contains(*r)).count();
        known * 100 >= runs.len() * KNOWN_PERCENT
    }
}

/// How two copies of a text compare across clients: `>` quote prefixes
/// removed, lowercased, whitespace (line breaks included) collapsed.
fn match_key(text: &str) -> String {
    text.lines()
        .map(|l| {
            l.trim_start()
                .trim_start_matches(|c: char| c == '>' || c.is_whitespace())
        })
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn shingles(key: &str) -> Vec<String> {
    let words: Vec<&str> = key.split_whitespace().collect();
    words.windows(SHINGLE_WORDS).map(|w| w.join(" ")).collect()
}

// ── Segmentation ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlockKind {
    /// What the sender wrote.
    Own,
    /// Quoted or forwarded text: `>` lines, or everything after an
    /// attribution / Outlook header / "Original/Forwarded message" line.
    Quoted,
    /// After a `-- ` delimiter.
    Signature,
    /// "Sent from my iPhone" and the like.
    Stub,
}

#[derive(Debug)]
struct Block<'a> {
    kind: BlockKind,
    /// The lines that announced the block (attribution, headers, `--`):
    /// shown with it, never compared with the thread.
    header: Vec<&'a str>,
    lines: Vec<&'a str>,
}

/// A line that is only one of these (dashes around it allowed) starts quoted
/// or forwarded text. Matched on the whole line: the words inside a sentence
/// ("te reenvío el mensaje original") are prose, not a marker.
const ORIGINAL_MARKERS: [&str; 5] = [
    "original message",
    "forwarded message",
    "mensaje original",
    "mensaje reenviado",
    "ursprüngliche nachricht",
];

/// Header lines of a quoted or forwarded message (Outlook, Gmail forwards).
const HEADER_KEYS: [&str; 20] = [
    "From:",
    "Sent:",
    "To:",
    "Cc:",
    "CC:",
    "Subject:",
    "Date:",
    "De:",
    "Enviado:",
    "Para:",
    "Asunto:",
    "Fecha:",
    "Von:",
    "Gesendet:",
    "An:",
    "Betreff:",
    "Datum:",
    "Envoyé:",
    "À:",
    "Objet:",
];

fn is_header_line(line: &str) -> bool {
    let t = line.trim();
    HEADER_KEYS.iter().any(|k| t.starts_with(k))
}

/// How many header lines run from `start`.
fn header_run(lines: &[&str], start: usize) -> usize {
    lines.iter().skip(start).take_while(|l| is_header_line(l)).count()
}

/// If quoted or forwarded text starts at line `i`, how many lines announce
/// it: the marker, attribution or header lines.
fn quote_header_len(lines: &[&str], i: usize) -> Option<usize> {
    let t = lines[i].trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_lowercase();
    if ORIGINAL_MARKERS.contains(&lower.trim_matches(|c: char| c == '-' || c.is_whitespace())) {
        return Some(1 + header_run(lines, i + 1));
    }
    // "On <date>, <name> wrote:" and its es/fr/de variants.
    if let Some(n) = attribution_len(lines, i) {
        return Some(n);
    }
    // Outlook header: a "From:" line followed by "Sent:" / "Subject:" within
    // the next few lines. Bare "From:" alone is too easy to false-positive
    // (some bodies talk about "from" addresses), so require the second
    // header line to confirm.
    if (t.starts_with("From:") || t.starts_with("De:") || t.starts_with("Von:"))
        && has_outlook_header_followup(lines, i)
    {
        return Some(header_run(lines, i).max(1));
    }
    None
}

/// Split a normalized body into blocks. Pure; markers only decide where a
/// block starts, never what is removed.
fn segment(text: &str) -> Vec<Block<'_>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if let Some(h) = quote_header_len(&lines, i) {
            // Once a quote is announced, the rest of the body is that message.
            let end = (i + h).min(lines.len());
            blocks.push(Block {
                kind: BlockKind::Quoted,
                header: lines[i..end].to_vec(),
                lines: lines[end..].to_vec(),
            });
            break;
        }
        if t.starts_with('>') {
            let start = i;
            while i < lines.len() {
                let here = lines[i].trim();
                let next_quoted = lines.get(i + 1).is_some_and(|n| n.trim().starts_with('>'));
                if here.starts_with('>') || (here.is_empty() && next_quoted) {
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(Block {
                kind: BlockKind::Quoted,
                header: Vec::new(),
                lines: lines[start..i].to_vec(),
            });
            continue;
        }
        if t == "--" {
            let start = i;
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with('>') && quote_header_len(&lines, i).is_none() {
                i += 1;
            }
            blocks.push(Block {
                kind: BlockKind::Signature,
                header: vec![lines[start]],
                lines: lines[start + 1..i].to_vec(),
            });
            continue;
        }
        let kind = if t.chars().count() <= 60 && is_mobile_stub(t) {
            BlockKind::Stub
        } else {
            BlockKind::Own
        };
        match blocks.last_mut() {
            Some(b) if b.kind == kind && kind == BlockKind::Own => b.lines.push(lines[i]),
            _ => blocks.push(Block {
                kind,
                header: Vec::new(),
                lines: vec![lines[i]],
            }),
        }
        i += 1;
    }
    blocks
}

/// A normalized body without what `history` already contains. Pure.
pub fn new_text(normalized: &str, history: &History) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut seen_here: HashSet<String> = HashSet::new();
    for block in segment(normalized) {
        match block.kind {
            BlockKind::Stub => {}
            BlockKind::Own => {
                let text = block
                    .lines
                    .iter()
                    .map(|l| if l.trim().is_empty() { "" } else { l })
                    .collect::<Vec<_>>()
                    .join("\n");
                let kept: Vec<&str> = text
                    .split("\n\n")
                    .filter(|para| {
                        let key = match_key(para);
                        let repeat = key.chars().count() >= MIN_REPEAT_CHARS
                            && (history.text.contains(&key) || !seen_here.insert(key.clone()));
                        !repeat
                    })
                    .collect();
                parts.push(kept.join("\n\n"));
            }
            BlockKind::Quoted | BlockKind::Signature => {
                if !history.knows(&block.lines.join("\n")) {
                    parts.extend(block.header.iter().chain(block.lines.iter()).map(|l| l.to_string()));
                }
            }
        }
    }
    collapse_whitespace(&parts.join("\n"))
}

/// Only what the sender wrote: no quoted text, signature or stub, known to
/// the thread or not. For samples of how someone writes.
pub fn own_text(body: &str) -> String {
    let normalized = normalize_body(body);
    let own: Vec<&str> = segment(&normalized)
        .into_iter()
        .filter(|b| b.kind == BlockKind::Own)
        .flat_map(|b| b.lines)
        .collect();
    collapse_whitespace(&own.join("\n"))
}

/// If an attribution starts at line `i`, how many lines it spans: one, or up
/// to three when a client wrapped it so its "wrote:" lands on a later line
/// ("On <date>, <name> <addr>" / "[addr]> wrote:").
fn attribution_len(lines: &[&str], i: usize) -> Option<usize> {
    let mut joined = lines[i].trim().to_string();
    if line_is_reply_attribution(&joined) {
        return Some(1);
    }
    for (n, next) in lines.iter().skip(i + 1).take(2).enumerate() {
        let next = next.trim();
        if next.is_empty() {
            return None;
        }
        joined.push(' ');
        joined.push_str(next);
        if line_is_reply_attribution(&joined) {
            return Some(n + 2);
        }
    }
    None
}

fn line_is_reply_attribution(line: &str) -> bool {
    // Lowercased once so we can do plain substring tests. French puts a
    // (non-breaking) space before the colon: "a écrit :".
    let lower = line.to_lowercase();
    let l = match lower.trim_end().strip_suffix(':') {
        Some(head) => format!("{}:", head.trim_end()),
        None => lower,
    };
    let starts_with_attr = l.starts_with("on ") || l.starts_with("el ") || l.starts_with("le ") || l.starts_with("am ");
    if !starts_with_attr {
        return false;
    }
    // German names the sender after the verb: "Am <date> schrieb <name>:".
    if l.starts_with("am ") && l.contains(" schrieb ") && l.ends_with(':') {
        return true;
    }
    // Ends with "wrote:" / "escribió:" / "a écrit:" / "schrieb:".
    l.ends_with("wrote:")
        || l.ends_with("escribió:")
        || l.ends_with("escribio:")
        || l.ends_with("a écrit:")
        || l.ends_with("a ecrit:")
        || l.ends_with("schrieb:")
}

fn has_outlook_header_followup(lines: &[&str], from_idx: usize) -> bool {
    // Look at the next 4 non-empty lines; at least one must be a Sent/Subject/To header.
    let mut seen = 0usize;
    for line in lines.iter().skip(from_idx + 1) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        seen += 1;
        if seen > 4 {
            return false;
        }
        if t.starts_with("Sent:")
            || t.starts_with("Subject:")
            || t.starts_with("To:")
            || t.starts_with("Enviado:")
            || t.starts_with("Asunto:")
            || t.starts_with("Para:")
            || t.starts_with("Gesendet:")
            || t.starts_with("Betreff:")
            || t.starts_with("An:")
        {
            return true;
        }
    }
    false
}

fn is_mobile_stub(line: &str) -> bool {
    let l = line.to_lowercase();
    l.starts_with("sent from my ")
        || l.starts_with("get outlook for ")
        || l.starts_with("enviado desde mi ")
        || l.starts_with("envoyé depuis mon ")
        || l.starts_with("von meinem ")
}

// ── Whitespace + truncation ────────────────────────────────────────────────

fn collapse_whitespace(text: &str) -> String {
    // Collapse runs of blank lines down to a single blank line, trim each
    // line's whitespace, and collapse internal whitespace runs. Preserves
    // paragraph structure (\n\n separation between content blocks).
    let mut out = String::with_capacity(text.len());
    let mut had_blank = false;
    let mut wrote_any = false;
    for line in text.lines() {
        let collapsed: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            if wrote_any {
                had_blank = true;
            }
            continue;
        }
        if wrote_any {
            out.push_str(if had_blank { "\n\n" } else { "\n" });
        }
        out.push_str(&collapsed);
        wrote_any = true;
        had_blank = false;
    }
    out
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    // Char-aware truncation, append "…" marker so the model knows it's cut.
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// `body` read as the next message of a thread whose earlier messages are `earlier`.
    fn reply(body: &str, earlier: &[&str]) -> String {
        let mut history = History::default();
        for e in earlier {
            history.add(&normalize_body(e));
        }
        new_text(&normalize_body(body), &history)
    }

    // ── what the thread already contains is dropped ──

    #[test]
    fn a_gmail_quote_of_an_earlier_message_is_dropped() {
        let body = "Thanks, that works for me.\n\nOn Wed, Apr 15, 2026 at 10:00 AM, Alice <alice@x.com> wrote:\n> Are you free Wednesday?\n> Let me know.";
        assert_eq!(
            reply(body, &["Are you free Wednesday?\nLet me know."]),
            "Thanks, that works for me."
        );
    }

    #[test]
    fn an_outlook_header_block_quoting_an_earlier_message_is_dropped() {
        let body = "Sounds good.\n\nFrom: Alice <alice@x.com>\nSent: Wednesday, April 15, 2026 10:00 AM\nTo: Bob\nSubject: Meeting\n\nAre you free Wednesday?";
        assert_eq!(reply(body, &["Are you free Wednesday?"]), "Sounds good.");
    }

    #[test]
    fn an_original_message_marker_quoting_an_earlier_message_is_dropped() {
        let body = "Replying inline.\n\n-----Original Message-----\nFrom: Alice\nThe original body here.";
        assert_eq!(reply(body, &["The original body here."]), "Replying inline.");
    }

    #[test]
    fn a_spanish_attribution_quoting_an_earlier_message_is_dropped() {
        let body = "Vale, perfecto.\n\nEl mié, 15 abr 2026 a las 10:00, Alice <alice@x.com> escribió:\n> ¿Tienes hueco el miércoles?";
        assert_eq!(reply(body, &["¿Tienes hueco el miércoles?"]), "Vale, perfecto.");
    }

    #[test]
    fn an_html_blockquote_of_earlier_messages_is_dropped() {
        // Apple Mail / Thunderbird quote the previous message in a
        // <blockquote type="cite"> and put nothing recognisable around it.
        let html = "<p>The budget is 4,000 EUR.</p><blockquote type=\"cite\"><p>Could you send me a budget?</p>\
                    <blockquote><p>older</p></blockquote><p>still quoted</p></blockquote>";
        let earlier = "<p>Could you send me a budget?</p><blockquote><p>older</p></blockquote><p>still quoted</p>";
        assert_eq!(reply(html, &[earlier]), "The budget is 4,000 EUR.");
    }

    #[test]
    fn the_whole_email_view_keeps_its_blockquotes() {
        let html = "<p>Answer.</p><blockquote><p>Earlier message.</p></blockquote>";
        assert!(body_to_plain_text(html).contains("Earlier message."));
    }

    #[test]
    fn a_gmail_html_quote_of_an_earlier_message_is_dropped() {
        let html = "<div dir=\"ltr\">The budget is 4,000 EUR.</div><br><div class=\"gmail_quote\">\
                    <div class=\"gmail_attr\">On Mon, 1 Jun 2017 at 10:00, Ana &lt;ana@example.com&gt; wrote:<br></div>\
                    <blockquote class=\"gmail_quote\">Could you send me a budget?</blockquote></div>";
        assert_eq!(
            reply(html, &["Could you send me a budget?"]),
            "The budget is 4,000 EUR."
        );
    }

    #[test]
    fn an_attribution_wrapped_onto_two_lines_is_recognised() {
        // Plain-text Gmail wraps long attributions: the "wrote:" lands on the
        // next line, and a one-line check never saw it.
        let text = "The budget is 4,000 EUR.\n\nEl lun, 1 jun 2017 a las 10:00, Ana Pérez <\nana@example.com> escribió:\n\n> Could you send me a budget?";
        assert_eq!(
            reply(text, &["Could you send me a budget?"]),
            "The budget is 4,000 EUR."
        );
    }

    #[test]
    fn an_attribution_whose_wrote_lands_on_the_next_line_is_recognised() {
        let body = "Sounds good.\n\nOn Wed, 26 Feb 2025 at 10:48, Sam Lee <sam@example.com>\nwrote:\n\n> Are you free?";
        assert_eq!(reply(body, &["Are you free?"]), "Sounds good.");
    }

    #[test]
    fn a_reply_wrapped_in_a_pre_block_is_read_as_html() {
        // Some clients send plain text inside <pre style="white-space:pre-wrap">
        // with its `<`/`>` escaped: the `&gt; ` quote prefix and the escaped
        // address stayed as entities, so no quote marker was ever seen.
        let body = "<pre style=\"white-space:pre-wrap\">Hi,\r\n\r\nJust following up on our earlier email.\r\n\r\n\
                    Best,\r\nSam\r\n\r\nOn Wed, February 26, 2025 10:48 AM, Sam Lee &lt;sam@example.com&gt;\r\n\
                    [sam@example.com]&gt; wrote:\r\n\r\n&gt; Dear team,\r\n&gt;\r\n&gt; We looked at your website.\r\n&gt;\r\n</pre>";
        assert_eq!(
            reply(body, &["Dear team,\n\nWe looked at your website."]),
            "Hi,\n\nJust following up on our earlier email.\n\nBest,\nSam"
        );
    }

    #[test]
    fn a_marker_line_framed_by_dashes_is_recognised() {
        let body = "Replying inline.\n\n-------- Mensaje original --------\nDe: Ana\nOld text.";
        assert_eq!(reply(body, &["Old text."]), "Replying inline.");
    }

    #[test]
    fn a_signature_the_thread_already_showed_is_dropped() {
        let body = "Sounds good — let's do Wednesday at 10.\n\n-- \nAlice Smith\nCEO @ Acme\nalice@x.com";
        let earlier = "Are you free?\n\n-- \nAlice Smith\nCEO @ Acme\nalice@x.com";
        assert_eq!(reply(body, &[earlier]), "Sounds good — let's do Wednesday at 10.");
    }

    #[test]
    fn interleaved_answers_stay_and_the_known_quote_lines_go() {
        let body = "> Could you send me a budget for the portal?\nThe budget is 4,000 EUR.\n> When can you start?\nNext Monday.";
        let earlier = "Could you send me a budget for the portal?\n\nWhen can you start?";
        assert_eq!(reply(body, &[earlier]), "The budget is 4,000 EUR.\nNext Monday.");
    }

    #[test]
    fn a_paragraph_repeated_from_an_earlier_message_is_dropped() {
        let ask = "Could you send me a budget for the customer portal mock-up before Friday?";
        assert_eq!(reply(&format!("Sure, 4,000 EUR.\n\n{ask}"), &[ask]), "Sure, 4,000 EUR.");
    }

    #[test]
    fn own_text_is_what_the_sender_wrote_and_nothing_quoted() {
        // Style samples: how the user writes, never the words they quote.
        let body = "Happy to help, Thursday works.\n\nOn Mon, 1 Jun 2017, Ana wrote:\n> Can we meet?\n\n-- \nUlises\n\nSent from my iPhone";
        assert_eq!(own_text(body), "Happy to help, Thursday works.");
    }

    #[test]
    fn a_thunderbird_quote_of_an_earlier_message_is_dropped() {
        let html = "<p>Sounds good.</p><div class=\"moz-cite-prefix\">On 01/06/2017 10:00, Ana wrote:<br></div>\
                    <blockquote type=\"cite\"><p>Can we meet Thursday afternoon?</p></blockquote>";
        assert_eq!(reply(html, &["Can we meet Thursday afternoon?"]), "Sounds good.");
    }

    #[test]
    fn an_outlook_html_reply_quoting_an_earlier_message_is_dropped() {
        let html = "<div>Perfecto.</div><div id=\"divRplyFwdMsg\"><b>De:</b> Ana &lt;ana@example.com&gt;<br>\
                    <b>Enviado:</b> lunes, 1 de junio de 2017 10:00<br><b>Para:</b> Ulises<br><b>Asunto:</b> Reunión</div>\
                    <div>¿Nos vemos el jueves por la tarde?</div>";
        assert_eq!(reply(html, &["¿Nos vemos el jueves por la tarde?"]), "Perfecto.");
    }

    #[test]
    fn a_french_attribution_with_a_space_before_the_colon_is_recognised() {
        let body =
            "D'accord.\n\nLe lun. 1 juin 2017 à 10:00, Ana <ana@example.com> a écrit\u{a0}:\n> On se voit jeudi ?";
        assert_eq!(reply(body, &["On se voit jeudi ?"]), "D'accord.");
    }

    #[test]
    fn a_german_attribution_with_the_name_after_schrieb_is_recognised() {
        let body = "Passt.\n\nAm 01.06.2017 um 10:00 schrieb Ana <ana@example.com>:\n> Treffen wir uns am Donnerstag?";
        assert_eq!(reply(body, &["Treffen wir uns am Donnerstag?"]), "Passt.");
    }

    #[test]
    fn an_outlook_forward_with_a_note_keeps_the_forwarded_message() {
        let body = "Te paso esto.\n\nDe: Ana <ana@example.com>\nEnviado: lunes, 1 de junio de 2017 10:00\n\
                    Para: Ulises\nAsunto: RV: Presupuesto\n\nEl presupuesto es de 4.000 EUR.";
        let cleaned = clean_email_body(body, 4000);
        assert!(
            cleaned.starts_with("Te paso esto.") && cleaned.contains("4.000 EUR"),
            "{cleaned:?}"
        );
    }

    // ── what the thread does not contain is kept ──

    #[test]
    fn a_quote_of_a_message_not_in_the_thread_is_kept() {
        // A one-message thread: the quoted message was never synced (another
        // account, older than the sync window), so the quote is the only copy.
        let body = "Thanks, that works for me.\n\nOn Wed, Apr 15, 2026 at 10:00 AM, Alice <alice@x.com> wrote:\n> Are you free Wednesday?";
        let cleaned = clean_email_body(body, 4000);
        assert!(cleaned.starts_with("Thanks, that works for me."), "{cleaned:?}");
        assert!(cleaned.contains("Are you free Wednesday?"), "{cleaned:?}");
    }

    #[test]
    fn a_quote_only_partly_in_the_thread_is_kept() {
        let body = "OK.\n\nOn Mon, 1 Jun 2017, Ana wrote:\n> The budget is 4,000 EUR for the portal mock-up.\n\
                    > Also, the hosting will cost another 300 EUR per month from July onwards.";
        let cleaned = reply(body, &["The budget is 4,000 EUR for the portal mock-up."]);
        assert!(cleaned.contains("the hosting will cost another 300 EUR"), "{cleaned:?}");
    }

    #[test]
    fn a_signature_seen_for_the_first_time_is_kept() {
        let body = "Sounds good.\n\n-- \nAlice Smith\nCEO @ Acme\n+34 600 000 000";
        assert!(clean_email_body(body, 4000).contains("+34 600 000 000"));
    }

    #[test]
    fn a_contact_form_footer_is_kept() {
        let body = "De: Sam sam@example.com\nAsunto: Consulta\n\nCuerpo del mensaje:\nHola\n\n--\n\
                    Este mensaje se ha enviado desde un formulario de contacto en Example (https://example.com)";
        assert!(clean_email_body(body, 4000).contains("formulario de contacto en Example"));
    }

    #[test]
    fn a_forward_with_a_note_keeps_the_forwarded_message() {
        let body = "FYI, see below.\n\n---------- Forwarded message ---------\nFrom: Ana <ana@example.com>\n\
                    Subject: Budget\n\nThe budget is 4,000 EUR.";
        let cleaned = clean_email_body(body, 4000);
        assert!(cleaned.starts_with("FYI, see below."), "{cleaned:?}");
        assert!(
            cleaned.contains("ana@example.com") && cleaned.contains("The budget is 4,000 EUR."),
            "{cleaned:?}"
        );
    }

    #[test]
    fn an_apple_mail_forward_in_a_blockquote_is_kept() {
        let html = "<div>FYI</div><div>Begin forwarded message:</div><blockquote type=\"cite\"><div>The budget is 4,000 EUR.</div></blockquote>";
        assert!(clean_email_body(html, 4000).contains("The budget is 4,000 EUR."));
    }

    #[test]
    fn a_form_notification_opening_with_from_and_subject_lines_is_kept() {
        let body = "From: Sam Lee <sam.lee@example.com>\nSubject: Question about availability\n\n\
                    Message Body:\nAre you available from next month?\n\n--\nSent from a contact form";
        let cleaned = clean_email_body(body, 4000);
        assert!(cleaned.contains("sam.lee@example.com"), "{cleaned:?}");
        assert!(cleaned.contains("Are you available from next month?"), "{cleaned:?}");
    }

    #[test]
    fn a_forward_with_no_note_keeps_the_forwarded_message() {
        let body = "---------- Forwarded message ---------\nFrom: Ana <ana@example.com>\nSubject: Budget\n\nThe budget is 4,000 EUR.";
        assert!(clean_email_body(body, 4000).contains("The budget is 4,000 EUR."));
    }

    #[test]
    fn a_plain_text_address_in_angle_brackets_is_kept() {
        let body = "Write to Irene Soto <irene@example.com> about the order.";
        assert_eq!(
            clean_email_body(body, 4000),
            "Write to Irene Soto irene@example.com about the order."
        );
    }

    #[test]
    fn a_marker_phrase_inside_a_sentence_is_prose() {
        let body = "Te reenvío el mensaje original que pediste.\n\nEl presupuesto es de 4.000 EUR.";
        let cleaned = reply(body, &["El presupuesto es de 4.000 EUR. Otra cosa distinta aquí."]);
        assert!(cleaned.starts_with("Te reenvío el mensaje original"), "{cleaned:?}");
    }

    #[test]
    fn strips_mobile_stub() {
        let body = "On my way.\n\nSent from my iPhone";
        let cleaned = clean_email_body(body, 4000);
        assert_eq!(cleaned, "On my way.");
    }

    #[test]
    fn handles_html_input() {
        let body = "<p>Hello <b>there</b>.</p><style>.foo{color:red}</style><p>How are you?</p>";
        let cleaned = clean_email_body(body, 4000);
        assert!(cleaned.contains("Hello"));
        assert!(cleaned.contains("How are you?"));
        assert!(!cleaned.contains("color:red"));
        assert!(!cleaned.contains("<p>"));
    }

    #[test]
    fn truncates_long_bodies_with_marker() {
        let long = "abcdefghij".repeat(500); // 5000 chars
        let cleaned = clean_email_body(&long, 100);
        assert_eq!(cleaned.chars().count(), 100);
        assert!(cleaned.ends_with('…'));
    }

    #[test]
    fn empty_body_yields_empty_cleaned() {
        assert_eq!(clean_email_body("", 4000), "");
        assert_eq!(clean_email_body("   \n  \n", 4000), "");
    }

    #[test]
    fn collapses_repeated_blank_lines() {
        let body = "First.\n\n\n\n\nSecond.\n\n\n\nThird.";
        let cleaned = clean_email_body(body, 4000);
        assert_eq!(cleaned, "First.\n\nSecond.\n\nThird.");
    }

    // ── body_to_plain_text (the `show` full-fidelity de-HTML path) ──────────

    #[test]
    fn body_to_plain_text_strips_html_tags_and_style() {
        let body = "<html><body><p>Hello <b>world</b></p><style>.x{color:red}</style><p>Bye</p></body></html>";
        let out = body_to_plain_text(body);
        assert!(out.contains("Hello world"), "got: {out:?}");
        assert!(out.contains("Bye"));
        assert!(!out.contains('<'), "tags leaked: {out:?}");
        assert!(!out.contains("color:red"), "style leaked: {out:?}");
    }

    #[test]
    fn body_to_plain_text_preserves_quotes_and_signature() {
        // Unlike clean_email_body, `show` must keep the FULL message — quoted
        // replies and signatures included — so the user sees everything.
        let body = "Thanks, that works.\n\nOn Wed, Apr 15, 2026 at 10:00 AM, Alice wrote:\n> Are you free Wednesday?\n\n-- \nAlice Smith";
        let out = body_to_plain_text(body);
        assert!(out.contains("Thanks, that works."));
        assert!(out.contains("Are you free Wednesday?"), "quote stripped: {out:?}");
        assert!(out.contains("Alice Smith"), "signature stripped: {out:?}");
    }

    #[test]
    fn body_to_plain_text_collapses_blank_runs_keeps_paragraphs() {
        assert_eq!(body_to_plain_text("First.\n\n\n\nSecond."), "First.\n\nSecond.");
    }

    #[test]
    fn body_to_plain_text_does_not_truncate_long_bodies() {
        let long = "abcdefghij".repeat(500); // 5000 chars, no tags/blank lines
        let out = body_to_plain_text(&long);
        assert_eq!(out.chars().count(), 5000);
        assert!(!out.ends_with('…'));
    }

    #[test]
    fn body_to_plain_text_passes_plain_text_through() {
        assert_eq!(body_to_plain_text("Just a plain note."), "Just a plain note.");
        assert_eq!(body_to_plain_text(""), "");
    }

    #[test]
    fn summary_chars_per_email_caps_a_single_email_well_below_full_body() {
        // Regression: the "resumen del día" preseed used MAX_CHARS_PER_EMAIL
        // (8000) per row, so one long newsletter ate the whole context and the
        // model summarised only that email. A summary row needs a gist, not the
        // full body — even a single result is capped at the summary ceiling.
        assert_eq!(summary_chars_per_email(1), MAX_SUMMARY_CHARS_PER_EMAIL);
        assert!(summary_chars_per_email(1) < MAX_CHARS_PER_EMAIL);
    }

    #[test]
    fn summary_chars_per_email_fair_shares_the_budget() {
        // No single email can dominate: the more rows, the smaller each excerpt.
        assert_eq!(summary_chars_per_email(5), 1_200);
        assert_eq!(summary_chars_per_email(10), 600);
    }

    #[test]
    fn summary_chars_per_email_floors_for_many_results() {
        // Beyond ~20 rows we stop shrinking so each email keeps a usable gist.
        assert_eq!(summary_chars_per_email(30), MIN_SUMMARY_CHARS_PER_EMAIL);
        assert_eq!(summary_chars_per_email(0), MAX_SUMMARY_CHARS_PER_EMAIL);
    }

    #[test]
    fn strips_newsletter_preheader_spacer_spam() {
        // Substack-style preheader: emoji encoded as numeric refs followed by a
        // long run of invisible spacer chars (combining grapheme joiner U+034F,
        // figure space U+2007, soft hyphen U+00AD) repeated to pad the preview.
        let body = "<p>&#128104;&#8205;&#128187; Hola</p>\
                    <p>&#847;&#8199;&#173;&#847;&#8199;&#173;&#847;&#8199;&#173; Mundo</p>";
        let cleaned = clean_email_body(body, 4000);
        // Numeric entities decode to real glyphs, not literal "&#…;".
        assert!(!cleaned.contains("&#"), "entities must be decoded: {cleaned:?}");
        assert!(cleaned.contains("👨‍💻 Hola"), "emoji+text preserved: {cleaned:?}");
        assert!(
            cleaned.contains("Mundo"),
            "real content after spam survives: {cleaned:?}"
        );
        // The invisible spacer chars must be gone.
        assert!(!cleaned.contains('\u{034F}'), "combining grapheme joiner stripped");
        assert!(!cleaned.contains('\u{00AD}'), "soft hyphen stripped");
    }

    #[test]
    fn preserves_zero_width_joiner_in_emoji_sequences() {
        // U+200D ZERO WIDTH JOINER glues emoji sequences (man + ZWJ + laptop);
        // stripping it would corrupt the glyph, so it must survive.
        let body = "On my way 👨\u{200D}💻";
        let cleaned = clean_email_body(body, 4000);
        assert!(cleaned.contains('\u{200D}'), "ZWJ kept inside emoji: {cleaned:?}");
        assert!(cleaned.contains("👨\u{200D}💻"));
    }
}
