//! "This answer is wrong" → the instruction that steers the retry.
//!
//! A correction is an ordinary turn with one extra line. It is NOT a new mode:
//! the route, the tools and the retrieval are whatever the question would have
//! got anyway, because the thing that was wrong is usually the answer, not the
//! path to it.
//!
//! Placement, like every other per-turn block: the **final user message**,
//! never the system prompt. The correction differs on every retry, so a system
//! placement would change the cached anchor and cold-prefill the turn.
//!
//! Pure: no I/O, no DB.

use crate::models::ChatCorrection;

/// How much of the rejected answer to quote back. The model already has it in
/// history verbatim; this is only an anchor so "the previous answer" is
/// unambiguous when several turns are in play.
const QUOTE_CHARS: usize = 240;

/// Trim to `QUOTE_CHARS` on a char boundary, with an ellipsis when cut.
fn quote(answer: &str) -> String {
    let collapsed = answer.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= QUOTE_CHARS {
        return collapsed;
    }
    let head: String = collapsed.chars().take(QUOTE_CHARS).collect();
    format!("{head}…")
}

/// Render the correction block, or `None` when there is nothing to say.
///
/// `rejected_answer` is the text of the message the user marked wrong; pass
/// `None` when it could not be read back (a deleted row), in which case the
/// block still carries the user's objection, which is the part that matters.
pub fn render_correction_block(correction: &ChatCorrection, rejected_answer: Option<&str>) -> Option<String> {
    let reason = correction.reason.trim();
    if reason.is_empty() {
        return None;
    }
    let mut block = String::from(
        "CORRECTION: your previous answer was wrong. Do not repeat it. \
         Work out what went wrong and answer again, using the tools if you need to re-check.\n",
    );
    if let Some(answer) = rejected_answer {
        let quoted = quote(answer);
        if !quoted.is_empty() {
            block.push_str(&format!("Previous answer: {quoted}\n"));
        }
    }
    block.push_str(&format!("What the user says is wrong with it: {reason}"));
    Some(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correction(reason: &str) -> ChatCorrection {
        ChatCorrection {
            rejected_message_id: "m1".into(),
            reason: reason.into(),
        }
    }

    #[test]
    fn the_block_carries_the_users_objection_verbatim() {
        let block = render_correction_block(&correction("esos correos son de septiembre, no de agosto"), None)
            .expect("a reason renders a block");
        assert!(block.contains("esos correos son de septiembre, no de agosto"));
    }

    #[test]
    fn the_block_tells_the_model_not_to_repeat_itself() {
        let block = render_correction_block(&correction("wrong dates"), None).expect("renders");
        assert!(block.contains("Do not repeat it"));
    }

    #[test]
    fn the_block_quotes_the_rejected_answer_as_an_anchor() {
        let block =
            render_correction_block(&correction("wrong"), Some("You have 4 invoices from August.")).expect("renders");
        assert!(block.contains("You have 4 invoices from August."));
    }

    #[test]
    fn a_long_rejected_answer_is_trimmed() {
        let long = "palabra ".repeat(500);
        let block = render_correction_block(&correction("wrong"), Some(&long)).expect("renders");
        assert!(block.chars().count() < 600, "block is {} chars", block.chars().count());
        assert!(block.contains('…'));
    }

    #[test]
    fn a_multiline_rejected_answer_is_collapsed_to_one_line() {
        let block = render_correction_block(&correction("wrong"), Some("line one\n\nline two")).expect("renders");
        let previous = block
            .lines()
            .find(|l| l.starts_with("Previous answer:"))
            .expect("has a Previous answer line");
        assert!(previous.contains("line one line two"));
    }

    #[test]
    fn an_empty_reason_renders_nothing() {
        assert_eq!(render_correction_block(&correction("   "), Some("x")), None);
    }

    #[test]
    fn a_missing_rejected_answer_still_renders_the_objection() {
        let block = render_correction_block(&correction("the dates are wrong"), None).expect("renders");
        assert!(!block.contains("Previous answer:"));
        assert!(block.contains("the dates are wrong"));
    }

    #[test]
    fn an_empty_rejected_answer_does_not_render_an_empty_quote_line() {
        let block = render_correction_block(&correction("wrong"), Some("   ")).expect("renders");
        assert!(!block.contains("Previous answer:"));
    }

    #[test]
    fn a_multibyte_answer_is_trimmed_on_a_char_boundary() {
        // Trimming by bytes here would panic; the emoji make each char 4 bytes.
        let emoji = "🧾".repeat(400);
        let block = render_correction_block(&correction("wrong"), Some(&emoji)).expect("renders");
        assert!(block.contains('…'));
    }
}
