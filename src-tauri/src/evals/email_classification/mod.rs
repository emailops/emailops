// Eval harness for the `distil-labs/distil-email-classifier` Hugging Face
// model (Qwen3-0.6B distilled into a 10-way email classifier).
//
// Unlike the existing `services::classification` pipeline (which produces
// intent + topic + urgency JSON via a structured prompt), the distil
// classifier outputs ONE label from a fixed 10-way taxonomy:
//
//   Billing · Newsletter · Work · Personal · Promotional ·
//   Security · Shipping · Travel · Spam · Other
//
// The model expects raw email text (subject + body/snippet) and returns
// the label. We hit it via Ollama (local) — see
// `src-tauri/evals/email_classification/README.md` for the one-time
// `ollama create` setup.
//
// This eval is *descriptive* by default: with no ground-truth labels in the
// DB, we record the prediction, latency, and label distribution. An
// optional LLM-as-judge mode (OpenRouter) can score whether each predicted
// label is reasonable for the email.

pub mod report;
pub mod runner;

/// The 10-way label set fine-tuned into the distil classifier. The model
/// emits these prefixed with "AI/" inside an `<output>` block (per the
/// reference `model_client.py` shipped with the model). The order is the
/// canonical priority order from the system prompt and keeps the report's
/// distribution histogram stable.
pub const LABELS: &[&str] = &[
    "Billing",
    "Newsletter",
    "Work",
    "Personal",
    "Promotional",
    "Security",
    "Shipping",
    "Travel",
    "Spam",
    "Other",
];

/// Extract the predicted label from the model output.
///
/// The model is trained to emit `<output>AI/Label</output>`. We accept:
///   - the canonical XML form,
///   - bare `AI/Label`,
///   - bare `Label` (defensive — useful when the model truncates).
///
/// We pick the first match found in the text so a stray label name
/// inside a rationale earlier in the output cannot override the real
/// answer (the actual prediction always appears at the end).
pub fn parse_label(raw: &str) -> Option<&'static str> {
    // 1. Prefer an explicit <output>…</output> block.
    if let (Some(start), Some(end)) = (raw.rfind("<output>"), raw.rfind("</output>")) {
        if start < end {
            let inside = &raw[start + "<output>".len()..end];
            if let Some(l) = match_label_token(inside) {
                return Some(l);
            }
        }
    }
    // 2. Fall back to a bare token search over the whole output.
    match_label_token(raw)
}

fn match_label_token(s: &str) -> Option<&'static str> {
    let lower = s.to_ascii_lowercase();
    let mut best: Option<(usize, &'static str)> = None;
    for label in LABELS {
        let needle = label.to_ascii_lowercase();
        // Search for "ai/<label>" first (canonical), then bare label.
        let candidates = [format!("ai/{}", needle), needle];
        for cand in &candidates {
            if let Some(pos) = lower.find(cand) {
                let bytes = lower.as_bytes();
                let end = pos + cand.len();
                let left_ok = pos == 0 || !is_word_byte(bytes[pos - 1]);
                let right_ok = end == bytes.len() || !is_word_byte(bytes[end]);
                if left_ok && right_ok && best.map(|(p, _)| pos < p).unwrap_or(true) {
                    best = Some((pos, label));
                }
            }
        }
    }
    best.map(|(_, l)| l)
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_output_block() {
        assert_eq!(parse_label("<output>AI/Work</output>"), Some("Work"));
        assert_eq!(parse_label("Some text\n<output>AI/Billing</output>\n"), Some("Billing"));
    }

    #[test]
    fn picks_last_output_block() {
        // If the model rambles, the operative <output> block is the last one.
        let raw = "<output>AI/Other</output> wait, on reflection: <output>AI/Work</output>";
        assert_eq!(parse_label(raw), Some("Work"));
    }

    #[test]
    fn parses_bare_ai_prefix() {
        assert_eq!(parse_label("AI/Personal"), Some("Personal"));
    }

    #[test]
    fn parses_bare_label() {
        assert_eq!(parse_label("Work"), Some("Work"));
        assert_eq!(parse_label("Work\n"), Some("Work"));
    }

    #[test]
    fn rejects_non_label() {
        assert_eq!(parse_label(""), None);
        assert_eq!(parse_label("not a category"), None);
    }

    #[test]
    fn word_boundary_required() {
        assert_eq!(parse_label("workflow"), None);
        assert_eq!(parse_label("billings"), None);
    }
}
