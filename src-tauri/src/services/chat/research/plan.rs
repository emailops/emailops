//! Pure planners for research mode: how much a batch holds, which searches
//! gather the candidates, how batches and condense groups are cut, and how
//! long a run will take. No I/O — the executor in `mod.rs` does that.

use std::ops::Range;

use crate::services::chat::planner::SearchPlan;

// ── Budget ──────────────────────────────────────────────────────────────────

/// Conservative chars-per-token for mixed EN/ES mail on the Qwen tokenizer
/// (measured ~3.5-4); erring low keeps a batch from front-truncating.
pub(crate) const CHARS_PER_TOKEN: usize = 3;
/// Instructions + question of the map prompt, in tokens.
const MAP_OVERHEAD_TOKENS: usize = 700;
/// What one batch's reply may take: one bounded entry per conversation (see
/// `reading`), so the reply always fits and nothing is cut mid-batch.
pub(crate) const MAP_MAX_TOKENS: u32 = 1200;
/// Instructions + question + coverage line of the reduce/condense prompts.
pub(crate) const REDUCE_OVERHEAD_TOKENS: usize = 700;
/// The final report's length: a quarter of the window, never below the floor
/// (a report that says anything useful about many conversations) nor above the
/// cap (a small model repeats itself past it, and every token costs time).
pub(crate) const MIN_REPORT_TOKENS: u32 = 1536;
pub(crate) const MAX_REPORT_TOKENS: u32 = 4096;
/// What one condense call may write: at most `MAX_CONDENSED_NOTES` merged
/// notes (see `notes`), each bounded.
pub(crate) const CONDENSE_MAX_TOKENS: u32 = 1100;
/// Slack for tokenizer error and chat-template tokens.
const SAFETY_TOKENS: usize = 256;
/// Cleaned body kept per email: enough for the substance of most mail, small
/// enough that a batch holds several.
const CHARS_PER_EMAIL: usize = 1500;
/// A small model extracts less reliably from a long batch ("lost in the
/// middle"), so batches stay this small even when the window allows more.
const MAX_EMAILS_PER_BATCH: usize = 10;

/// How a research turn is cut, derived from the context window. There is no
/// cap on how many emails a turn reads: notes that outgrow one reduce prompt
/// are condensed in rounds (see [`plan_condense_groups`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResearchBudget {
    pub n_ctx: u32,
    /// Cleaned body chars per email.
    pub chars_per_email: usize,
    /// Email chars one map batch may carry.
    pub batch_chars: usize,
    pub max_emails_per_batch: usize,
    /// Notes chars one reduce (or condense) prompt can carry.
    pub notes_chars: usize,
    /// Output tokens kept free for the report when the notes fill its prompt.
    pub report_tokens: u32,
}

/// Size a research turn to the window. Pure: `n_ctx` is the only input.
pub(crate) fn plan_research_budget(n_ctx: u32) -> ResearchBudget {
    let window = n_ctx as usize;
    let batch_tokens = window.saturating_sub(MAP_OVERHEAD_TOKENS + MAP_MAX_TOKENS as usize + SAFETY_TOKENS);
    let batch_chars = (batch_tokens * CHARS_PER_TOKEN).min(CHARS_PER_EMAIL * MAX_EMAILS_PER_BATCH);
    let max_emails_per_batch = (batch_chars / CHARS_PER_EMAIL).clamp(1, MAX_EMAILS_PER_BATCH);
    let report_tokens = (n_ctx / 4).clamp(MIN_REPORT_TOKENS, MAX_REPORT_TOKENS);
    // The reduce writes the longer output, so its reserve bounds both steps.
    let notes_tokens = window.saturating_sub(REDUCE_OVERHEAD_TOKENS + report_tokens as usize + SAFETY_TOKENS);
    ResearchBudget {
        n_ctx,
        chars_per_email: CHARS_PER_EMAIL,
        batch_chars,
        max_emails_per_batch,
        notes_chars: (notes_tokens * CHARS_PER_TOKEN).max(CHARS_PER_TOKEN * 256),
        report_tokens,
    }
}

