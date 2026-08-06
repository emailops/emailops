//! Which work Apple's on-device model gets a first attempt at.
//!
//! Pure decision over (operation, prompt size, availability, preference). No
//! I/O, so the policy is table-tested rather than discovered on a phone.
//!
//! ## Optimisation, not selection
//!
//! Apple's model is *tried first* for eligible work and the configured backend
//! serves everything it declines. A refusal is never a user-visible error:
//! Apple's guardrails will refuse things a mail client legitimately has to look
//! at — phishing and spam content especially, which is exactly where junk
//! detection earns its keep — and a classification that fails because the
//! safety filter fired is a bug, not a result.
//!
//! ## Why size, not task type
//!
//! The window is ~4096 tokens **shared between input and output**, and going
//! over is a hard `exceededContextWindowSize` failure rather than a worse
//! answer. A typical chat turn's retrieved sources are only ~1,200 tokens and
//! would fit; eight full body slices are ~8,000 and would not. So eligibility
//! is measured per prompt, not assumed per feature.
//!
//! ## What is excluded, and why it is not about size
//!
//! Chat is out for capability reasons: the turn runs an agentic loop through
//! `chat_stream_with_tools`, and this backend declares `tools: false`, so
//! routing chat here would amputate the loop rather than shrink the window. It
//! also streams tokens live, which the blocking `respond(to:)` path cannot do.
//! Both are implementable later (`streamResponse`, Apple's `Tool` protocol) and
//! neither is a size problem.

/// Operations Apple's model may attempt, keyed by the `operation` label that
/// `AiService::complete` already carries.
///
/// Drafts are deliberately here. They need no tools and no streaming — the
/// draft arrives as an event, not a live stream — and a reply to a short email
/// sits well inside the budget. They are also the slowest thing on a phone, so
/// they have the most to gain.
const ELIGIBLE_OPERATIONS: &[&str] = &[
    "generate_draft",
    "translate.detect",
    "translate.email",
    "tasks_extraction",
    "memory_extraction",
];

/// Whether this kind of work may go to Apple's model at all, before size is
/// considered.
pub fn afm_eligible_operation(operation: &str) -> bool {
    ELIGIBLE_OPERATIONS.contains(&operation)
}

/// What to do with one completion request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfmRoute {
    /// Ask Apple first; fall through to the configured backend if it declines.
    TryAppleFirst,
    /// Go straight to the configured backend.
    ConfiguredOnly,
}

/// Decide where a completion goes.
///
/// `enabled` is the user's preference. It exists because routing is not purely
/// a win: someone who has deliberately configured a frontier model through
/// OpenRouter should not have their extraction quietly downgraded to a 3B
/// on-device model. The default belongs to the caller, not here.
pub fn plan_afm_route(operation: &str, apple_available: bool, enabled: bool, prompt_fits: bool) -> AfmRoute {
    if enabled && apple_available && prompt_fits && afm_eligible_operation(operation) {
        AfmRoute::TryAppleFirst
    } else {
        AfmRoute::ConfiguredOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything on, prompt small, operation eligible.
    fn route(operation: &str, available: bool, enabled: bool, fits: bool) -> AfmRoute {
        plan_afm_route(operation, available, enabled, fits)
    }

    #[test]
    fn eligible_short_work_tries_apple_first() {
        for op in [
            "generate_draft",
            "translate.email",
            "tasks_extraction",
            "memory_extraction",
        ] {
            assert_eq!(route(op, true, true, true), AfmRoute::TryAppleFirst, "{op}");
        }
    }

    #[test]
    fn drafts_are_eligible() {
        // Called out separately because excluding them was my first instinct and
        // it was wrong: no tools, no streaming, and usually a small prompt.
        assert!(afm_eligible_operation("generate_draft"));
    }

    #[test]
    fn chat_is_never_eligible_however_small_the_prompt() {
        // Not a size decision: the chat turn needs tool calling, which this
        // backend does not have. A one-line question is still excluded.
        assert_eq!(route("chat_turn", true, true, true), AfmRoute::ConfiguredOnly);
        assert!(!afm_eligible_operation("chat_turn"));
    }

    #[test]
    fn an_oversized_prompt_goes_to_the_configured_backend() {
        // Over the window is a hard failure, not a worse answer, so it must
        // never be attempted.
        assert_eq!(route("generate_draft", true, true, false), AfmRoute::ConfiguredOnly);
    }

    #[test]
    fn an_unavailable_model_is_not_attempted() {
        // Ineligible device, Apple Intelligence switched off, or assets still
        // downloading.
        assert_eq!(route("generate_draft", false, true, true), AfmRoute::ConfiguredOnly);
    }

    #[test]
    fn the_preference_can_turn_the_whole_thing_off() {
        // Someone paying for a frontier model through OpenRouter should not
        // have their extraction quietly downgraded to a 3B on-device model.
        assert_eq!(route("generate_draft", true, false, true), AfmRoute::ConfiguredOnly);
    }

    #[test]
    fn an_unknown_operation_is_not_routed_by_accident() {
        // New operations opt in explicitly; the default is the backend the user
        // actually configured.
        assert!(!afm_eligible_operation("some_new_feature"));
        assert_eq!(route("some_new_feature", true, true, true), AfmRoute::ConfiguredOnly);
    }
}
