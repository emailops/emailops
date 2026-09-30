//! Context budget for a chat prompt: what is cut, and in which order, when
//! the prompt would not fit the model's window. Pure — the executor in `turn`
//! measures the pieces and applies the plan.
//!
//! The order is a product decision (docs/DECISIONS.md, 2026-09-30): the system
//! prompt, the question and its per-turn blocks are never cut; the emails of
//! earlier questions go first, then earlier turns, then this turn's retrieved
//! emails, then this turn's tool results.

use crate::ai::provider::AiMessage;
use crate::models::{BudgetCut, BudgetTrace};

/// Chars per token before any call of the conversation has been measured:
/// the low end of what the Qwen tokenizer measures on these prompts (a chat
/// prompt of 20.6k chars counted 5224 tokens, 3.95 each; mail runs 3.5-4).
/// An estimate that errs high cuts a little early; one that errs low leaves
/// the runtime to truncate, which the safety margin is there to avoid.
const DEFAULT_CHARS_PER_TOKEN: f32 = 3.5;
/// A measured ratio outside this range is a measurement artefact (a prompt
/// the runtime truncated, a provider counting its own tool schemas).
const MIN_CHARS_PER_TOKEN: f32 = 2.0;
const MAX_CHARS_PER_TOKEN: f32 = 5.0;
/// Outside this range a provider's count is not the size of the prompt.
const IMPLAUSIBLE_BELOW: f32 = 1.5;
const IMPLAUSIBLE_ABOVE: f32 = 8.0;

/// The reply keeps an eighth of the window, within these bounds.
const MIN_REPLY_RESERVE: u32 = 1024;
const MAX_REPLY_RESERVE: u32 = 4096;
/// Slack for chat-template tokens and estimate error.
const SAFETY_TOKENS: u32 = 256;
/// Role markers and separators the chat template adds around each message.
const MESSAGE_OVERHEAD_CHARS: usize = 24;

/// Below this an excerpt no longer says what the email is about.
pub(crate) const MIN_SOURCE_BODY_CHARS: usize = 600;
/// The open thread is what the question is about: it keeps more.
pub(crate) const MIN_OPEN_THREAD_CHARS: usize = 2_000;
/// A tool result is never cut below this.
pub(crate) const MIN_TOOL_RESULT_CHARS: usize = 1_500;
/// Chars the truncation note appended to a cut tool result may take.
const TOOL_NOTE_CHARS: usize = 200;

/// Windows under this get the compact system prefix: the full one takes ~7.4k
/// tokens, which leaves an 8k window nothing and a 12k one very little.
const COMPACT_PREFIX_BELOW_N_CTX: u32 = 16_384;

/// Whether a window of `n_ctx` tokens gets the compact system prefix. Decided
/// by the window alone, never per turn, so the prefix stays byte-stable for
/// the KV cache. An unknown window (`0`) keeps the full one.
pub(crate) fn compact_prefix(n_ctx: u32) -> bool {
    n_ctx > 0 && n_ctx < COMPACT_PREFIX_BELOW_N_CTX
}

/// Converts between prompt chars and tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Estimator {
    chars_per_token: f32,
}

impl Estimator {
    pub(crate) fn uncalibrated() -> Self {
        Self {
            chars_per_token: DEFAULT_CHARS_PER_TOKEN,
        }
    }

    /// From a call the provider measured: `prompt_chars` went in and it
    /// counted `prompt_tokens`.
    pub(crate) fn calibrated(prompt_chars: usize, prompt_tokens: u32) -> Self {
        if prompt_chars == 0 || prompt_tokens == 0 {
            return Self::uncalibrated();
        }
        Self {
            chars_per_token: (prompt_chars as f32 / prompt_tokens as f32)
                .clamp(MIN_CHARS_PER_TOKEN, MAX_CHARS_PER_TOKEN),
        }
    }

    pub(crate) fn tokens(&self, chars: usize) -> usize {
        (chars as f32 / self.chars_per_token).ceil() as usize
    }

    pub(crate) fn chars(&self, tokens: usize) -> usize {
        (tokens as f32 * self.chars_per_token).floor() as usize
    }
}

/// Tokens kept free for the reply.
pub(crate) fn plan_reply_reserve(n_ctx: u32) -> u32 {
    (n_ctx / 8).clamp(MIN_REPLY_RESERVE, MAX_REPLY_RESERVE)
}

/// Tokens the prompt may take in a window of `n_ctx`.
pub(crate) fn plan_prompt_tokens(n_ctx: u32) -> usize {
    n_ctx
        .saturating_sub(plan_reply_reserve(n_ctx))
        .saturating_sub(SAFETY_TOKENS) as usize
}

/// One earlier message of the conversation, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryItem {
    pub is_user: bool,
    /// Chars it replays with today.
    pub full_chars: usize,
    /// Chars it would take without the emails it was asked with; `None` when
    /// it carries none.
    pub stripped_chars: Option<usize>,
}