/// The report's output limit for an actual reduce prompt of `prompt_chars`:
/// whatever the window leaves free, between the floor and the cap. A prompt
/// the notes filled still leaves [`ResearchBudget::report_tokens`]. Pure.
pub(crate) fn plan_report_tokens(budget: &ResearchBudget, prompt_chars: usize) -> u32 {
    let prompt_tokens = prompt_chars / CHARS_PER_TOKEN;
    let free = (budget.n_ctx as usize).saturating_sub(prompt_tokens + SAFETY_TOKENS);
    u32::try_from(free)
        .unwrap_or(u32::MAX)
        .clamp(MIN_REPORT_TOKENS, MAX_REPORT_TOKENS)
}

// ── Direction ───────────────────────────────────────────────────────────────

/// Which side of the user's mail a question is about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Direction {
    /// Mail the user sent ("which quotes have I sent?").
    Sent,
    /// Mail the user received ("which quotes did I get?").
    Received,
    #[default]
    Any,
}

/// Whether `address` is one the user sends from.
pub(crate) fn is_user_address(address: &str, user_addresses: &[String]) -> bool {
    let address = address.trim();
    !address.is_empty() && user_addresses.iter().any(|u| u.trim().eq_ignore_ascii_case(address))
}

/// The direction the planner already decided: a sender filter on one of the
/// user's addresses is a question about sent mail, a recipient filter on one
/// about received mail. Read from the plan, never guessed from the wording.
/// Pure.
pub(crate) fn plan_direction(plan: Option<&SearchPlan>, user_addresses: &[String]) -> Direction {
    let is_me = |addr: &Option<String>| addr.as_deref().is_some_and(|a| is_user_address(a, user_addresses));
    match plan {
        Some(p) if is_me(&p.from) => Direction::Sent,
        Some(p) if is_me(&p.to) => Direction::Received,
        _ => Direction::Any,
    }
}

// ── Gather ──────────────────────────────────────────────────────────────────

/// One search that feeds the candidate set.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GatherStep {
    /// Every email matching the planner's filter, tags included.
    Filter(SearchPlan),
    /// The same filter without its classifier tags. A tag ranks rather than
    /// excludes in a normal turn: each email carries one intent and older mail
    /// none, so the untagged matches are part of the answer too. Only planned
    /// when another selective filter remains — an intent alone, untagged, would
    /// be the whole mailbox.
    FilterUntagged(SearchPlan),
    /// Emails close in meaning to `query`, plus exact keyword hits for
    /// `keywords` when the planner extracted any.
    Semantic { query: String, keywords: Option<String> },
}

/// Decide how to gather candidates from the query planner's filter.
///
/// A plan with a structural filter (sender, date window, tag…) states exactly
/// which mail the question is about, so only that filter runs: semantic
/// neighbours from outside the window would dilute the notes. A keyword-only
/// plan, or no plan at all, is a topic question — meaning finds what wording
/// misses, and the keywords add exact hits.
pub(crate) fn plan_gather(plan: Option<&SearchPlan>, question: &str) -> Vec<GatherStep> {
    let semantic = |keywords: Option<String>| GatherStep::Semantic {
        query: question.to_string(),
        keywords,
    };
    let Some(plan) = plan else {
        return vec![semantic(None)];
    };
    let mut plan = plan.clone();
    // "The first email" plans one oldest row; research reads them all, and
    // the planner's page size means nothing here.
    plan.limit = None;
    plan.order = None;
    if !plan.has_structural_filter() {
        return vec![semantic(plan.query.clone())];
    }
    let mut steps = vec![GatherStep::Filter(plan.clone())];
    let tagged = plan.intent.is_some() || plan.topic.is_some();
    let untagged = plan.clone().without_classifier_tags();
    let untagged_is_selective = untagged.has_structural_filter() || untagged.query.is_some();
    if tagged && untagged_is_selective {
        steps.push(GatherStep::FilterUntagged(untagged));
    }
    steps
}

