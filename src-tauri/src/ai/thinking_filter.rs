// Pure reasoning/thinking filter for token streams from local backends that
// emit a model's "thinking" prelude inline as plain text.
//
// Unlike tool-call syntax (handled by `stream_gate`), some instruction-tuned
// models wrap their internal reasoning in marker tags that leak straight into
// the user-visible answer when the runtime doesn't strip them:
//   • Gemma 4   — `<|channel>thought\n … \n<channel|>`
//   • Qwen 3 /
//     DeepSeek  — `<think> … </think>`
// These markers are NEVER legitimate prose, so any complete reasoning span (and
// any orphan/dangling marker) must be removed before the text reaches the user.
//
// Two entry points:
//   • `strip_reasoning` — one-shot cleanup of a finished assistant string.
//   • `ThinkingGate`    — buffer-and-suppress state machine for live streaming,
//     mirroring `StreamGate`: feed raw chunks via `push`, flush with `finish`.
//
// This module is intentionally pure (no I/O, no feature gate) so it builds and
// is unit-tested under `make test-fast`, independent of the heavy `llamacpp`
// C++ build that consumes it.

/// Reasoning marker pairs, as `(open, close)`. Order is irrelevant — the spans
/// are disjoint in practice and each pair is stripped independently.
const MARKER_PAIRS: &[(&str, &str)] = &[("<|channel>", "<channel|>"), ("<think>", "</think>")];

/// Remove every complete reasoning span and any orphan marker from a finished
/// assistant string, then trim surrounding whitespace.
///
/// For each `(open, close)` pair this drops `open … close` spans, a dangling
/// `open …` with no closer (truncated reasoning), and a stray `close` with no
/// opener. The logic mirrors the official Gemma 4 chat template's
/// `strip_thinking` macro so prior-turn and live-output handling agree.
pub fn strip_reasoning(text: &str) -> String {
    let mut out = text.to_string();
    for (open, close) in MARKER_PAIRS {
        out = strip_pair(&out, open, close);
    }
    out.trim().to_string()
}

/// Remove `open … close` spans (and orphan markers) for a single marker pair.
///
/// Splitting on `close` drops every closer; for any segment that contains an
/// `open`, we keep only the text before it — discarding the opener and the
/// reasoning that follows it up to the (already-consumed) closer. A dangling
/// `open` with no closer is handled by the same rule on the final segment.
fn strip_pair(text: &str, open: &str, close: &str) -> String {
    let mut result = String::new();
    for segment in text.split(close) {
        match segment.find(open) {
            Some(idx) => result.push_str(&segment[..idx]),
            None => result.push_str(segment),
        }
    }
    result
}

/// Buffer-and-suppress state machine for live streaming. Feed it raw stream
/// chunks via [`push`]; it returns the text that should be forwarded to the
/// user (reasoning spans removed). Call [`finish`] at end of stream to flush any
/// held prose and drop truncated markup.
///
/// [`push`]: ThinkingGate::push
/// [`finish`]: ThinkingGate::finish
#[derive(Debug, Default)]
pub struct ThinkingGate {
    suppressing: bool,
    pending: String,
}

impl ThinkingGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one stream chunk. Returns the text to forward now (reasoning spans
    /// suppressed, partial trailing markers held back for the next chunk).
    pub fn push(&mut self, chunk: &str) -> String {
        let mut buf = std::mem::take(&mut self.pending);
        buf.push_str(chunk);
        let mut out = String::new();

        loop {
            if self.suppressing {
                // Inside a reasoning span: discard until the matching closer.
                if let Some((pos, len)) = earliest_marker(&buf, CLOSE_MARKERS) {
                    buf.drain(..pos + len);
                    self.suppressing = false;
                    continue;
                }
                // No closer yet — drop everything but a trailing partial closer.
                let hold = trailing_partial_len(&buf, CLOSE_MARKERS);
                self.pending = buf[buf.len() - hold..].to_string();
                break;
            }

            // Forwarding: emit prose up to the next reasoning opener.
            if let Some((pos, len)) = earliest_marker(&buf, OPEN_MARKERS) {
                out.push_str(&buf[..pos]);
                buf.drain(..pos + len);
                self.suppressing = true;
                continue;
            }
            // No opener — forward all but a trailing partial opener.
            let hold = trailing_partial_len(&buf, OPEN_MARKERS);
            let split = buf.len() - hold;
            out.push_str(&buf[..split]);
            self.pending = buf[split..].to_string();
            break;
        }

        out
    }

    /// Flush at end of stream. A held tail is either a truncated reasoning span
    /// (suppressing) or a partial opener prefix (forwarding) — both are
    /// truncated markup, so drop them.
    pub fn finish(&mut self) -> String {
        self.pending.clear();
        String::new()
    }
}

/// Reasoning opener markers (kept in sync with [`MARKER_PAIRS`]).
const OPEN_MARKERS: &[&str] = &["<|channel>", "<think>"];
/// Reasoning closer markers (kept in sync with [`MARKER_PAIRS`]).
const CLOSE_MARKERS: &[&str] = &["<channel|>", "</think>"];