/// One retrieved email of this turn, best first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceItem {
    /// The From/Subject/Date line.
    pub header_chars: usize,
    /// The cleaned body, uncut.
    pub body_chars: usize,
}

/// The pieces of a turn's first prompt, in chars.
#[derive(Debug, Clone)]
pub(crate) struct PromptParts<'a> {
    pub system_chars: usize,
    pub history: &'a [HistoryItem],
    /// The question and every per-turn block but the sources and the open
    /// thread.
    pub tail_chars: usize,
    pub sources: &'a [SourceItem],
    /// The excerpt cap when nothing has to be cut.
    pub max_source_body_chars: usize,
    /// The open email thread, when the turn carries one.
    pub open_thread_chars: Option<usize>,
}

/// What to build the prompt with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PromptPlan {
    /// Index of the first history item that stays.
    pub history_start: usize,
    /// Kept history items replayed without their emails.
    pub stripped: Vec<usize>,
    pub source_count: usize,
    pub source_body_chars: usize,
    pub open_thread_chars: Option<usize>,
    pub estimated_chars: usize,
    pub fits: bool,
    pub cuts: Vec<BudgetCut>,
}

/// Fit a turn's first prompt into `budget_chars`.
pub(crate) fn plan_prompt(parts: &PromptParts<'_>, budget_chars: usize) -> PromptPlan {
    let history = parts.history;
    let sources_chars = |count: usize, cap: usize| -> usize {
        parts.sources[..count]
            .iter()
            .map(|s| s.header_chars + s.body_chars.min(cap))
            .sum()
    };

    let mut plan = PromptPlan {
        source_count: parts.sources.len(),
        source_body_chars: parts.max_source_body_chars,
        open_thread_chars: parts.open_thread_chars,
        ..PromptPlan::default()
    };
    // System message + final user message; every kept history item adds one.
    let fixed = parts.system_chars + parts.tail_chars + 2 * MESSAGE_OVERHEAD_CHARS;
    let history_chars: usize = history.iter().map(|h| h.full_chars + MESSAGE_OVERHEAD_CHARS).sum();
    let this_turn = sources_chars(plan.source_count, plan.source_body_chars) + plan.open_thread_chars.unwrap_or(0);
    let mut total = fixed + history_chars + this_turn;

    if total > budget_chars {
        // 1. Emails of earlier questions, oldest first. Past the budget once,
        // cut down to three quarters of it: the stripped form is what later
        // turns replay, so the next few turns extend this prompt instead of
        // cutting again.
        let target = budget_chars / 4 * 3;
        for (i, item) in history.iter().enumerate() {
            if total <= target {
                break;
            }
            if let Some(stripped) = item.stripped_chars.filter(|s| *s < item.full_chars) {
                let saved = item.full_chars - stripped;
                total -= saved;
                plan.stripped.push(i);
            }
        }
        if !plan.stripped.is_empty() {
            plan.cuts.push(BudgetCut::HistorySources {
                messages: plan.stripped.len() as u32,
            });
        }

        // 2. Whole exchanges, oldest first. An exchange is a user message and
        // what follows it up to the next one, so no answer is left without
        // its question.
        let chars_of = |i: usize, stripped: &[usize]| -> usize {
            let item = &history[i];
            let chars = match item.stripped_chars {
                Some(s) if stripped.contains(&i) => s,
                _ => item.full_chars,
            };
            chars + MESSAGE_OVERHEAD_CHARS
        };
        while total > budget_chars && plan.history_start < history.len() {
            loop {
                let gone = chars_of(plan.history_start, &plan.stripped);
                total -= gone;
                plan.history_start += 1;
                if plan.history_start >= history.len() || history[plan.history_start].is_user {
                    break;
                }
            }
        }
        if plan.history_start > 0 {
            let start = plan.history_start;
            plan.stripped.retain(|i| *i >= start);
            plan.cuts.push(BudgetCut::HistoryTurns { messages: start as u32 });
        }
    }

    // 3a. The open thread, down to its floor.
    if let Some(thread) = plan.open_thread_chars {
        if total > budget_chars && thread > MIN_OPEN_THREAD_CHARS {
            let kept = thread.saturating_sub(total - budget_chars).max(MIN_OPEN_THREAD_CHARS);
            total -= thread - kept;
            plan.open_thread_chars = Some(kept);
            plan.cuts.push(BudgetCut::OpenThread { chars: kept as u32 });
        }
    }

    // 3b. This turn's retrieved emails: shorter excerpts, shared evenly, then
    // fewer emails from the end of the list (thread expansion, then the
    // lowest ranked).
    if total > budget_chars && plan.source_count > 0 {
        let full = sources_chars(plan.source_count, plan.source_body_chars);
        let room = budget_chars.saturating_sub(total - full);
        while plan.source_count > 1 && sources_chars(plan.source_count, MIN_SOURCE_BODY_CHARS) > room {
            plan.source_count -= 1;
        }
        // The largest excerpt cap the emails that stay fit with.
        let (mut lo, mut hi) = (
            MIN_SOURCE_BODY_CHARS,
            parts.max_source_body_chars.max(MIN_SOURCE_BODY_CHARS),
        );
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            if sources_chars(plan.source_count, mid) <= room {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        plan.source_body_chars = lo;
        total = total - full + sources_chars(plan.source_count, lo);
        if parts.sources[..plan.source_count].iter().any(|s| s.body_chars > lo) {
            plan.cuts.push(BudgetCut::SourceExcerpts {
                chars_per_email: lo as u32,
            });
        }
        let dropped = parts.sources.len() - plan.source_count;
        if dropped > 0 {
            plan.cuts.push(BudgetCut::SourcesDropped { emails: dropped as u32 });
        }
    }

    plan.estimated_chars = total;
    plan.fits = total <= budget_chars;
    plan
}

/// The cap to cut this turn's tool results to so the prompt loses
/// `excess_chars`: every result longer than the cap is cut to it. `None` when
/// nothing has to go.
pub(crate) fn plan_tool_result_cap(result_chars: &[usize], excess_chars: usize) -> Option<usize> {
    if excess_chars == 0 {
        return None;
    }
    // What cutting to `cap` gives back, net of the note each cut result gains.
    let saved = |cap: usize| -> usize {
        result_chars
            .iter()
            .filter(|c| **c > cap + TOOL_NOTE_CHARS)
            .map(|c| c - cap - TOOL_NOTE_CHARS)
            .sum()
    };
    if saved(MIN_TOOL_RESULT_CHARS) == 0 {
        return None;
    }
    let (mut lo, mut hi) = (
        MIN_TOOL_RESULT_CHARS,
        result_chars.iter().copied().max().unwrap_or(MIN_TOOL_RESULT_CHARS),
    );
    // The largest cap that still gives back enough; the floor when none does.
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if saved(mid) >= excess_chars {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Some(lo)
}

/// A tool result cut to `cap` chars, ending with a note that tells the model
/// it is incomplete. Unchanged when it already fits.
pub(crate) fn cut_tool_result(text: &str, cap: usize) -> String {
    if text.len() <= cap + TOOL_NOTE_CHARS {
        return text.to_string();
    }
    let kept = crate::util::text::truncate_utf8(text, cap);
    format!(
        "{kept}\n[… cut here: {} of {} characters omitted to fit the context window. \
Ask for less at once if you need the rest.]",
        text.len() - kept.len(),
        text.len()
    )
}

/// Tokens a prompt of `chars_now` takes: what the provider counted on the
/// previous call of the turn plus an estimate of what changed since.
pub(crate) fn estimate_prompt_tokens(est: Estimator, chars_now: usize, last_call: Option<(usize, u32)>) -> usize {
    match last_call {
        None => est.tokens(chars_now),
        Some((chars_then, tokens_then)) if chars_now >= chars_then => {
            tokens_then as usize + est.tokens(chars_now - chars_then)
        }
        Some((chars_then, tokens_then)) => (tokens_then as usize).saturating_sub(est.tokens(chars_then - chars_now)),
    }
}

/// The budget of one chat turn: sized once, then consulted before every model
/// call of the turn, and the record of what it cut.
#[derive(Debug, Clone)]
pub(crate) struct TurnBudget {
    n_ctx: u32,
    /// Tokens the prompt may take; `usize::MAX` when there is no limit.
    prompt_tokens: usize,
    est: Estimator,
    /// Chars that take window but ride outside the message list: the tool
    /// schemas an HTTP provider is sent as its `tools` parameter.
    extra_chars: usize,
    /// Prompt chars and the tokens the provider counted for them, on the
    /// turn's last measured call.
    last_call: Option<(usize, u32)>,
    cuts: Vec<BudgetCut>,
    max_estimated_tokens: usize,
    /// The last prompt was estimated over the budget and the provider has not
    /// said yet whether it fit.
    estimated_over: bool,
    /// A prompt of this turn did not fit.
    overflowed: bool,
}

impl TurnBudget {
    /// No limit: nothing is ever cut and nothing is traced.
    pub(crate) fn unlimited() -> Self {
        Self {
            n_ctx: 0,
            prompt_tokens: usize::MAX,
            est: Estimator::uncalibrated(),
            extra_chars: 0,
            last_call: None,
            cuts: Vec::new(),
            max_estimated_tokens: 0,
            estimated_over: false,
            overflowed: false,
        }
    }

    pub(crate) fn new(n_ctx: u32, est: Estimator, extra_chars: usize) -> Self {
        Self {
            n_ctx,
            prompt_tokens: plan_prompt_tokens(n_ctx),
            est,
            extra_chars,
            ..Self::unlimited()
        }
    }

    pub(crate) fn n_ctx(&self) -> u32 {
        self.n_ctx
    }

    /// See [`compact_prefix`].
    pub(crate) fn compact_prefix(&self) -> bool {
        compact_prefix(self.n_ctx)
    }

    /// Chars the message list may take.
    pub(crate) fn message_chars(&self) -> usize {
        self.est.chars(self.prompt_tokens).saturating_sub(self.extra_chars)
    }

    /// Record how the turn's first prompt was planned.
    pub(crate) fn record_plan(&mut self, plan: &PromptPlan) {
        self.cuts.extend(plan.cuts.iter().cloned());
        self.estimated_over = !plan.fits;
        self.max_estimated_tokens = self
            .max_estimated_tokens
            .max(self.est.tokens(plan.estimated_chars + self.extra_chars));
    }

    /// Make `messages` fit before they are sent: cut this turn's tool results
    /// when the prompt would not fit otherwise. Returns the prompt's chars,
    /// for [`record_call`](Self::record_call).
    pub(crate) fn fit(&mut self, messages: &mut [AiMessage]) -> usize {
        let mut chars = prompt_chars(messages) + self.extra_chars;
        if self.prompt_tokens == usize::MAX {
            return chars;
        }
        let mut tokens = estimate_prompt_tokens(self.est, chars, self.last_call);
        if tokens > self.prompt_tokens {
            let excess = self.est.chars(tokens - self.prompt_tokens) + 1;
            let results: Vec<usize> = messages
                .iter()
                .filter(|m| m.role == "tool")
                .map(|m| m.content.len())
                .collect();
            if let Some(cap) = plan_tool_result_cap(&results, excess) {
                let (mut cut, mut dropped) = (0u32, 0usize);
                for m in messages.iter_mut().filter(|m| m.role == "tool") {
                    let shorter = cut_tool_result(&m.content, cap);
                    if shorter.len() < m.content.len() {
                        cut += 1;
                        dropped += m.content.len() - shorter.len();
                        m.content = shorter;
                    }
                }
                self.record_tool_cut(cut, dropped as u32);
                chars = prompt_chars(messages) + self.extra_chars;
                tokens = estimate_prompt_tokens(self.est, chars, self.last_call);
            }
        }
        self.estimated_over = tokens > self.prompt_tokens;
        self.max_estimated_tokens = self.max_estimated_tokens.max(tokens);
        chars
    }

    fn record_tool_cut(&mut self, cut: u32, dropped: u32) {
        for entry in &mut self.cuts {
            if let BudgetCut::ToolResults { results, chars_dropped } = entry {
                *results += cut;
                *chars_dropped += dropped;
                return;
            }
        }
        self.cuts.push(BudgetCut::ToolResults {
            results: cut,
            chars_dropped: dropped,
        });
    }

    /// What the provider measured for a prompt of `chars`.
    pub(crate) fn record_call(&mut self, chars: usize, prompt_tokens: Option<u32>, dropped_front: Option<u32>) {
        // Only the embedded runtime says whether it truncated; any other
        // provider leaves the estimate standing.
        match dropped_front {
            Some(0) => {}
            Some(_) => self.overflowed = true,
            None => self.overflowed |= self.estimated_over,
        }
        self.estimated_over = false;
        let Some(kept) = prompt_tokens.filter(|t| *t > 0) else {
            return;
        };
        let tokens = kept + dropped_front.unwrap_or(0);
        self.max_estimated_tokens = self.max_estimated_tokens.max(tokens as usize);
        // A count far off any real tokenizer is not the prompt's size (a
        // provider reporting only the tokens it did not have cached).
        let ratio = chars as f32 / tokens as f32;
        if !(IMPLAUSIBLE_BELOW..=IMPLAUSIBLE_ABOVE).contains(&ratio) {
            return;
        }
        self.last_call = Some((chars, tokens));
        self.est = Estimator::calibrated(chars, tokens);
    }

    /// The turn's budget record; `None` when nothing was cut and it all fit.
    pub(crate) fn trace(&self) -> Option<BudgetTrace> {
        let fits = !self.overflowed && !self.estimated_over;
        if fits && self.cuts.is_empty() {
            return None;
        }
        Some(BudgetTrace {
            n_ctx: self.n_ctx,
            reply_reserve: plan_reply_reserve(self.n_ctx),
            estimated_prompt_tokens: self.max_estimated_tokens as u32,
            cuts: self.cuts.clone(),
            fits,
        })
    }
}

/// One cut, for the log and the trace.
pub(crate) fn describe_cut(cut: &BudgetCut) -> String {
    match cut {
        BudgetCut::HistorySources { messages } => format!("emails of {messages} earlier question(s) left out"),
        BudgetCut::HistoryTurns { messages } => format!("{messages} earlier message(s) left out"),
        BudgetCut::OpenThread { chars } => format!("open thread cut to {chars} chars"),
        BudgetCut::SourceExcerpts { chars_per_email } => format!("excerpts cut to {chars_per_email} chars per email"),
        BudgetCut::SourcesDropped { emails } => format!("{emails} retrieved email(s) left out"),
        BudgetCut::ToolResults { results, chars_dropped } => {
            format!("{results} tool result(s) cut by {chars_dropped} chars")
        }
    }
}

/// Every cut of a turn on one line.
pub(crate) fn describe_cuts(cuts: &[BudgetCut]) -> String {
    cuts.iter().map(describe_cut).collect::<Vec<_>>().join("; ")
}

/// Chars a message list takes in the prompt.
pub(crate) fn prompt_chars(messages: &[AiMessage]) -> usize {
    messages
        .iter()
        .map(|m| {
            let calls = m
                .tool_calls
                .as_ref()
                .map(|calls| {
                    calls
                        .iter()
                        .map(|c| c.function.name.len() + c.function.arguments.to_string().len())
                        .sum::<usize>()
                })
                .unwrap_or(0);
            m.content.len() + calls + MESSAGE_OVERHEAD_CHARS
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(full: usize, stripped: Option<usize>) -> HistoryItem {
        HistoryItem {
            is_user: true,
            full_chars: full,
            stripped_chars: stripped,
        }
    }

    fn assistant(chars: usize) -> HistoryItem {
        HistoryItem {
            is_user: false,
            full_chars: chars,
            stripped_chars: None,
        }
    }

    fn source(body: usize) -> SourceItem {
        SourceItem {
            header_chars: 100,
            body_chars: body,
        }
    }

    fn parts<'a>(history: &'a [HistoryItem], sources: &'a [SourceItem]) -> PromptParts<'a> {
        PromptParts {
            system_chars: 10_000,
            history,
            tail_chars: 500,
            sources,
            max_source_body_chars: 4_000,
            open_thread_chars: None,
        }
    }

    #[test]
    fn the_compact_prefix_is_for_windows_under_16k() {
        for (n_ctx, want) in [
            (0, false),
            (4096, true),
            (8192, true),
            (16_383, true),
            (16_384, false),
            (32_768, false),
        ] {
            assert_eq!(compact_prefix(n_ctx), want, "n_ctx={n_ctx}");
        }
        assert!(!TurnBudget::unlimited().compact_prefix());
        assert!(TurnBudget::new(8192, Estimator::uncalibrated(), 0).compact_prefix());
    }

    #[test]
    fn reply_reserve_is_an_eighth_of_the_window_within_bounds() {
        for (n_ctx, want) in [
            (4096, 1024),
            (8192, 1024),
            (16384, 2048),
            (32768, 4096),
            (200_000, 4096),
        ] {
            assert_eq!(plan_reply_reserve(n_ctx), want, "n_ctx={n_ctx}");
        }
    }

    #[test]
    fn prompt_tokens_leave_the_reserve_and_the_safety_margin() {
        assert_eq!(plan_prompt_tokens(8192), 8192 - 1024 - 256);
        assert_eq!(plan_prompt_tokens(32768), 32768 - 4096 - 256);
        // A window smaller than the reserve leaves nothing, not an underflow.
        assert_eq!(plan_prompt_tokens(1024), 0);
    }

    #[test]
    fn the_uncalibrated_estimate_is_three_and_a_half_chars_per_token() {
        let est = Estimator::uncalibrated();
        assert_eq!(est.tokens(3500), 1000);
        assert_eq!(est.tokens(3501), 1001, "rounds up");
        assert_eq!(est.chars(1000), 3500);
    }

    #[test]
    fn calibration_uses_the_measured_ratio_within_bounds() {
        assert_eq!(Estimator::calibrated(4000, 1000).tokens(4000), 1000);
        // Measured 1 char/token (a truncated prompt): clamped to 2.
        assert_eq!(Estimator::calibrated(1000, 1000).tokens(4000), 2000);
        // Measured 10 chars/token: clamped to 5.
        assert_eq!(Estimator::calibrated(10_000, 1000).tokens(5000), 1000);
        // Nothing measured: the default.
        assert_eq!(Estimator::calibrated(4000, 0), Estimator::uncalibrated());
        assert_eq!(Estimator::calibrated(0, 100), Estimator::uncalibrated());
    }

    #[test]
    fn a_prompt_that_fits_is_left_alone() {
        let history = [user(9_000, Some(200)), assistant(300)];
        let sources = [source(4_000), source(9_000)];
        let plan = plan_prompt(&parts(&history, &sources), 100_000);
        assert_eq!(plan.history_start, 0);
        assert!(plan.stripped.is_empty());
        assert_eq!(plan.source_count, 2);
        assert_eq!(plan.source_body_chars, 4_000);
        assert!(plan.fits);
        assert!(plan.cuts.is_empty());
        // system + 2 history + tail message, sources capped at 4000 each.
        assert_eq!(
            plan.estimated_chars,
            10_000 + 9_000 + 300 + 500 + (100 + 4_000) * 2 + 4 * MESSAGE_OVERHEAD_CHARS
        );
    }

    #[test]
    fn old_sources_go_first_oldest_first_down_to_three_quarters() {
        // 10k system + three exchanges of 9k + 0.5k tail = ~38.5k.
        let history = [
            user(9_000, Some(200)),
            assistant(300),
            user(9_000, Some(200)),
            assistant(300),
            user(9_000, Some(200)),
            assistant(300),
        ];
        // Budget 36k → over. Target 27k: stripping the first gets to ~29.7k,
        // the second to ~20.9k, so the third keeps its emails.
        let plan = plan_prompt(&parts(&history, &[]), 36_000);
        assert_eq!(plan.stripped, vec![0, 2]);
        assert_eq!(plan.history_start, 0);
        assert!(plan.fits);
        assert_eq!(plan.cuts, vec![BudgetCut::HistorySources { messages: 2 }]);
    }

    #[test]
    fn whole_exchanges_go_next_oldest_first() {
        // Nothing to strip: plain exchanges of 6k each.
        let history = [
            user(3_000, None),
            assistant(3_000),
            user(3_000, None),
            assistant(3_000),
            user(3_000, None),
            assistant(3_000),
        ];
        // 10k + 18k + 0.5k ≈ 28.7k against 20k: two exchanges must go.
        let plan = plan_prompt(&parts(&history, &[]), 20_000);
        assert_eq!(plan.history_start, 4);
        assert!(plan.fits);
        assert_eq!(plan.cuts, vec![BudgetCut::HistoryTurns { messages: 4 }]);
    }

    #[test]
    fn dropping_history_never_leaves_an_orphan_assistant_message() {
        // A history that starts mid-exchange (the message cap cut it there).
        let history = [assistant(5_000), user(5_000, None), assistant(5_000)];
        let plan = plan_prompt(&parts(&history, &[]), 22_000);
        // Dropping the leading assistant message alone is enough.
        assert_eq!(plan.history_start, 1);
        let plan = plan_prompt(&parts(&history, &[]), 12_000);
        assert_eq!(plan.history_start, 3);
    }

    #[test]
    fn dropped_turns_are_not_reported_as_stripped() {
        let history = [
            user(9_000, Some(8_000)),
            assistant(300),
            user(9_000, Some(8_000)),
            assistant(300),
        ];
        // Stripping barely helps; both exchanges have to go.
        let plan = plan_prompt(&parts(&history, &[]), 11_000);
        assert_eq!(plan.history_start, 4);
        assert!(plan.stripped.is_empty());
        assert_eq!(
            plan.cuts,
            vec![
                BudgetCut::HistorySources { messages: 2 },
                BudgetCut::HistoryTurns { messages: 4 }
            ]
        );
    }

    #[test]
    fn this_turns_excerpts_shrink_evenly_before_any_email_is_dropped() {
        let sources = [source(4_000), source(4_000), source(4_000), source(300)];
        // Fixed: 10k + 0.5k + 2 messages. Sources get what is left.
        let fixed = 10_000 + 500 + 2 * MESSAGE_OVERHEAD_CHARS;
        let budget = fixed + 4 * 100 + 300 + 3 * 1_000;
        let plan = plan_prompt(&parts(&[], &sources), budget);
        assert_eq!(plan.source_count, 4);
        assert_eq!(
            plan.source_body_chars, 1_000,
            "the short email is not charged more than it has"
        );
        assert!(plan.fits);
        assert_eq!(plan.cuts, vec![BudgetCut::SourceExcerpts { chars_per_email: 1_000 }]);
    }

    #[test]
    fn emails_are_dropped_from_the_end_once_excerpts_hit_the_floor() {
        let sources = [source(4_000), source(4_000), source(4_000), source(4_000)];
        let fixed = 10_000 + 500 + 2 * MESSAGE_OVERHEAD_CHARS;
        // Room for two emails at the floor, plus a little.
        let budget = fixed + 2 * (100 + MIN_SOURCE_BODY_CHARS) + 250;
        let plan = plan_prompt(&parts(&[], &sources), budget);
        assert_eq!(plan.source_count, 2);
        // The two that stay share the room that is left.
        assert_eq!(plan.source_body_chars, MIN_SOURCE_BODY_CHARS + 125);
        assert!(plan.fits);
        assert_eq!(
            plan.cuts,
            vec![
                BudgetCut::SourceExcerpts {
                    chars_per_email: (MIN_SOURCE_BODY_CHARS + 125) as u32
                },
                BudgetCut::SourcesDropped { emails: 2 },
            ]
        );
    }

    #[test]
    fn history_is_cut_before_this_turns_emails() {
        let history = [user(3_000, None), assistant(3_000)];
        let sources = [source(4_000), source(4_000)];
        let fixed = 10_000 + 500 + 2 * MESSAGE_OVERHEAD_CHARS;
        // Exactly the two emails in full once the history is gone.
        let plan = plan_prompt(&parts(&history, &sources), fixed + 2 * 4_100);
        assert_eq!(plan.history_start, 2);
        assert_eq!(plan.source_body_chars, 4_000);
        assert_eq!(plan.cuts, vec![BudgetCut::HistoryTurns { messages: 2 }]);
    }

    #[test]
    fn the_open_thread_shrinks_to_its_floor_and_no_further() {
        let mut p = parts(&[], &[]);
        p.open_thread_chars = Some(16_000);
        let fixed = 10_000 + 500 + 2 * MESSAGE_OVERHEAD_CHARS;
        let plan = plan_prompt(&p, fixed + 6_000);
        assert_eq!(plan.open_thread_chars, Some(6_000));
        assert!(plan.fits);
        assert_eq!(plan.cuts, vec![BudgetCut::OpenThread { chars: 6_000 }]);

        let plan = plan_prompt(&p, fixed + 500);
        assert_eq!(plan.open_thread_chars, Some(MIN_OPEN_THREAD_CHARS));
        assert!(!plan.fits);
    }

    #[test]
    fn a_system_prompt_larger_than_the_budget_is_reported_not_cut() {
        let history = [user(3_000, None), assistant(3_000)];
        let sources = [source(4_000), source(4_000)];
        let plan = plan_prompt(&parts(&history, &sources), 8_000);
        assert!(!plan.fits);
        assert_eq!(plan.history_start, 2);
        // One email always stays, at the floor: the turn is about it.
        assert_eq!(plan.source_count, 1);
        assert_eq!(plan.source_body_chars, MIN_SOURCE_BODY_CHARS);
        assert!(plan.estimated_chars > 8_000);
    }

    #[test]
    fn tool_results_are_cut_to_one_shared_cap() {
        // Nothing to lose → nothing to cut.
        assert_eq!(plan_tool_result_cap(&[16_000, 3_000], 0), None);
        // 10k to lose (plus the notes): only the large result pays.
        let cap = plan_tool_result_cap(&[16_000, 3_000], 10_000).expect("cap");
        assert_eq!(cap, 16_000 - 10_000 - TOOL_NOTE_CHARS);
        // Both pay once the cap goes below the smaller one.
        let cap = plan_tool_result_cap(&[16_000, 16_000, 2_000], 20_000).expect("cap");
        assert_eq!(cap, 16_000 - 10_000 - TOOL_NOTE_CHARS);
        // Never below the floor, even when that is not enough.
        assert_eq!(plan_tool_result_cap(&[16_000], 50_000), Some(MIN_TOOL_RESULT_CHARS));
        // No result is long enough to give anything back.
        assert_eq!(plan_tool_result_cap(&[800, 1_200], 5_000), None);
    }

    #[test]
    fn a_cut_tool_result_says_it_is_incomplete() {
        let text = "x".repeat(5_000);
        let cut = cut_tool_result(&text, 2_000);
        assert!(cut.starts_with(&"x".repeat(2_000)));
        assert!(!cut.starts_with(&"x".repeat(2_001)));
        assert!(cut.contains("3000 of 5000 characters omitted"), "{}", &cut[2_000..]);
        assert!(cut.len() <= 2_000 + TOOL_NOTE_CHARS);
        // Already short enough: untouched.
        assert_eq!(cut_tool_result("short", 2_000), "short");
    }

    #[test]
    fn a_cut_tool_result_never_splits_a_character() {
        let text = "ñ".repeat(3_000);
        let cut = cut_tool_result(&text, 2_001);
        assert!(cut.starts_with(&"ñ".repeat(1_000)));
    }

    #[test]
    fn prompt_tokens_build_on_the_last_measured_call() {
        let est = Estimator::uncalibrated();
        assert_eq!(estimate_prompt_tokens(est, 35_000, None), 10_000);
        // 7000 tokens measured at 28k chars; 3.5k chars were added since.
        assert_eq!(estimate_prompt_tokens(est, 31_500, Some((28_000, 7_000))), 8_000);
        // The prompt shrank since (tool results were cut).
        assert_eq!(estimate_prompt_tokens(est, 24_500, Some((28_000, 7_000))), 6_000);
        assert_eq!(estimate_prompt_tokens(est, 1_000, Some((28_000, 7_000))), 0);
    }

    fn msg(role: &str, chars: usize) -> AiMessage {
        AiMessage {
            role: role.to_string(),
            content: "x".repeat(chars),
            tool_calls: None,
        }
    }

    #[test]
    fn an_unlimited_budget_cuts_and_traces_nothing() {
        let mut budget = TurnBudget::unlimited();
        let mut messages = vec![msg("system", 30_000), msg("tool", 900_000)];
        budget.fit(&mut messages);
        assert_eq!(messages[1].content.len(), 900_000);
        assert_eq!(budget.trace(), None);
    }

    #[test]
    fn a_turn_that_fits_leaves_no_trace() {
        let mut budget = TurnBudget::new(8192, Estimator::uncalibrated(), 0);
        let mut messages = vec![msg("system", 6_000), msg("user", 300), msg("tool", 4_000)];
        let chars = budget.fit(&mut messages);
        assert_eq!(chars, 10_300 + 3 * MESSAGE_OVERHEAD_CHARS);
        assert_eq!(messages[2].content.len(), 4_000);
        assert_eq!(budget.trace(), None);
    }

    #[test]
    fn message_chars_leave_room_for_what_rides_outside_the_messages() {
        // 8192 → 6912 prompt tokens → 24192 chars at 3.5 chars/token.
        assert_eq!(
            TurnBudget::new(8192, Estimator::uncalibrated(), 0).message_chars(),
            24_192
        );
        assert_eq!(
            TurnBudget::new(8192, Estimator::uncalibrated(), 5_000).message_chars(),
            19_192
        );
        assert_eq!(
            TurnBudget::new(8192, Estimator::uncalibrated(), 50_000).message_chars(),
            0
        );
    }

    #[test]
    fn tool_results_are_cut_before_a_call_that_would_not_fit() {
        let mut budget = TurnBudget::new(8192, Estimator::uncalibrated(), 0);
        let mut messages = vec![
            msg("system", 9_000),
            msg("user", 300),
            msg("tool", 16_000),
            msg("tool", 2_000),
        ];
        let chars = budget.fit(&mut messages);
        assert_eq!(messages[0].content.len(), 9_000, "the system prompt is never cut");
        assert!(messages[2].content.len() < 16_000);
        assert_eq!(messages[3].content.len(), 2_000, "a short result is left alone");
        assert!(Estimator::uncalibrated().tokens(chars) <= plan_prompt_tokens(8192));
        let trace = budget.trace().expect("trace");
        assert!(trace.fits);
        assert_eq!(trace.n_ctx, 8192);
        assert_eq!(trace.reply_reserve, 1024);
        assert!(matches!(
            trace.cuts.as_slice(),
            [BudgetCut::ToolResults { results: 1, chars_dropped }] if *chars_dropped > 3_000
        ));
    }

    #[test]
    fn later_cuts_of_a_turn_add_to_the_same_entry() {
        let mut budget = TurnBudget::new(8192, Estimator::uncalibrated(), 0);
        let mut messages = vec![msg("system", 9_000), msg("tool", 16_000)];
        budget.fit(&mut messages);
        messages.push(msg("tool", 16_000));
        budget.fit(&mut messages);
        let trace = budget.trace().expect("trace");
        assert!(matches!(
            trace.cuts.as_slice(),
            [BudgetCut::ToolResults { results: 3, .. }]
        ));
    }

    #[test]
    fn a_prompt_no_cut_can_fit_is_reported() {
        let mut budget = TurnBudget::new(8192, Estimator::uncalibrated(), 0);
        let mut messages = vec![msg("system", 30_000), msg("user", 300)];
        budget.fit(&mut messages);
        let trace = budget.trace().expect("trace");
        assert!(!trace.fits);
        assert!(trace.cuts.is_empty());
        assert!(trace.estimated_prompt_tokens > 8_000);
    }

    #[test]
    fn a_measured_call_replaces_the_estimate() {
        let mut budget = TurnBudget::new(8192, Estimator::uncalibrated(), 0);
        // 28k chars estimate to 8000 tokens — over. The runtime counted 6000
        // and truncated nothing, so the turn fit after all.
        let mut messages = vec![msg("system", 28_000 - MESSAGE_OVERHEAD_CHARS)];
        let chars = budget.fit(&mut messages);
        budget.record_call(chars, Some(6_000), Some(0));
        assert_eq!(budget.trace(), None);
        // 1400 more chars at the measured 4.67 chars/token: 6300 tokens, fits.
        messages.push(msg("tool", 1_400 - MESSAGE_OVERHEAD_CHARS));
        budget.fit(&mut messages);
        assert_eq!(budget.trace(), None);
        assert_eq!(budget.max_estimated_tokens, 8_000);
    }

    #[test]
    fn a_prompt_the_runtime_truncated_is_reported() {
        let mut budget = TurnBudget::new(8192, Estimator::uncalibrated(), 0);
        let mut messages = vec![msg("system", 9_000)];
        let chars = budget.fit(&mut messages);
        budget.record_call(chars, Some(7_168), Some(300));
        let trace = budget.trace().expect("trace");
        assert!(!trace.fits);
        assert_eq!(trace.estimated_prompt_tokens, 7_468);
    }

    #[test]
    fn an_implausible_measurement_is_ignored() {
        let mut budget = TurnBudget::new(8192, Estimator::uncalibrated(), 0);
        // A provider that reports only the uncached tokens: 40 for 9000 chars.
        budget.record_call(9_000, Some(40), None);
        assert_eq!(budget.last_call, None);
        assert_eq!(budget.est, Estimator::uncalibrated());
    }

    #[test]
    fn the_first_prompts_plan_is_part_of_the_trace() {
        let mut budget = TurnBudget::new(8192, Estimator::uncalibrated(), 0);
        let plan = PromptPlan {
            cuts: vec![BudgetCut::HistoryTurns { messages: 2 }],
            estimated_chars: 17_500,
            fits: true,
            ..PromptPlan::default()
        };
        budget.record_plan(&plan);
        let trace = budget.trace().expect("trace");
        assert_eq!(trace.cuts, plan.cuts);
        assert_eq!(trace.estimated_prompt_tokens, 5_000);
        assert!(trace.fits);
        assert!(!trace.affects_answer());
    }
}