/// A filter on the user runs once per address the user sends from: the
/// planner writes the account's address, and mail sent from an alias is the
/// user's mail too. Filters on anyone else are left alone. Pure.
pub(crate) fn for_every_user_address(steps: Vec<GatherStep>, user_addresses: &[String]) -> Vec<GatherStep> {
    let on_user = |a: &Option<String>| a.as_deref().is_some_and(|a| is_user_address(a, user_addresses));
    let expand = |plan: SearchPlan, wrap: fn(SearchPlan) -> GatherStep| -> Vec<GatherStep> {
        if on_user(&plan.from) {
            let each = |a: &String| SearchPlan {
                from: Some(a.clone()),
                ..plan.clone()
            };
            user_addresses.iter().map(|a| wrap(each(a))).collect()
        } else if on_user(&plan.to) {
            let each = |a: &String| SearchPlan {
                to: Some(a.clone()),
                ..plan.clone()
            };
            user_addresses.iter().map(|a| wrap(each(a))).collect()
        } else {
            vec![wrap(plan)]
        }
    };
    steps
        .into_iter()
        .flat_map(|step| match step {
            GatherStep::Filter(p) => expand(p, GatherStep::Filter),
            GatherStep::FilterUntagged(p) => expand(p, GatherStep::FilterUntagged),
            other => vec![other],
        })
        .collect()
}

/// Hits kept from the semantic pool: everything within `band` of the best
/// similarity. A fixed top-k would cut a large topic off at k and pad a small
/// one with noise; a band relative to the best hit follows the topic's size.
pub(crate) fn semantic_cutoff(similarities: &[f32], band: f32) -> usize {
    let Some(best) = similarities.iter().copied().reduce(f32::max) else {
        return 0;
    };
    similarities.iter().take_while(|s| **s >= best - band).count()
}

/// Merge candidate lists in order, dropping repeats.
pub(crate) fn merge_candidates(lists: &[Vec<String>]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for list in lists {
        for id in list {
            if seen.insert(id.as_str()) {
                out.push(id.clone());
            }
        }
    }
    out
}

// ── Batches and condense groups ─────────────────────────────────────────────

/// Split items (by length) into consecutive groups of at most `max_chars` and
/// `max_items` each. An item longer than the budget still gets a group of its
/// own.
pub(crate) fn plan_batches(lens: &[usize], max_chars: usize, max_items: usize) -> Vec<Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (i, len) in lens.iter().enumerate() {
        let full = i - start >= max_items || (i > start && used + len > max_chars);
        if full {
            groups.push(start..i);
            start = i;
            used = 0;
        }
        used += len;
    }
    if start < lens.len() {
        groups.push(start..lens.len());
    }
    groups
}

/// Group consecutive batches' notes (by length) into condense calls that each
/// fit one prompt. `None` when the notes already fit the reduce as they are.
pub(crate) fn plan_condense_groups(note_lens: &[usize], notes_chars: usize) -> Option<Vec<Range<usize>>> {
    let total: usize = note_lens.iter().sum();
    if total <= notes_chars {
        return None;
    }
    Some(plan_batches(note_lens, notes_chars, usize::MAX))
}

// ── Estimate ────────────────────────────────────────────────────────────────

/// Seconds per email on a machine that has not run research yet: ~1.5 s to
/// read (measured, qwen3.5-4b-q8_0, 16k window) plus the report's share.
pub(crate) const DEFAULT_MS_PER_EMAIL: u64 = 1800;