/// Byte offset and length of the earliest complete marker from `markers` in `s`.
fn earliest_marker(s: &str, markers: &[&str]) -> Option<(usize, usize)> {
    markers
        .iter()
        .filter_map(|m| s.find(m).map(|pos| (pos, m.len())))
        .min_by_key(|(pos, _)| *pos)
}

/// Length (bytes) of the longest suffix of `s` that is a strict prefix of some
/// marker — i.e. the start of a marker that the next chunk may complete. Markers
/// are ASCII, so a matching suffix lands on a char boundary.
fn trailing_partial_len(s: &str, markers: &[&str]) -> usize {
    let max = markers.iter().map(|m| m.len()).max().unwrap_or(0).saturating_sub(1);
    let start = s.len().saturating_sub(max);
    for i in start..s.len() {
        if !s.is_char_boundary(i) {
            continue;
        }
        let suffix = &s[i..];
        if markers.iter().any(|m| m.len() > suffix.len() && m.starts_with(suffix)) {
            return suffix.len();
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_reasoning (one-shot) ──────────────────────────────────────────

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(strip_reasoning("Hello world"), "Hello world");
    }

    #[test]
    fn gemma_channel_span_is_removed() {
        let input = "<|channel>thought\nLet me check the inbox.\n<channel|>You have 3 new emails.";
        assert_eq!(strip_reasoning(input), "You have 3 new emails.");
    }

    #[test]
    fn qwen_think_span_is_removed() {
        let input = "<think>\nThe user asked for the latest email.\n</think>\nThe latest email is from Jorge.";
        assert_eq!(strip_reasoning(input), "The latest email is from Jorge.");
    }

    #[test]
    fn multiple_channel_spans_are_removed() {
        let input = "<|channel>thought\na<channel|>Answer one. <|channel>thought\nb<channel|>Answer two.";
        assert_eq!(strip_reasoning(input), "Answer one. Answer two.");
    }

    #[test]
    fn dangling_open_marker_is_removed() {
        // Truncated reasoning with no closer — keep only the prefix before it.
        let input = "Here is the result. <|channel>thought\nstill thinking";
        assert_eq!(strip_reasoning(input), "Here is the result.");
    }

    #[test]
    fn orphan_close_marker_is_removed() {
        let input = "No opener here<channel|>but a stray closer.";
        assert_eq!(strip_reasoning(input), "No opener herebut a stray closer.");
    }

    #[test]
    fn empty_thought_block_is_removed() {
        // Gemma 4 with thinking disabled still emits an empty wrapper.
        let input = "<|channel>thought\n<channel|>The answer.";
        assert_eq!(strip_reasoning(input), "The answer.");
    }

    #[test]
    fn text_without_markers_keeps_internal_whitespace() {
        assert_eq!(strip_reasoning("a  b\nc"), "a  b\nc");
    }

    // ── ThinkingGate (streaming) ────────────────────────────────────────────

    /// Drive the gate through a sequence of chunks and collect everything it
    /// forwarded, including the final flush.
    fn run(chunks: &[&str]) -> String {
        let mut gate = ThinkingGate::new();
        let mut out = String::new();
        for c in chunks {
            out.push_str(&gate.push(c));
        }
        out.push_str(&gate.finish());
        out
    }

    #[test]
    fn stream_plain_prose_passes_through() {
        assert_eq!(run(&["Hello ", "world", "!"]), "Hello world!");
    }

    #[test]
    fn stream_leading_channel_block_is_suppressed() {
        assert_eq!(
            run(&["<|channel>thought\n", "reasoning here", "\n<channel|>", "The answer."]),
            "The answer."
        );
    }

    #[test]
    fn stream_think_block_is_suppressed() {
        assert_eq!(run(&["<think>", "reasoning", "</think>", "Answer."]), "Answer.");
    }

    #[test]
    fn stream_open_marker_split_across_chunks_is_suppressed() {
        assert_eq!(run(&["<|chan", "nel>thought\nx<channel|>", "Done."]), "Done.");
    }

    #[test]
    fn stream_close_marker_split_across_chunks_is_suppressed() {
        assert_eq!(run(&["<think>reasoning</thi", "nk>Answer."]), "Answer.");
    }

    #[test]
    fn stream_prose_then_thought_then_prose() {
        assert_eq!(
            run(&["Sure. ", "<|channel>thought\nx\n<channel|>", "Here it is."]),
            "Sure. Here it is."
        );
    }

    #[test]
    fn stream_dangling_open_at_eof_is_dropped() {
        assert_eq!(run(&["All set. ", "<|channel>thought\ntrunc"]), "All set. ");
    }

    #[test]
    fn stream_partial_open_prefix_at_eof_is_dropped() {
        // A trailing `<` that could begin a marker is held, then dropped at EOF.
        assert_eq!(run(&["Done.", "<"]), "Done.");
    }

    #[test]
    fn stream_angle_bracket_prose_is_preserved() {
        // `<3` cannot complete any marker → must stream once enough follows.
        assert_eq!(run(&["I love it ", "<3 so much"]), "I love it <3 so much");
    }
}
