// Pure prose/tool-call gate for token streams from backends that emit
// tool-call *syntax* inline (notably llama.cpp with native OpenAI-compat
// parsing). Such backends stream raw tokens, so a tool call appears in the
// stream as `<tool_call>…`, `<|python_tag|>…`, `[TOOL_CALLS]…`, `<function…`,
// or bare JSON like `{"name":…}` / `[{"name":…}]`. We must never forward that
// syntax to the user-visible stream.
//
// `StreamGate` buffers the leading tokens until it can tell prose from a tool
// call, then either flushes the buffer and streams live (prose) or suppresses
// everything (tool call). Providers whose protocol separates `tool_calls` from
// `content` structurally (Ollama, OpenRouter) do NOT need this — they stream
// content directly and accumulate tool_calls on the side.
//
// This module is intentionally pure (no I/O, no feature gate) so it builds and
// is unit-tested under `make test-fast`, independent of the heavy `llamacpp`
// C++ build that consumes it.

/// Tool-call openers, with insignificant whitespace removed (none of the tag
/// markers contain whitespace; JSON markers are matched after normalisation).
const TOOL_OPENERS: &[&str] = &[
    "<tool_call>",
    "<|python_tag|>",
    "<function",
    "[TOOL_CALLS]",
    "{\"name\"",
    "[{\"name\"",
];

/// Once the normalised leading text reaches this length without matching a tool
/// opener, it cannot be one of the known markers (longest is `<|python_tag|>`,
/// 14 chars) — so it is prose and we flush.
const MAX_GATE_CHARS: usize = 16;

