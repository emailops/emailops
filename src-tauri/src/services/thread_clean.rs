//! Clean an email thread for use as chat context.
//!
//! Email bodies, especially in long threads, contain a lot of noise that wastes
//! the LLM's context window: quoted reply blocks, signatures, "Sent from my
//! iPhone" stubs, legal disclaimers, tracking pixels, repeated boilerplate.
//! This module strips that noise so the model sees just the substantive
//! conversation.
//!
//! Pipeline per body:
//!   1. HTML → text (reuses [`strip_html_for_fts`]).
//!   2. Drop quoted reply blocks (Gmail/Outlook headers, `>`-prefixed lines).
//!   3. Drop the signature block (RFC 3676 `-- ` delimiter, common stubs).
//!   4. Collapse whitespace.
//!   5. Cap length per email at `max_chars`.

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
    // Quoted history in <blockquote> is someone else's earlier message. Only
    // here: `body_to_plain_text` shows the whole email, quotes included.
    let body = if looks_like_html(body) {
        std::borrow::Cow::Owned(drop_blockquotes(body))
    } else {
        std::borrow::Cow::Borrowed(body)
    };
    let text = to_plain_text(&body);
    let visible = strip_invisible_chars(&text);
    let de_quoted = strip_quoted_replies(&visible);
    let de_signed = strip_signature(&de_quoted);
    let collapsed = collapse_whitespace(&de_signed);
    truncate_chars(&collapsed, max_chars)
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

