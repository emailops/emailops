//! How much retrieved mail a chat turn may put in front of the model.
//!
//! Pure decision over the model's *effective* context window — the `n_ctx` the
//! runtime actually opens, not the maximum the model card advertises.
//!
//! ## Why the window and not the platform
//!
//! The obvious version of this is "phone gets less". That is wrong in both
//! directions: a phone running a remote frontier model has no reason to be
//! starved, and a laptop running a 2.6B model at an 8k window is just as
//! squeezed as the phone is. The constraint belongs to the model, so the budget
//! is keyed to the model.
//!
//! ## Why it matters
//!
//! The prompt slices up to `MAX_SOURCE_BODY_CHARS` (4,000) characters per
//! source across up to `TOP_K_SOURCES` (8) sources: 32,000 characters, roughly
//! **8,000 tokens**, before the system prompt, tool definitions and history.
//! A phone opens an 8,192-token window. So the worst case is not "a large share
//! of the budget" — it is the entire budget, and the tail gets truncated.
//!
//! It is also where the latency lives: every one of those tokens is prefilled
//! on a phone CPU before the model writes a word.
//!
//! Cutting context trades recall for speed, which is a real loss: fewer sources
//! means "summarise everything from today" sees less mail. The tiers below aim
//! to keep the *number* of sources as high as the window allows and shorten the
//! excerpts first, on the reasoning that knowing a message exists matters more
//! than seeing four sentences of it — the model can ask for a body it needs.

/// What one chat turn may spend on retrieved mail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetrievalBudget {
    /// How many retrieved emails to put in the prompt.
    pub max_sources: usize,
    /// Characters of each source's body put in the prompt.
    pub source_body_chars: usize,
}

/// The generous budget, used when the window has room to spare. Matches the
/// values that were hard-coded before this planner existed, so a desktop-sized
/// window behaves exactly as it did.
pub const FULL_BUDGET: RetrievalBudget = RetrievalBudget {
    max_sources: 8,
    source_body_chars: 4_000,
};

/// Windows at or below this get the smallest budget. 8192 is what
/// `util::system::auto_n_ctx_tier` picks for a phone (and for any machine whose
/// memory probe failed), so this is the tier that actually runs on a device.
const SMALL_WINDOW: u32 = 8_192;

/// Windows at or below this get the middle budget.
const MEDIUM_WINDOW: u32 = 16_384;

/// Decide the retrieval budget for a model whose runtime window is `n_ctx`.
///
/// `n_ctx == 0` means "unknown" — a remote backend that never reported one.
/// Those get the full budget: an unknown window is far more likely to be a
/// hosted model with a large one than a squeezed local model, and starving a
/// frontier model to be safe would be the worse mistake.
pub fn plan_retrieval_budget(n_ctx: u32) -> RetrievalBudget {
    match n_ctx {
        0 => FULL_BUDGET,
        // ~4,200 characters, near enough 1,050 tokens: about an eighth of an
        // 8k window, leaving room for the system prompt, the tool definitions
        // and an actual answer — and short enough that prefill on a phone CPU
        // is not the whole of the wait.
        n if n <= SMALL_WINDOW => RetrievalBudget {
            // Sources are kept at 6 of 8 rather than halved: dropping a source
            // removes a message from consideration entirely, while a shorter
            // body only blurs one the model can still ask to read in full.
            max_sources: 6,
            source_body_chars: 700,
        },
        n if n <= MEDIUM_WINDOW => RetrievalBudget {
            max_sources: 8,
            source_body_chars: 1_500,
        },
        _ => FULL_BUDGET,
    }
}

/// Rough characters of excerpt this budget puts in the prompt. Used for logging
/// and for the tests that pin the relative sizes.
pub fn budget_chars(budget: RetrievalBudget) -> usize {
    budget.max_sources * budget.source_body_chars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_sized_window_gets_the_smallest_budget() {
        // 8192 is what `auto_n_ctx_tier` opens on a device.
        let budget = plan_retrieval_budget(8_192);
        assert_eq!(budget.max_sources, 6);
        assert_eq!(budget.source_body_chars, 700);
    }

    #[test]
    fn the_small_budget_fits_a_phone_window_with_room_to_answer() {
        // ~4 chars per token: the sources must not eat the window they are
        // supposed to fit inside.
        let tokens = budget_chars(plan_retrieval_budget(8_192)) / 4;
        assert!(tokens < 8_192 / 4, "sources take {tokens} tokens of an 8192 window");
    }

    #[test]
    fn the_small_budget_is_a_fraction_of_the_full_one() {
        // The point of the exercise: prefill on a phone CPU is the latency, and
        // this is the lever with the most of it.
        let small = budget_chars(plan_retrieval_budget(8_192));
        let full = budget_chars(FULL_BUDGET);
        assert!(
            small * 5 <= full,
            "small budget {small} is not meaningfully smaller than {full}"
        );
    }

    #[test]
    fn shrinking_costs_excerpt_length_before_it_costs_sources() {
        // Losing a source removes a message from consideration entirely; a
        // shorter excerpt only blurs one the model can still ask to read.
        let small = plan_retrieval_budget(8_192);
        let full = FULL_BUDGET;
        let source_loss = (full.max_sources - small.max_sources) as f64 / full.max_sources as f64;
        let excerpt_loss = (full.source_body_chars - small.source_body_chars) as f64 / full.source_body_chars as f64;
        assert!(
            excerpt_loss > source_loss,
            "excerpts should shrink harder than the source count ({excerpt_loss} vs {source_loss})"
        );
    }

    #[test]
    fn a_desktop_window_is_unchanged_from_before_the_planner() {
        // Regression guard: introducing this planner must not silently degrade
        // the machines that were fine.
        assert_eq!(plan_retrieval_budget(32_768), FULL_BUDGET);
        assert_eq!(plan_retrieval_budget(262_144), FULL_BUDGET);
    }

    #[test]
    fn a_middle_window_lands_between_the_two() {
        let mid = plan_retrieval_budget(16_384);
        assert_eq!(mid.max_sources, 8);
        assert!(mid.source_body_chars > plan_retrieval_budget(8_192).source_body_chars);
        assert!(mid.source_body_chars < FULL_BUDGET.source_body_chars);
    }

    #[test]
    fn an_unknown_window_is_not_treated_as_a_small_one() {
        // A remote backend that reports nothing is far likelier to be a hosted
        // model with a large window than a squeezed local one.
        assert_eq!(plan_retrieval_budget(0), FULL_BUDGET);
    }

    #[test]
    fn the_boundaries_fall_on_the_generous_side() {
        // One token over a tier boundary should not cost a source.
        assert_eq!(plan_retrieval_budget(SMALL_WINDOW).max_sources, 6);
        assert_eq!(plan_retrieval_budget(SMALL_WINDOW + 1).max_sources, 8);
        assert_eq!(plan_retrieval_budget(MEDIUM_WINDOW + 1), FULL_BUDGET);
    }
}