/// Unambiguous tool-call *tag* markers. Unlike the bare-JSON openers in
/// [`TOOL_OPENERS`], these never appear in legitimate prose, so we also watch
/// for them MID-stream (after prose is established): a model may emit real
/// prose and then a tool call in the same turn. Bare JSON is excluded here on
/// purpose — `{"name"…}` shows up in answers that talk about JSON, and we must
/// not truncate those.
const TOOL_TAG_MARKERS: &[&str] = &["<tool_call>", "<|python_tag|>", "[TOOL_CALLS]", "<function"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateState {
    /// Still accumulating the leading text; undecided.
    Buffering,
    /// Decided the turn is prose — forward every chunk live.
    Streaming,
    /// Decided the turn is a tool call — drop every chunk.
    Suppressing,
}

/// Buffer-and-gate state machine. Feed it raw stream chunks via [`push`]; it
/// returns the text that should be forwarded to the user (possibly empty). Call
/// [`finish`] once the stream ends to flush any buffered prose.
///
/// [`push`]: StreamGate::push
/// [`finish`]: StreamGate::finish
#[derive(Debug)]
pub struct StreamGate {
    state: GateState,
    buffer: String,
}

impl Default for StreamGate {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamGate {
    pub fn new() -> Self {
        Self {
            state: GateState::Buffering,
            buffer: String::new(),
        }
    }

    /// Whether the gate has decided this turn is a tool call (all prose suppressed).
    pub fn is_suppressed(&self) -> bool {
        self.state == GateState::Suppressing
    }

    /// Feed one stream chunk. Returns the text to forward to the user now
    /// (empty while buffering or while suppressing a tool call).
    pub fn push(&mut self, chunk: &str) -> String {
        match self.state {
            GateState::Streaming => {
                // Prose already established, but the model can still emit a
                // tool-call tag after it (`…done.<tool_call>…`). `self.buffer`
                // holds any trailing partial-tag prefix carried over from the
                // previous chunk so a tag split across chunks isn't missed.
                let mut combined = std::mem::take(&mut self.buffer);
                combined.push_str(chunk);
                self.stream_through(combined)
            }
            GateState::Suppressing => String::new(),
            GateState::Buffering => {
                self.buffer.push_str(chunk);
                let normalized = normalize(&self.buffer);

                // Only whitespace so far: keep buffering until prose arrives, or
                // give up and treat the whitespace as prose once it gets long.
                if normalized.is_empty() {
                    if self.buffer.chars().count() >= MAX_GATE_CHARS {
                        self.state = GateState::Streaming;
                        let buffered = std::mem::take(&mut self.buffer);
                        return self.stream_through(buffered);
                    }
                    return String::new();
                }

                if matches_opener(&normalized) {
                    self.state = GateState::Suppressing;
                    self.buffer.clear();
                    return String::new();
                }

                if could_become_opener(&normalized) && normalized.chars().count() < MAX_GATE_CHARS {
                    return String::new();
                }

                // Not a leading tool opener and can't become one → prose. Flush,
                // but route through the mid-stream gate so a trailing partial tag
                // (e.g. prose ending in `<`) is held back rather than leaked.
                self.state = GateState::Streaming;
                let buffered = std::mem::take(&mut self.buffer);
                self.stream_through(buffered)
            }
        }
    }

    /// Forward prose while watching for a mid-stream tool-call tag. Emits the
    /// text up to the earliest complete tag marker (and switches to Suppressing
    /// when one is found), otherwise holds back a trailing partial-tag prefix in
    /// `self.buffer` so a tag split across chunk boundaries is caught next time.
    /// Only callable once `self.state` is `Streaming`.
    fn stream_through(&mut self, text: String) -> String {
        if let Some(pos) = earliest_tag_marker(&text) {
            self.state = GateState::Suppressing;
            self.buffer.clear();
            return text[..pos].to_string();
        }
        let hold = trailing_partial_tag_len(&text);
        let split = text.len() - hold;
        self.buffer = text[split..].to_string();
        text[..split].to_string()
    }

    /// Flush at end of stream. While buffering a partial leading tool-opener
    /// prefix (a truncated tool call), suppress it; otherwise emit buffered
    /// prose. In `Streaming` state, any held tail is by construction a partial
    /// tag prefix — truncated markup at EOF — so drop it.
    pub fn finish(&mut self) -> String {
        match self.state {
            GateState::Suppressing => String::new(),
            GateState::Streaming => {
                // Held tail is always a partial tag prefix → truncated markup.
                self.buffer.clear();
                String::new()
            }
            GateState::Buffering => {
                let normalized = normalize(&self.buffer);
                if !normalized.is_empty() && could_become_opener(&normalized) {
                    // Truncated tool-call syntax at EOF — never surface as prose.
                    self.state = GateState::Suppressing;
                    self.buffer.clear();
                    return String::new();
                }
                self.state = GateState::Streaming;
                std::mem::take(&mut self.buffer)
            }
        }
    }
}

/// Byte offset of the earliest complete tool-call tag marker in `s`, if any.
fn earliest_tag_marker(s: &str) -> Option<usize> {
    TOOL_TAG_MARKERS.iter().filter_map(|m| s.find(m)).min()
}

/// Length (in bytes) of the longest suffix of `s` that is a strict prefix of
/// some tag marker — i.e. the start of a tag that may be completed by the next
/// chunk. Tag markers are ASCII, so any matching suffix is ASCII and the split
/// point lands on a char boundary. Returns 0 when no suffix could start a tag.
fn trailing_partial_tag_len(s: &str) -> usize {
    let max = TOOL_TAG_MARKERS
        .iter()
        .map(|m| m.len())
        .max()
        .unwrap_or(0)
        .saturating_sub(1);
    let start = s.len().saturating_sub(max);
    for i in start..s.len() {
        if !s.is_char_boundary(i) {
            continue;
        }
        let suffix = &s[i..];
        if TOOL_TAG_MARKERS
            .iter()
            .any(|m| m.len() > suffix.len() && m.starts_with(suffix))
        {
            return suffix.len();
        }
    }
    0
}

/// Drop all Unicode whitespace. Tag markers contain none; JSON whitespace is
/// insignificant, so this lets `{ "name" }` match `{"name"`.
fn normalize(s: &str) -> String {
    s.trim_start().chars().filter(|c| !c.is_whitespace()).collect()
}

/// True when the normalised leading text already begins with a full tool opener.
fn matches_opener(normalized: &str) -> bool {
    TOOL_OPENERS.iter().any(|m| normalized.starts_with(m))
}

/// True when the normalised leading text is a strict prefix of some opener and
/// so might still resolve into one with more tokens.
fn could_become_opener(normalized: &str) -> bool {
    TOOL_OPENERS
        .iter()
        .any(|m| m.starts_with(normalized) && m.len() > normalized.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the gate through a sequence of chunks and collect everything it
    /// forwarded, including the final flush.
    fn run(chunks: &[&str]) -> (String, bool) {
        let mut gate = StreamGate::new();
        let mut out = String::new();
        for c in chunks {
            out.push_str(&gate.push(c));
        }
        out.push_str(&gate.finish());
        (out, gate.is_suppressed())
    }

    #[test]
    fn plain_prose_streams_through() {
        let (out, suppressed) = run(&["Hello ", "world", "!"]);
        assert_eq!(out, "Hello world!");
        assert!(!suppressed);
    }

    #[test]
    fn prose_preserves_leading_whitespace() {
        let (out, _) = run(&["  ", "Hello"]);
        assert_eq!(out, "  Hello");
    }

    #[test]
    fn whole_tool_call_tag_is_suppressed() {
        let (out, suppressed) = run(&["<tool_call>", "{\"name\":\"x\"}", "</tool_call>"]);
        assert_eq!(out, "");
        assert!(suppressed);
    }

    #[test]
    fn tool_call_tag_split_across_chunks_is_suppressed() {
        let (out, suppressed) = run(&["<", "tool_", "call>{\"name\":\"x\"}"]);
        assert_eq!(out, "");
        assert!(suppressed);
    }

    #[test]
    fn python_tag_split_is_suppressed() {
        let (out, suppressed) = run(&["<|", "python", "_tag|>do()"]);
        assert_eq!(out, "");
        assert!(suppressed);
    }

    #[test]
    fn mistral_tool_calls_marker_is_suppressed() {
        let (out, suppressed) = run(&["[TOOL", "_CALLS]", "[{\"name\":\"x\"}]"]);
        assert_eq!(out, "");
        assert!(suppressed);
    }

    #[test]
    fn function_marker_is_suppressed() {
        let (out, suppressed) = run(&["<function", "=search>"]);
        assert_eq!(out, "");
        assert!(suppressed);
    }

    #[test]
    fn bare_json_tool_call_is_suppressed() {
        let (out, suppressed) = run(&["{\"name\":", "\"search\",\"arguments\":{}}"]);
        assert_eq!(out, "");
        assert!(suppressed);
    }

    #[test]
    fn json_tool_call_with_whitespace_is_suppressed() {
        let (out, suppressed) = run(&["{ \"name\" : ", "\"search\" }"]);
        assert_eq!(out, "");
        assert!(suppressed);
    }

    #[test]
    fn json_array_tool_call_is_suppressed() {
        let (out, suppressed) = run(&["[{\"name\":\"x\"}]"]);
        assert_eq!(out, "");
        assert!(suppressed);
    }

    #[test]
    fn prose_starting_with_angle_bracket_streams() {
        // `<3` is not a tool marker prefix → should flush as prose.
        let (out, suppressed) = run(&["<3 ", "love it"]);
        assert_eq!(out, "<3 love it");
        assert!(!suppressed);
    }

    #[test]
    fn prose_starting_with_brace_but_not_name_streams() {
        let (out, suppressed) = run(&["{", "this is text}"]);
        assert_eq!(out, "{this is text}");
        assert!(!suppressed);
    }

    #[test]
    fn truncated_tool_marker_at_eof_is_suppressed() {
        let (out, suppressed) = run(&["<tool_"]);
        assert_eq!(out, "");
        assert!(suppressed);
    }

    #[test]
    fn short_prose_flushes_on_finish() {
        // `Hi` is decided as prose on first push (not a marker prefix).
        let (out, suppressed) = run(&["Hi"]);
        assert_eq!(out, "Hi");
        assert!(!suppressed);
    }

    #[test]
    fn once_streaming_passes_everything_including_brace() {
        // After prose is established, a later `{` must not re-trigger gating.
        let mut gate = StreamGate::new();
        let mut out = String::new();
        out.push_str(&gate.push("Here is JSON: "));
        out.push_str(&gate.push("{\"name\":\"x\"}"));
        out.push_str(&gate.finish());
        assert_eq!(out, "Here is JSON: {\"name\":\"x\"}");
        assert!(!gate.is_suppressed());
    }

    // ── Mid-stream tool-call markup (prose, THEN a tool call) ───────────────
    //
    // Small models sometimes emit genuine prose and then a tool-call tag in the
    // same turn ("Let me look that up.<tool_call>…"). We must forward the prose
    // and suppress everything from the tag onward — the leading-only gate used
    // to pass the tag through once prose was established.

    #[test]
    fn mid_stream_tool_tag_after_prose_is_cut() {
        let (out, suppressed) = run(&["Here is your summary. ", "<tool_call>{\"name\":\"x\"}</tool_call>"]);
        assert_eq!(out, "Here is your summary. ");
        assert!(suppressed);
    }

    #[test]
    fn mid_stream_function_marker_after_prose_is_cut() {
        let (out, suppressed) = run(&["Summary: ", "<function=get_email_body>{}"]);
        assert_eq!(out, "Summary: ");
        assert!(suppressed);
    }

    #[test]
    fn mid_stream_tool_tag_split_across_chunks_is_cut() {
        // The `<` that begins the tag arrives glued to prose; the rest follows
        // in the next chunk. The gate must hold the partial prefix back.
        let (out, suppressed) = run(&["Done. <", "tool_call>{}"]);
        assert_eq!(out, "Done. ");
        assert!(suppressed);
    }

    #[test]
    fn mid_stream_python_tag_split_is_cut() {
        let (out, suppressed) = run(&["Result ready <|", "python_tag|>foo()"]);
        assert_eq!(out, "Result ready ");
        assert!(suppressed);
    }

    #[test]
    fn mid_stream_bare_json_after_prose_still_streams() {
        // Bare JSON is NOT a tag marker — once prose is established it must
        // pass through unchanged (locks in `once_streaming_passes_everything`).
        let (out, suppressed) = run(&["The shape is ", "{\"name\": \"x\"} ok"]);
        assert_eq!(out, "The shape is {\"name\": \"x\"} ok");
        assert!(!suppressed);
    }

    #[test]
    fn mid_stream_angle_bracket_prose_is_preserved() {
        // `<3` mid-stream is not a tag prefix once enough follows — must stream.
        let (out, suppressed) = run(&["I love it ", "<3 so much"]);
        assert_eq!(out, "I love it <3 so much");
        assert!(!suppressed);
    }

    #[test]
    fn mid_stream_partial_tag_prefix_at_eof_is_dropped() {
        // Prose then a dangling, truncated tag prefix at EOF — drop the markup.
        let (out, _suppressed) = run(&["All set. ", "<tool_"]);
        assert_eq!(out, "All set. ");
    }
}