/// How long, in seconds, a run reading `emails` will take. `ms_per_email` is
/// what the last run on this machine measured, when known.
pub(crate) fn plan_estimate(emails: usize, ms_per_email: Option<u64>) -> u64 {
    let per_email = ms_per_email.filter(|ms| *ms > 0).unwrap_or(DEFAULT_MS_PER_EMAIL);
    emails as u64 * per_email / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn plan(f: impl FnOnce(&mut SearchPlan)) -> SearchPlan {
        let mut p = SearchPlan::default();
        f(&mut p);
        p
    }

    // ── budget ──

    #[test]
    fn budget_batches_fit_the_map_window() {
        for n_ctx in [8192u32, 16384, 32768] {
            let b = plan_research_budget(n_ctx);
            let batch_tokens = b.batch_chars / CHARS_PER_TOKEN + MAP_OVERHEAD_TOKENS + MAP_MAX_TOKENS as usize;
            assert!(
                batch_tokens <= n_ctx as usize,
                "n_ctx={n_ctx}: batch needs {batch_tokens}"
            );
            assert!(b.max_emails_per_batch * b.chars_per_email <= b.batch_chars);
        }
    }

    #[test]
    fn budget_notes_fit_the_reduce_window() {
        for n_ctx in [8192u32, 16384, 32768] {
            let b = plan_research_budget(n_ctx);
            let tokens = b.notes_chars / CHARS_PER_TOKEN + REDUCE_OVERHEAD_TOKENS + b.report_tokens as usize;
            assert!(tokens <= n_ctx as usize, "n_ctx={n_ctx}: reduce needs {tokens}");
        }
    }

    #[test]
    fn the_report_reserve_is_a_quarter_of_the_window_within_bounds() {
        assert_eq!(plan_research_budget(4096).report_tokens, MIN_REPORT_TOKENS);
        assert_eq!(plan_research_budget(8192).report_tokens, 2048);
        assert_eq!(plan_research_budget(15360).report_tokens, 3840);
        assert_eq!(plan_research_budget(32768).report_tokens, MAX_REPORT_TOKENS);
    }

    #[test]
    fn the_report_gets_what_its_prompt_leaves_free_up_to_the_cap() {
        let b = plan_research_budget(15360);
        // The production run: a 5.5k-token prompt left ~9.5k free.
        assert_eq!(plan_report_tokens(&b, 5_566 * CHARS_PER_TOKEN), MAX_REPORT_TOKENS);
        // A reduce prompt filled with notes to the brim still gets the reserve.
        let full = REDUCE_OVERHEAD_TOKENS * CHARS_PER_TOKEN + b.notes_chars;
        assert!(plan_report_tokens(&b, full) >= b.report_tokens);
        // On a small window the report never goes below the floor.
        let small = plan_research_budget(4096);
        assert_eq!(plan_report_tokens(&small, 4096 * CHARS_PER_TOKEN), MIN_REPORT_TOKENS);
    }

    // ── direction ──

    fn addrs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_filter_on_the_user_searches_every_address_the_user_sends_from() {
        let me = addrs(&["me@mail.example", "me@work.example"]);
        let steps = plan_gather(Some(&plan(|p| p.from = Some("Me@Mail.example".into()))), "q");
        let froms: Vec<String> = for_every_user_address(steps, &me)
            .into_iter()
            .filter_map(|s| match s {
                GatherStep::Filter(p) => p.from,
                _ => None,
            })
            .collect();
        assert_eq!(froms, me);
    }

    #[test]
    fn a_filter_on_someone_else_is_left_alone() {
        let me = addrs(&["me@mail.example", "me@work.example"]);
        let steps = plan_gather(Some(&plan(|p| p.from = Some("ana@client.example".into()))), "q");
        assert_eq!(for_every_user_address(steps.clone(), &me), steps);
    }

    #[test]
    fn direction_follows_a_sender_or_recipient_filter_on_the_user() {
        let me = &addrs(&["me@example.com", "me@work.example"]);
        let from = |a: &str| plan(|p| p.from = Some(a.into()));
        let to = |a: &str| plan(|p| p.to = Some(a.into()));
        assert_eq!(plan_direction(Some(&from("Me@Example.com")), me), Direction::Sent);
        assert_eq!(
            plan_direction(Some(&from("me@work.example")), me),
            Direction::Sent,
            "an alias is the user too"
        );
        assert_eq!(plan_direction(Some(&to("me@example.com")), me), Direction::Received);
        assert_eq!(plan_direction(Some(&from("ana@client.example")), me), Direction::Any);
        assert_eq!(plan_direction(Some(&plan(|_| {})), me), Direction::Any);
        assert_eq!(plan_direction(None, me), Direction::Any);
        assert_eq!(
            plan_direction(Some(&from("me@example.com")), &[]),
            Direction::Any,
            "no known address"
        );
    }

    #[test]
    fn a_full_reading_reply_always_fits_its_output_budget() {
        // Worst case: every conversation of a full batch gets an entry with the
        // longest text and the most emails. At the conservative chars-per-token
        // estimate, plus JSON keys and punctuation per entry.
        let per_entry = super::super::reading::MAX_FINDING_CHARS / CHARS_PER_TOKEN + 30;
        let worst = MAX_EMAILS_PER_BATCH * per_entry + 16;
        assert!(worst <= MAP_MAX_TOKENS as usize, "{worst} > {MAP_MAX_TOKENS}");
    }

    #[test]
    fn a_full_condense_reply_always_fits_its_output_budget() {
        let per_note = 300 / CHARS_PER_TOKEN + 30;
        let worst = super::super::notes::MAX_CONDENSED_NOTES * per_note + 16;
        assert!(worst <= CONDENSE_MAX_TOKENS as usize, "{worst} > {CONDENSE_MAX_TOKENS}");
    }

    #[test]
    fn a_condensed_group_is_smaller_than_what_went_in() {
        // Otherwise condensing would never converge.
        let b = plan_research_budget(8192);
        assert!((CONDENSE_MAX_TOKENS as usize) * CHARS_PER_TOKEN < b.notes_chars);
    }

    // ── gather ──

    #[test]
    fn no_plan_gathers_by_meaning() {
        assert_eq!(
            plan_gather(None, "¿qué problemas reportan?"),
            vec![GatherStep::Semantic {
                query: "¿qué problemas reportan?".into(),
                keywords: None
            }]
        );
    }

    #[test]
    fn a_keyword_plan_gathers_by_meaning_with_its_keywords() {
        let p = plan(|p| p.query = Some("migración".into()));
        assert_eq!(
            plan_gather(Some(&p), "q"),
            vec![GatherStep::Semantic {
                query: "q".into(),
                keywords: Some("migración".into())
            }]
        );
    }

    #[test]
    fn a_structural_filter_runs_alone_without_its_page_size() {
        let p = plan(|p| {
            p.from = Some("alice@example.com".into());
            p.limit = Some(5);
            p.order = Some("oldest".into());
        });
        let steps = plan_gather(Some(&p), "q");
        let expected = plan(|p| p.from = Some("alice@example.com".into()));
        assert_eq!(steps, vec![GatherStep::Filter(expected)]);
    }

    #[test]
    fn a_tag_with_another_filter_also_gathers_the_untagged_matches() {
        let p = plan(|p| {
            p.intent = Some("inquiry".into());
            p.subject = Some("Petición de contacto".into());
        });
        let steps = plan_gather(Some(&p), "q");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0], GatherStep::Filter(p.clone()));
        assert_eq!(
            steps[1],
            GatherStep::FilterUntagged(plan(|p| p.subject = Some("Petición de contacto".into())))
        );
    }

    #[test]
    fn a_tag_alone_never_widens_to_the_whole_mailbox() {
        let p = plan(|p| p.intent = Some("inquiry".into()));
        assert_eq!(plan_gather(Some(&p), "q"), vec![GatherStep::Filter(p)]);
    }

    #[test]
    fn semantic_cutoff_keeps_the_band_below_the_best_hit() {
        assert_eq!(semantic_cutoff(&[0.80, 0.78, 0.71, 0.69, 0.50], 0.10), 3);
        assert_eq!(semantic_cutoff(&[0.80], 0.10), 1);
        assert_eq!(semantic_cutoff(&[], 0.10), 0);
    }

    #[test]
    fn merge_keeps_order_and_drops_repeats() {
        let merged = merge_candidates(&[ids(&["a", "b"]), ids(&["b", "c"]), ids(&["a", "d"])]);
        assert_eq!(merged, ids(&["a", "b", "c", "d"]));
    }

    // ── batches ──

    #[test]
    fn batches_respect_the_char_budget() {
        assert_eq!(plan_batches(&[400, 400, 400, 400], 1000, 10), vec![0..2, 2..4]);
    }

    #[test]
    fn batches_respect_the_count_cap() {
        assert_eq!(plan_batches(&[10; 5], 10_000, 2), vec![0..2, 2..4, 4..5]);
    }

    #[test]
    fn an_oversized_item_gets_its_own_batch() {
        assert_eq!(plan_batches(&[100, 5000, 100], 1000, 10), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn no_items_no_batches() {
        assert!(plan_batches(&[], 1000, 10).is_empty());
    }

    // ── condense ──

    #[test]
    fn notes_that_fit_need_no_condensing() {
        assert_eq!(plan_condense_groups(&[300, 300], 1000), None);
    }

    #[test]
    fn overflowing_notes_are_grouped_to_fit_one_prompt_each() {
        assert_eq!(
            plan_condense_groups(&[600, 600, 600, 600, 600], 1000),
            Some(vec![0..1, 1..2, 2..3, 3..4, 4..5])
        );
        assert_eq!(
            plan_condense_groups(&[300, 300, 300, 300, 300], 1000),
            Some(vec![0..3, 3..5])
        );
    }

    // ── estimate ──

    #[test]
    fn estimate_uses_the_measured_speed_when_there_is_one() {
        assert_eq!(plan_estimate(1000, Some(1500)), 1500);
        assert_eq!(plan_estimate(1000, None), 1800);
        assert_eq!(plan_estimate(0, None), 0);
    }
}