/// Remove every `<blockquote>…</blockquote>` element, nested ones included.
/// Mail clients quote the previous message this way — often with no
/// attribution line the text-level cut could see. Depth-counted rather than a
/// lazy regex, which would leak the tail of an outer quote after an inner one.
fn drop_blockquotes(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut depth = 0usize;
    let mut i = 0;
    while i < html.len() {
        let rest = &lower[i..];
        if rest.starts_with("<blockquote") && rest[11..].starts_with(|c: char| c == '>' || c.is_whitespace()) {
            depth += 1;
            i += rest.find('>').map_or(rest.len(), |p| p + 1);
            continue;
        }
        if rest.starts_with("</blockquote") && depth > 0 {
            depth -= 1;
            i += rest.find('>').map_or(rest.len(), |p| p + 1);
            continue;
        }
        let Some(ch) = html[i..].chars().next() else { break };
        if depth == 0 {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    out
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

// ── Quoted-reply stripping ─────────────────────────────────────────────────

/// Drop quoted-reply blocks. Matches:
///   - "On {date}, {Name} <{email}> wrote:" (Gmail-style, multi-locale)
///   - "From: ... Sent: ... To: ... Subject: ..." (Outlook header block)
///   - "-----Original Message-----" + locale variants
///   - Contiguous lines starting with `>` (RFC 3676 quote prefix)
///
/// Heuristic, not perfect: we cut at the *first* match and drop everything
/// after, since once a reply quote starts the rest of the body is almost always
/// quoted history. A match with nothing written above it is not a reply's
/// quote — a forward with no note, or a contact-form notification that opens
/// with the visitor's From:/Subject: — so it never cuts.
fn strip_quoted_replies(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();

    // A line that is only one of these (dashes around it allowed) starts the
    // quoted history. Matched on the whole line: the words inside a sentence
    // ("te reenvío el mensaje original") are prose, not a marker.
    let original_markers: [&str; 5] = [
        "original message",
        "forwarded message",
        "mensaje original",
        "mensaje reenviado",
        "ursprüngliche nachricht",
    ];

    let mut cut_at: Option<usize> = None;
    let mut own_content = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();
        let marker = lower.trim_matches(|c: char| c == '-' || c.is_whitespace());
        let starts_quote = original_markers.contains(&marker)
            // "On <date>, <name> wrote:" / "On <date> at <time>, <name> wrote:"
            // Localised variants: "El <date>, <name> escribió:", "Le <date>, ... a écrit:",
            // "Am <date> schrieb <name>:".
            || attribution_starts_at(&lines, i)
            // Outlook header: a "From:" line followed by "Sent:" / "Subject:" within
            // the next few lines. Bare "From:" alone is too easy to false-positive
            // (some bodies talk about "from" addresses), so require the second
            // header line to confirm.
            || ((trimmed.starts_with("From:") || trimmed.starts_with("De:") || trimmed.starts_with("Von:"))
                && has_outlook_header_followup(&lines, i));
        if starts_quote && own_content {
            cut_at = Some(i);
            break;
        }
        // The marker line of an uncut forward is not the sender's own text.
        own_content |= !starts_quote;
    }

    let kept: Vec<&str> = match cut_at {
        Some(i) => lines[..i].to_vec(),
        None => lines,
    };

    // Drop any trailing run of `>`-prefixed lines (quote prefix style).
    let mut end = kept.len();
    while end > 0 {
        let l = kept[end - 1].trim_start();
        if l.starts_with('>') || l.is_empty() {
            end -= 1;
        } else {
            break;
        }
    }
    kept[..end].join("\n")
}

/// An attribution line, or one a client wrapped so its "wrote:" lands on one
/// of the next two lines ("On <date>, <name> <addr>" / "[addr]> wrote:").
fn attribution_starts_at(lines: &[&str], i: usize) -> bool {
    let mut joined = lines[i].trim().to_string();
    if line_is_reply_attribution(&joined) {
        return true;
    }
    for next in lines.iter().skip(i + 1).take(2) {
        let next = next.trim();
        if next.is_empty() {
            return false;
        }
        joined.push(' ');
        joined.push_str(next);
        if line_is_reply_attribution(&joined) {
            return true;
        }
    }
    false
}

fn line_is_reply_attribution(line: &str) -> bool {
    // Lowercased once so we can do plain substring tests.
    let l = line.to_lowercase();
    let starts_with_attr = l.starts_with("on ") || l.starts_with("el ") || l.starts_with("le ") || l.starts_with("am ");
    if !starts_with_attr {
        return false;
    }
    // Ends with "wrote:" / "escribió:" / "a écrit:" / "schrieb:"  …possibly
    // with a stray hard space or unicode quote char.
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

// ── Signature stripping ────────────────────────────────────────────────────

/// Drop the signature. Recognises:
///   - "\n-- \n" (RFC 3676 sig delimiter — note the trailing space on `--`).
///   - Trailing short blocks containing "Sent from my iPhone" / similar.
fn strip_signature(text: &str) -> String {
    // RFC 3676 — split at the FIRST occurrence so multi-stamped sigs don't slip through.
    if let Some(pos) = text.find("\n-- \n") {
        return text[..pos].to_string();
    }
    if let Some(pos) = text.find("\n--\n") {
        // Tolerate the variant without trailing space (common in plain-text
        // emails sent through clients that strip trailing whitespace).
        return text[..pos].to_string();
    }

    // Tail-stub heuristic: the last non-empty line matches a short canned phrase.
    let mut lines: Vec<&str> = text.lines().collect();
    while let Some(last) = lines.last() {
        let l = last.trim();
        if l.is_empty() {
            lines.pop();
            continue;
        }
        if is_mobile_stub(l) {
            lines.pop();
            continue;
        }
        break;
    }
    lines.join("\n")
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

    #[test]
    fn strips_gmail_quoted_reply() {
        let body = "Thanks, that works for me.\n\nOn Wed, Apr 15, 2026 at 10:00 AM, Alice <alice@x.com> wrote:\n> Are you free Wednesday?\n> Let me know.";
        let cleaned = clean_email_body(body, 4000);
        assert_eq!(cleaned, "Thanks, that works for me.");
    }

    #[test]
    fn strips_outlook_header_block() {
        let body = "Sounds good.\n\nFrom: Alice <alice@x.com>\nSent: Wednesday, April 15, 2026 10:00 AM\nTo: Bob\nSubject: Meeting\n\nAre you free Wednesday?";
        let cleaned = clean_email_body(body, 4000);
        assert_eq!(cleaned, "Sounds good.");
    }

    #[test]
    fn strips_original_message_marker() {
        let body = "Replying inline.\n\n-----Original Message-----\nFrom: Alice\nThe original body here.";
        let cleaned = clean_email_body(body, 4000);
        assert_eq!(cleaned, "Replying inline.");
    }

    #[test]
    fn strips_spanish_attribution() {
        let body = "Vale, perfecto.\n\nEl mié, 15 abr 2026 a las 10:00, Alice <alice@x.com> escribió:\n> ¿Tienes hueco el miércoles?";
        let cleaned = clean_email_body(body, 4000);
        assert_eq!(cleaned, "Vale, perfecto.");
    }

    #[test]
    fn strips_an_html_blockquote_even_without_an_attribution_line() {
        // Apple Mail / Thunderbird quote the previous message in a
        // <blockquote type="cite"> and put nothing recognisable around it.
        let html = "<p>The budget is 4,000 EUR.</p><blockquote type=\"cite\"><p>Could you send me a budget?</p>\
                    <blockquote><p>older</p></blockquote><p>still quoted</p></blockquote>";
        assert_eq!(clean_email_body(html, 1000), "The budget is 4,000 EUR.");
    }

    #[test]
    fn the_whole_email_view_keeps_its_blockquotes() {
        let html = "<p>Answer.</p><blockquote><p>Earlier message.</p></blockquote>";
        assert!(body_to_plain_text(html).contains("Earlier message."));
    }

    #[test]
    fn strips_gmail_html_quote() {
        let html = "<div dir=\"ltr\">The budget is 4,000 EUR.</div><br><div class=\"gmail_quote\">\
                    <div class=\"gmail_attr\">On Mon, 1 Jun 2017 at 10:00, Ana &lt;ana@example.com&gt; wrote:<br></div>\
                    <blockquote class=\"gmail_quote\">Could you send me a budget?</blockquote></div>";
        assert_eq!(clean_email_body(html, 1000), "The budget is 4,000 EUR.");
    }

    #[test]
    fn strips_an_attribution_wrapped_onto_two_lines() {
        // Plain-text Gmail wraps long attributions: the "wrote:" lands on the
        // next line, and a one-line check never saw it.
        let text = "The budget is 4,000 EUR.\n\nEl lun, 1 jun 2017 a las 10:00, Ana Pérez <\nana@example.com> escribió:\n\n> Could you send me a budget?";
        assert_eq!(clean_email_body(text, 1000), "The budget is 4,000 EUR.");
    }

    #[test]
    fn a_form_notification_opening_with_from_and_subject_lines_is_kept() {
        // Contact-form plugins put the visitor's From:/Subject: at the top of
        // the body. With nothing written above it, it is not a quoted reply.
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
    fn a_marker_phrase_inside_a_sentence_does_not_cut_the_body() {
        let body = "Te reenvío el mensaje original que pediste.\n\nEl presupuesto es de 4.000 EUR.";
        let cleaned = clean_email_body(body, 4000);
        assert!(cleaned.contains("El presupuesto es de 4.000 EUR."), "{cleaned:?}");
    }

    #[test]
    fn a_marker_line_framed_by_dashes_still_cuts() {
        let body = "Replying inline.\n\n-------- Mensaje original --------\nDe: Ana\nOld text.";
        assert_eq!(clean_email_body(body, 4000), "Replying inline.");
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
            clean_email_body(body, 4000),
            "Hi,\n\nJust following up on our earlier email.\n\nBest,\nSam"
        );
    }

    #[test]
    fn an_attribution_whose_wrote_lands_on_the_next_line_is_cut() {
        let body = "Sounds good.\n\nOn Wed, 26 Feb 2025 at 10:48, Sam Lee <sam@example.com>\nwrote:\n\n> Are you free?";
        assert_eq!(clean_email_body(body, 4000), "Sounds good.");
    }

    #[test]
    fn strips_rfc_signature() {
        let body = "Sounds good — let's do Wednesday at 10.\n\n-- \nAlice Smith\nCEO @ Acme\nalice@x.com";
        let cleaned = clean_email_body(body, 4000);
        assert_eq!(cleaned, "Sounds good — let's do Wednesday at 10.");
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
