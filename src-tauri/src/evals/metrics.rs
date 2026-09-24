// Deterministic (non-LLM) heuristic assertions.
//
// These run unconditionally on every case and are what decides pass/fail in
// CI. The judge metrics (see `judge.rs`) are separate and only influence the
// score numbers shown in the report.

use regex::Regex;

use crate::evals::case_loader::EvalCase;
use crate::evals::harness::{CaseOutcome, SourceSummary};
use crate::evals::EvalResult;
use crate::models::ChatTrace;

#[derive(Debug, Clone)]
pub struct HeuristicCheck {
    pub name: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct HeuristicReport {
    pub checks: Vec<HeuristicCheck>,
}

impl HeuristicReport {
    pub fn all_passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }

    pub fn total(&self) -> usize {
        self.checks.len()
    }

    pub fn passed_count(&self) -> usize {
        self.checks.iter().filter(|c| c.passed).count()
    }
}

pub fn evaluate(case: &EvalCase, outcome: &CaseOutcome) -> EvalResult<HeuristicReport> {
    let mut checks: Vec<HeuristicCheck> = Vec::new();

    // Unconditional check: every case must produce a non-empty assistant reply.
    // An empty answer means the turn silently failed somewhere (model produced
    // no tokens, streaming aborted, etc) and there is no point running the
    // judge against an empty string — surface it as a hard failure instead.
    checks.push(check_answer_nonempty(&outcome.assistant_content));

    if let Some(expected) = case.expected_route.as_ref() {
        checks.push(check_route(expected, outcome.assistant_trace.as_ref()));
    }

    if !case.expected_tools_called.is_empty() {
        checks.push(check_tools(
            &case.expected_tools_called,
            outcome.assistant_trace.as_ref(),
        ));
    }

    if !case.expected_tools_not_called.is_empty() {
        checks.push(check_tools_not_called(
            &case.expected_tools_not_called,
            outcome.assistant_trace.as_ref(),
        ));
    }

    if !case.expected_answer_contains.is_empty() {
        checks.push(check_answer_contains(
            &case.expected_answer_contains,
            &outcome.assistant_content,
        ));
    }

    if !case.expected_answer_contains_any.is_empty() {
        checks.push(check_answer_contains_any(
            &case.expected_answer_contains_any,
            &outcome.assistant_content,
        ));
    }

    if !case.expected_answer_not_contains.is_empty() {
        checks.push(check_answer_not_contains(
            &case.expected_answer_not_contains,
            &outcome.assistant_content,
        ));
    }

    if !case.expected_tool_args_contains.is_empty() {
        let tool_calls = outcome
            .assistant_trace
            .as_ref()
            .map(|t| t.tool_calls.as_slice())
            .unwrap_or(&[]);
        checks.push(check_tool_args_contains(&case.expected_tool_args_contains, tool_calls));
    }

    if !case.expected_tool_args_not_contains.is_empty() {
        let tool_calls = outcome
            .assistant_trace
            .as_ref()
            .map(|t| t.tool_calls.as_slice())
            .unwrap_or(&[]);
        checks.push(check_tool_args_not_contains(
            &case.expected_tool_args_not_contains,
            tool_calls,
        ));
    }

    if !case.expected_help_pages_any.is_empty() {
        checks.push(check_help_pages_any(
            &case.expected_help_pages_any,
            outcome.assistant_trace.as_ref(),
        ));
    }

    if case.expected_no_email_sources {
        checks.push(check_no_email_sources(&outcome.sources_used));
    }

    if let Some(min) = case.expected_min_research_emails {
        checks.push(check_research_coverage(min, outcome.assistant_trace.as_ref()));
    }

    if !case.expected_research_matches.is_empty() || !case.forbidden_research_matches.is_empty() {
        checks.push(check_research_matches(
            &case.expected_research_matches,
            &case.forbidden_research_matches,
            &outcome.sources_used,
        ));
    }

    if !case.expected_cited_subjects.is_empty() {
        checks.push(check_cited_subjects(
            &case.expected_cited_subjects,
            &outcome.assistant_content,
            &outcome.sources_used,
        ));
    }

    if let Some(pattern) = case.expected_title_pattern.as_deref() {
        checks.push(check_title_pattern(pattern, &outcome.conversation_title)?);
    }

    Ok(HeuristicReport { checks })
}

fn check_route(expected: &crate::models::RouteMode, trace: Option<&ChatTrace>) -> HeuristicCheck {
    let actual = trace.map(|t| t.route.mode).map(|m| format!("{:?}", m));
    let expected_str = format!("{:?}", expected);
    let passed = trace.map(|t| t.route.mode == *expected).unwrap_or(false);
    HeuristicCheck {
        name: "route".into(),
        passed,
        expected: expected_str.clone(),
        actual: actual.clone().unwrap_or_else(|| "<no trace>".into()),
        detail: if passed {
            "router picked the expected mode".into()
        } else {
            format!(
                "expected route {:?}, got {}",
                expected,
                actual.unwrap_or_else(|| "nothing".into())
            )
        },
    }
}

/// Assert that none of `forbidden` was invoked this turn.
///
/// The inverse of [`check_tools`], for cases whose whole point is that the
/// model answers in text: thread-bound chat puts `generate_email_draft` on the
/// menu, and a read-only request that fires it produces a saved draft instead
/// of the requested answer. A positive anchor can't catch that — the reply
/// still contains plausible words — so the tool list is the real assertion.
fn check_tools_not_called(forbidden: &[String], trace: Option<&ChatTrace>) -> HeuristicCheck {
    let actual: Vec<String> = trace
        .map(|t| t.tool_calls.iter().map(|tc| tc.name.clone()).collect())
        .unwrap_or_default();

    let violations: Vec<String> = forbidden
        .iter()
        .filter(|needle| actual.iter().any(|a| a == *needle))
        .cloned()
        .collect();

    let passed = violations.is_empty();

    HeuristicCheck {
        name: "tools_not_called".into(),
        passed,
        expected: format!("none of: {}", forbidden.join(", ")),
        actual: if actual.is_empty() {
            "<none>".into()
        } else {
            actual.join(", ")
        },
        detail: if passed {
            "no forbidden tool was invoked".into()
        } else {
            format!("forbidden tool calls: {}", violations.join(", "))
        },
    }
}

/// Assert that the help lookup served at least one section of one of `pages`
/// (guide file stems, e.g. `ai-features`). Chunk ids are `<lang>/<page>#…`,
/// and the lookup serves sections in the AI output language, so the page is
/// matched regardless of language.
fn check_help_pages_any(pages: &[String], trace: Option<&ChatTrace>) -> HeuristicCheck {
    let served: Vec<String> = trace
        .and_then(|t| t.help.as_ref())
        .map(|h| h.chunk_ids.clone())
        .unwrap_or_default();
    let page_of = |chunk_id: &str| -> Option<String> {
        let (_, rest) = chunk_id.split_once('/')?;
        Some(rest.split('#').next().unwrap_or(rest).to_string())
    };
    let passed = served.iter().any(|id| page_of(id).is_some_and(|p| pages.contains(&p)));

    HeuristicCheck {
        name: "help_pages_any".into(),
        passed,
        expected: format!("a guide section from any of: {}", pages.join(", ")),
        actual: if served.is_empty() {
            "<no guide section served>".into()
        } else {
            served.join(", ")
        },
        detail: if passed {
            "the help lookup served an expected guide page".into()
        } else {
            "the answer was not grounded in the expected guide page".into()
        },
    }
}

/// Assert that no mailbox email was fed to the model as a RAG source — a
/// question about the app must be answered from the guides, not from an
/// email that happens to discuss the same topic.
/// A research-mode turn must have read at least `min` emails — the whole point
/// of the mode is coverage, and a planner filter that pages nothing or a
/// retrieval that silently fails would still yield a fluent report.
fn check_research_coverage(min: u32, trace: Option<&ChatTrace>) -> HeuristicCheck {
    let read = trace
        .and_then(|t| t.research.as_ref())
        .map(|r| (r.emails_analyzed, r.batches));
    let passed = read.is_some_and(|(emails, _)| emails >= min);
    HeuristicCheck {
        name: "research_coverage".into(),
        passed,
        expected: format!(">= {min} emails read"),
        actual: match read {
            Some((emails, batches)) => format!("{emails} emails read in {batches} batches"),
            None => "no research trace (the turn did not run research mode)".into(),
        },
        detail: if passed {
            "research mode read the expected share of the mailbox".into()
        } else {
            "research mode read fewer emails than the case requires".into()
        },
    }
}

/// The conversations a research run matched are its sources: each `expected`
/// subject substring must be among them and no `forbidden` one may be.
fn check_research_matches(
    expected: &[String],
    forbidden: &[String],
    sources: &[crate::evals::harness::SourceSummary],
) -> HeuristicCheck {
    let has = |needle: &String| {
        let needle = needle.to_lowercase();
        sources.iter().find(|s| s.subject.to_lowercase().contains(&needle))
    };
    let missing: Vec<&str> = expected
        .iter()
        .filter(|e| has(e).is_none())
        .map(String::as_str)
        .collect();
    let wrong: Vec<&str> = forbidden.iter().filter_map(has).map(|s| s.subject.as_str()).collect();
    let passed = missing.is_empty() && wrong.is_empty();
    HeuristicCheck {
        name: "research_matches".into(),
        passed,
        expected: format!("matches {expected:?}, never {forbidden:?}"),
        actual: format!(
            "{} conversations matched; missing {missing:?}; wrongly matched {wrong:?}",
            sources.len()
        ),
        detail: if passed {
            "research matched the right conversations".into()
        } else {
            "research matched the wrong set of conversations".into()
        },
    }
}

fn check_no_email_sources(sources: &[crate::evals::harness::SourceSummary]) -> HeuristicCheck {
    let passed = sources.is_empty();
    HeuristicCheck {
        name: "no_email_sources".into(),
        passed,
        expected: "0 email sources".into(),
        actual: format!("{} email sources", sources.len()),
        detail: if passed {
            "mailbox RAG fed nothing to the model".into()
        } else {
            let ids: Vec<&str> = sources.iter().take(5).map(|s| s.email_id.as_str()).collect();
            format!("mailbox RAG fed emails to the model: {}", ids.join(", "))
        },
    }
}

fn check_tools(expected: &[String], trace: Option<&ChatTrace>) -> HeuristicCheck {
    let actual: Vec<String> = trace
        .map(|t| t.tool_calls.iter().map(|tc| tc.name.clone()).collect())
        .unwrap_or_default();

    let missing: Vec<String> = expected
        .iter()
        .filter(|needle| !actual.iter().any(|a| a == *needle))
        .cloned()
        .collect();

    let passed = missing.is_empty();

    HeuristicCheck {
        name: "tools_called".into(),
        passed,
        expected: expected.join(", "),
        actual: if actual.is_empty() {
            "<none>".into()
        } else {
            actual.join(", ")
        },
        detail: if passed {
            "all expected tools were invoked".into()
        } else {
            format!("missing tool calls: {}", missing.join(", "))
        },
    }
}

fn check_answer_nonempty(content: &str) -> HeuristicCheck {
    let trimmed = content.trim();
    let passed = !trimmed.is_empty();
    HeuristicCheck {
        name: "answer_nonempty".into(),
        passed,
        expected: "non-empty assistant reply".into(),
        actual: if passed {
            format!("{} chars", trimmed.chars().count())
        } else {
            "<empty>".into()
        },
        detail: if passed {
            "assistant produced text".into()
        } else {
            "assistant produced no text — treating as failure and skipping the judge".into()
        },
    }
}

fn check_answer_contains(expected: &[String], content: &str) -> HeuristicCheck {
    let lc = content.to_lowercase();
    let missing: Vec<String> = expected
        .iter()
        .filter(|needle| !lc.contains(&needle.to_lowercase()))
        .cloned()
        .collect();
    let passed = missing.is_empty();
    HeuristicCheck {
        name: "answer_contains".into(),
        passed,
        expected: expected.join(", "),
        actual: truncate(content, 200),
        detail: if passed {
            "all required substrings present".into()
        } else {
            format!("missing substrings: {}", missing.join(", "))
        },
    }
}

/// Case-insensitive substrings that must NOT appear in the final assistant
/// At least ONE of `alternatives` must appear in the answer (case-insensitive).
///
/// The disjunctive twin of [`check_answer_contains`], for anchoring on a fact
/// the model states in more than one shape. A retrieval case wants to assert
/// "the answer names the right email"; the model may do that by subject on one
/// run and by date on the next, so an AND of both flakes and either one alone
/// flakes on the other. List every phrasing that proves the same fact.
fn check_answer_contains_any(alternatives: &[String], content: &str) -> HeuristicCheck {
    let lc = content.to_lowercase();
    let hit = alternatives.iter().find(|needle| lc.contains(&needle.to_lowercase()));
    HeuristicCheck {
        name: "answer_contains_any".into(),
        passed: hit.is_some(),
        expected: format!("any of: {}", alternatives.join(", ")),
        actual: truncate(content, 200),
        detail: match hit {
            Some(found) => format!("matched alternative: {found}"),
            None => format!("none of the alternatives present: {}", alternatives.join(", ")),
        },
    }
}

/// content — the negative twin of [`check_answer_contains`]. Guards against
/// failure-mode phrasings ("I couldn't access your emails", "please paste the
/// content") that a positive anchor cannot distinguish from a real answer.
fn check_answer_not_contains(forbidden: &[String], content: &str) -> HeuristicCheck {
    let lc = content.to_lowercase();
    let present: Vec<String> = forbidden
        .iter()
        .filter(|needle| lc.contains(&needle.to_lowercase()))
        .cloned()
        .collect();
    let passed = present.is_empty();
    HeuristicCheck {
        name: "answer_not_contains".into(),
        passed,
        expected: format!("none of: {}", forbidden.join(", ")),
        actual: truncate(content, 200),
        detail: if passed {
            "no forbidden substrings present".into()
        } else {
            format!("forbidden substrings present: {}", present.join(", "))
        },
    }
}

/// Case-insensitive substrings that must appear in the serialized arguments of
/// at least one traced tool call. Pins *what the tools were asked*, not just
/// which tools ran — e.g. that `search_emails` was called with the exact
/// sender address the user wrote (and not a mangled variant).
fn check_tool_args_contains(expected: &[String], tool_calls: &[crate::models::ToolCallTrace]) -> HeuristicCheck {
    let serialized: Vec<String> = tool_calls
        .iter()
        .map(|tc| tc.arguments.to_string().to_lowercase())
        .collect();
    let missing: Vec<String> = expected
        .iter()
        .filter(|needle| {
            let n = needle.to_lowercase();
            !serialized.iter().any(|args| args.contains(&n))
        })
        .cloned()
        .collect();
    let passed = missing.is_empty();
    HeuristicCheck {
        name: "tool_args_contains".into(),
        passed,
        expected: expected.join(", "),
        actual: if serialized.is_empty() {
            "<no tool calls>".into()
        } else {
            truncate(&serialized.join(" | "), 300)
        },
        detail: if passed {
            "all expected substrings present in tool arguments".into()
        } else {
            format!("missing from every tool call's arguments: {}", missing.join(", "))
        },
    }
}

/// The mirror of [`check_tool_args_contains`]: every needle must be absent
/// from EVERY traced call's arguments.
fn check_tool_args_not_contains(expected: &[String], tool_calls: &[crate::models::ToolCallTrace]) -> HeuristicCheck {
    let serialized: Vec<String> = tool_calls
        .iter()
        .map(|tc| tc.arguments.to_string().to_lowercase())
        .collect();
    let present: Vec<String> = expected
        .iter()
        .filter(|needle| {
            let n = needle.to_lowercase();
            serialized.iter().any(|args| args.contains(&n))
        })
        .cloned()
        .collect();
    let passed = present.is_empty();
    HeuristicCheck {
        name: "tool_args_not_contains".into(),
        passed,
        expected: format!("absent: {}", expected.join(", ")),
        actual: if serialized.is_empty() {
            "<no tool calls>".into()
        } else {
            truncate(&serialized.join(" | "), 300)
        },
        detail: if passed {
            "no tool call carried a forbidden substring".into()
        } else {
            format!("forbidden substrings present in tool arguments: {}", present.join(", "))
        },
    }
}

/// Every citation — a bare `[n]`, resolved to the n-th source the way the UI
/// does, or an `email://ID` link, resolved to the source with that id — must
/// name a source whose subject contains one of `expected` (case-insensitive).
/// Catches an answer whose facts are right but whose citations point the user
/// at an unrelated email. An answer without citations passes; whether it is
/// grounded at all is the other anchors' job.
fn check_cited_subjects(expected: &[String], content: &str, sources: &[SourceSummary]) -> HeuristicCheck {
    static LINK_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    // Hard-coded literal that cannot fail by construction.
    #[allow(clippy::unwrap_used)]
    let link_re = LINK_RE.get_or_init(|| Regex::new(r"\]\(email://([^)\s]+)\)").unwrap());
    let wanted: Vec<String> = expected.iter().map(|s| s.to_lowercase()).collect();
    let mut cited: Vec<String> = Vec::new();
    let mut wrong: Vec<String> = Vec::new();
    let mut record = |marker: String, subject: Option<&str>| {
        let label = format!("{marker} → {}", subject.unwrap_or("<no source>"));
        let ok = subject.is_some_and(|s| {
            let s = s.to_lowercase();
            wanted.iter().any(|w| s.contains(w))
        });
        if !ok {
            wrong.push(label.clone());
        }
        cited.push(label);
    };
    for n in crate::services::chat::bare_citation_numbers(content) {
        let subject = sources
            .iter()
            .find(|s| usize::try_from(s.citation_number).ok() == Some(n))
            .map(|s| s.subject.as_str());
        record(format!("[{n}]"), subject);
    }
    for cap in link_re.captures_iter(content) {
        let id = &cap[1];
        let subject = sources.iter().find(|s| s.email_id == id).map(|s| s.subject.as_str());
        record(format!("email://{id}"), subject);
    }
    let passed = wrong.is_empty();
    HeuristicCheck {
        name: "cited_subjects".into(),
        passed,
        expected: format!("every citation names one of: {}", expected.join(", ")),
        actual: if cited.is_empty() {
            "<no citations>".into()
        } else {
            truncate(&cited.join(" | "), 300)
        },
        detail: if passed {
            "every citation resolves to an expected source".into()
        } else {
            format!("citations resolve to unexpected sources: {}", wrong.join(" | "))
        },
    }
}

fn check_title_pattern(pattern: &str, title: &str) -> EvalResult<HeuristicCheck> {
    let re = Regex::new(pattern)?;
    let passed = re.is_match(title);
    Ok(HeuristicCheck {
        name: "title_pattern".into(),
        passed,
        expected: pattern.to_string(),
        actual: title.to_string(),
        detail: if passed {
            "title matched pattern".into()
        } else {
            format!("title '{}' did not match /{}/", title, pattern)
        },
    })
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ToolCallTrace;

    // ── expected_answer_contains_any ────────────────────────────────────────
    //
    // `expected_answer_contains` is AND, which cannot express "the answer names
    // the right email". A model that retrieved correctly may report it by
    // SUBJECT on one run and by DATE on the next; requiring both flaked ~1 run
    // in 4, and requiring either one alone flaked on the other. A flaky guard
    // is worse than none — people learn to ignore it.

    #[test]
    fn contains_any_passes_when_a_single_alternative_is_present() {
        let alts = vec!["mayo".to_string(), "dificultades desarrollo".to_string()];
        let check = check_answer_contains_any(&alts, "Fue enviado el 5 de mayo de 2026.");
        assert!(check.passed, "one alternative is enough: {}", check.detail);
    }

    #[test]
    fn contains_any_passes_on_the_other_alternative() {
        let alts = vec!["mayo".to_string(), "dificultades desarrollo".to_string()];
        let check = check_answer_contains_any(&alts, "Asunto: RE: Chatbot: Dificultades desarrollo");
        assert!(check.passed, "case-insensitive, either side: {}", check.detail);
    }

    #[test]
    fn contains_any_fails_when_no_alternative_is_present() {
        let alts = vec!["mayo".to_string(), "dificultades desarrollo".to_string()];
        let check = check_answer_contains_any(&alts, "El correo es del 16 de febrero de 2026.");
        assert!(!check.passed, "the near-miss answer must not satisfy the anchor");
        assert!(
            check.detail.contains("mayo"),
            "the failure must list what it looked for: {}",
            check.detail
        );
    }

    fn tool_call(name: &str, args: serde_json::Value) -> ToolCallTrace {
        ToolCallTrace {
            name: name.to_string(),
            round: 0,
            arguments: args,
            result_preview: String::new(),
            result_chars: 0,
            elapsed_ms: 0,
        }
    }

    /// Minimal trace carrying just the tool calls under assertion.
    /// `ChatTrace` has no `Default`, so build the required fields explicitly.
    fn trace_with(tool_calls: Vec<ToolCallTrace>) -> ChatTrace {
        ChatTrace {
            route: crate::models::RouteDecision {
                mode: crate::models::RouteMode::ToolsFirst,
                reason: "test".into(),
                matched_keywords: vec![],
                classifier: "forced".into(),
            },
            retrieval: None,
            tool_calls,
            model: "test-model".into(),
            total_elapsed_ms: 0,
            tool_loop_ms: 0,
            llm_streaming_ms: None,
            help: None,
            research: None,
            llm_calls: vec![],
            steps: vec![],
        }
    }

    // ── help grounding ──────────────────────────────────────────────────
    // An app question is answered from the guides. The answer text cannot
    // prove it: the turn appends a `help://` link whenever any guide section
    // rode in the prompt, and mailbox RAG runs alongside the help lookup, so
    // a reply built from a user's email about the same topic still passes the
    // answer anchors. The trace records both corpora, so assert on it.

    fn trace_with_help(chunk_ids: &[&str]) -> ChatTrace {
        let mut trace = trace_with(vec![]);
        trace.help = Some(crate::models::HelpTrace {
            included: chunk_ids.len() as i32,
            chunk_ids: chunk_ids.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        });
        trace
    }

    // ── expected_min_research_emails ───────────────────────────────────────

    #[test]
    fn research_coverage_passes_when_enough_emails_were_read() {
        let mut trace = trace_with(vec![]);
        trace.research = Some(crate::models::ResearchTrace {
            emails_analyzed: 13,
            batches: 2,
            ..Default::default()
        });
        let check = check_research_coverage(10, Some(&trace));
        assert!(check.passed, "{}", check.detail);
    }

    #[test]
    fn research_coverage_fails_when_too_few_emails_were_read() {
        let mut trace = trace_with(vec![]);
        trace.research = Some(crate::models::ResearchTrace {
            emails_analyzed: 4,
            ..Default::default()
        });
        assert!(!check_research_coverage(10, Some(&trace)).passed);
    }

    #[test]
    fn research_coverage_fails_when_the_turn_did_not_research() {
        let check = check_research_coverage(10, Some(&trace_with(vec![])));
        assert!(!check.passed);
        assert!(check.actual.contains("no research"), "{}", check.actual);
    }

    #[test]
    fn help_pages_passes_when_a_section_of_an_expected_page_was_served() {
        let trace = trace_with_help(&["en/troubleshooting#1.0", "en/ai-features#2.0"]);
        let check = check_help_pages_any(&["ai-features".to_string()], Some(&trace));
        assert!(check.passed, "{}", check.detail);
    }

    #[test]
    fn help_pages_ignores_the_language_of_the_served_section() {
        let trace = trace_with_help(&["es/ai-features#2.0"]);
        let check = check_help_pages_any(&["ai-features".to_string()], Some(&trace));
        assert!(check.passed, "{}", check.detail);
    }

    #[test]
    fn help_pages_fails_when_only_other_pages_were_served() {
        let trace = trace_with_help(&["en/troubleshooting#1.0"]);
        let check = check_help_pages_any(&["ai-features".to_string()], Some(&trace));
        assert!(!check.passed);
        assert!(check.actual.contains("en/troubleshooting#1.0"), "{}", check.actual);
    }

    #[test]
    fn help_pages_fails_when_the_help_lookup_did_not_run() {
        let check = check_help_pages_any(&["ai-features".to_string()], Some(&trace_with(vec![])));
        assert!(!check.passed);
    }

    #[test]
    fn no_email_sources_passes_when_nothing_from_the_mailbox_was_fed() {
        assert!(check_no_email_sources(&[]).passed);
    }

    #[test]
    fn no_email_sources_fails_when_mailbox_rag_fed_the_prompt() {
        let check = check_no_email_sources(&[source(1, "demo_1", ""), source(2, "demo_2", "")]);
        assert!(!check.passed);
        assert!(check.actual.contains('2'), "{}", check.actual);
    }

    // ── tools_not_called ────────────────────────────────────────────────
    // Guards the inverse of `tools_called`: a turn that must answer in text.
    // Thread-bound chat exposes `generate_email_draft`, and a read-only
    // request ("traduceme este email") used to fire it and save a junk draft
    // instead of translating.

    #[test]
    fn tools_not_called_passes_when_no_tools_ran() {
        let trace = trace_with(vec![]);
        let check = check_tools_not_called(&["generate_email_draft".to_string()], Some(&trace));
        assert!(check.passed, "{check:?}");
    }

    #[test]
    fn tools_not_called_passes_when_a_different_tool_ran() {
        let trace = trace_with(vec![tool_call("search_emails", serde_json::json!({}))]);
        let check = check_tools_not_called(&["generate_email_draft".to_string()], Some(&trace));
        assert!(check.passed, "{check:?}");
    }

    #[test]
    fn tools_not_called_fails_when_the_forbidden_tool_ran() {
        let trace = trace_with(vec![tool_call(
            "generate_email_draft",
            serde_json::json!({"body": "…"}),
        )]);
        let check = check_tools_not_called(&["generate_email_draft".to_string()], Some(&trace));
        assert!(!check.passed, "{check:?}");
        assert!(check.detail.contains("generate_email_draft"));
    }

    #[test]
    fn tools_not_called_passes_when_trace_is_missing() {
        // No trace at all means no tool ran, which satisfies the constraint.
        let check = check_tools_not_called(&["generate_email_draft".to_string()], None);
        assert!(check.passed, "{check:?}");
    }

    #[test]
    fn answer_not_contains_passes_when_forbidden_text_absent() {
        let check = check_answer_not_contains(
            &["no he podido acceder".to_string()],
            "Aquí tienes el análisis de los correos.",
        );
        assert!(check.passed);
    }

    #[test]
    fn answer_not_contains_fails_case_insensitively_when_forbidden_text_present() {
        let check = check_answer_not_contains(
            &["No he podido ACCEDER".to_string()],
            "no he podido acceder a los emails. Por favor pega el contenido.",
        );
        assert!(!check.passed);
        assert!(
            check.detail.contains("forbidden"),
            "detail names the failure: {}",
            check.detail
        );
    }

    #[test]
    fn tool_args_contains_passes_when_any_call_carries_the_substring() {
        let calls = vec![
            tool_call("search_emails", serde_json::json!({})),
            tool_call(
                "search_emails",
                serde_json::json!({"from": "cosasdefreelance@substack.com", "limit": 25}),
            ),
        ];
        let check = check_tool_args_contains(&["cosasdefreelance@substack.com".to_string()], &calls);
        assert!(check.passed);
    }

    #[test]
    fn tool_args_contains_fails_when_no_call_carries_the_substring() {
        // The mangled-address failure: the model searched a translated variant
        // instead of the address the user actually wrote.
        let calls = vec![tool_call(
            "search_emails",
            serde_json::json!({"from": "thingsdefreelance@substack.com"}),
        )];
        let check = check_tool_args_contains(&["cosasdefreelance@substack.com".to_string()], &calls);
        assert!(!check.passed);
        assert!(check.detail.contains("cosasdefreelance"), "detail: {}", check.detail);
    }

    #[test]
    fn tool_args_contains_fails_on_empty_tool_calls() {
        let check = check_tool_args_contains(&["x@y.com".to_string()], &[]);
        assert!(!check.passed);
    }

    #[test]
    fn tool_args_not_contains_fails_when_any_call_carries_the_substring() {
        // The planner-limit failure: the FIRST call asked for 5 rows and a
        // later one for 25, which the positive check alone accepts.
        let calls = vec![
            tool_call(
                "search_emails",
                serde_json::json!({"intent": "introduction", "limit": 5}),
            ),
            tool_call(
                "search_emails",
                serde_json::json!({"intent": "introduction", "limit": 25}),
            ),
        ];
        let check = check_tool_args_not_contains(&["\"limit\":5".to_string()], &calls);
        assert!(!check.passed);
        assert!(check.detail.contains("limit"), "detail: {}", check.detail);
    }

    #[test]
    fn tool_args_not_contains_passes_when_no_call_carries_the_substring() {
        let calls = vec![tool_call(
            "search_emails",
            serde_json::json!({"intent": "introduction", "limit": 25}),
        )];
        let check = check_tool_args_not_contains(&["\"limit\":5".to_string()], &calls);
        assert!(check.passed);
    }

    #[test]
    fn tool_args_not_contains_passes_on_empty_tool_calls() {
        // Nothing ran, so nothing forbidden was asked for. A missing call is
        // the positive check's job to catch, not this one's.
        let check = check_tool_args_not_contains(&["\"limit\":5".to_string()], &[]);
        assert!(check.passed);
    }

    // ── expected_cited_subjects ─────────────────────────────────────────────
    //
    // The UI resolves a bare `[n]` to the n-th pre-retrieved source. A model
    // that found its evidence through a tool (whose results carry no number)
    // numbered those emails itself, so `[1]` rendered as an unrelated shipping
    // notice while the answer's facts came from the vendor's support mail.

    // ── expected / forbidden research matches ─────────────────────────────

    #[test]
    fn research_matches_pass_with_every_expected_and_no_forbidden_conversation() {
        let sources = vec![source(1, "a", "Re: Proposal: PrivacyHub migration")];
        let check = check_research_matches(&["privacyhub".into()], &["translation".into()], &sources);
        assert!(check.passed, "{}", check.actual);
    }

    #[test]
    fn research_matches_fail_on_a_forbidden_conversation() {
        let sources = vec![
            source(1, "a", "Proposal: PrivacyHub migration"),
            source(2, "b", "Re: Quote request: German translation"),
        ];
        let check = check_research_matches(&["privacyhub".into()], &["translation".into()], &sources);
        assert!(!check.passed);
        assert!(check.actual.contains("German translation"), "{}", check.actual);
    }

    #[test]
    fn research_matches_fail_when_an_expected_conversation_is_missing() {
        let sources = vec![source(1, "a", "Something else")];
        assert!(!check_research_matches(&["privacyhub".into()], &[], &sources).passed);
    }

    fn source(n: i32, email_id: &str, subject: &str) -> SourceSummary {
        SourceSummary {
            citation_number: n,
            email_id: email_id.into(),
            subject: subject.into(),
            sender: String::new(),
            sender_email: String::new(),
            relevance_score: None,
            body_snippet: String::new(),
        }
    }

    fn shop_then_support_sources() -> Vec<SourceSummary> {
        vec![
            source(1, "eml-ship", "Your order has shipped"),
            source(2, "eml-review", "How was your recent purchase?"),
            source(3, "eml-claim", "Your warranty claim is approved"),
        ]
    }

    #[test]
    fn cited_subjects_fails_when_a_citation_resolves_to_an_unrelated_source() {
        let answer = "Write to help@vendor.example [1].\n\n[1](email://eml-claim)";
        let check = check_cited_subjects(&["warranty claim".to_string()], answer, &shop_then_support_sources());
        assert!(!check.passed, "{check:?}");
        assert!(
            check.detail.contains("[1]"),
            "detail names the citation: {}",
            check.detail
        );
    }

    #[test]
    fn cited_subjects_passes_when_every_citation_resolves_to_an_expected_source() {
        let answer = "Write to help@vendor.example [3].";
        let check = check_cited_subjects(&["warranty claim".to_string()], answer, &shop_then_support_sources());
        assert!(check.passed, "{check:?}");
    }

    #[test]
    fn cited_subjects_ignores_numbered_link_labels() {
        // `[1](email://…)` is a link to the email it names, not a source citation.
        let answer = "Write to help@vendor.example [1](email://eml-claim).";
        let check = check_cited_subjects(&["warranty claim".to_string()], answer, &shop_then_support_sources());
        assert!(check.passed, "{check:?}");
    }

    #[test]
    fn cited_subjects_passes_when_the_answer_has_no_citations() {
        let answer = "Write to [help@vendor.example](email://eml-claim).";
        let check = check_cited_subjects(&["warranty claim".to_string()], answer, &shop_then_support_sources());
        assert!(check.passed, "{check:?}");
    }

    #[test]
    fn cited_subjects_resolves_an_email_link_through_the_sources() {
        // On a tool turn the sources are the emails the tools returned and the
        // answer cites them with `email://` links — a link to the shipping
        // notice is as wrong as a `[1]` that opened it.
        let answer = "Write to [help@vendor.example](email://eml-ship).";
        let check = check_cited_subjects(&["warranty claim".to_string()], answer, &shop_then_support_sources());
        assert!(!check.passed, "{check:?}");
        assert!(
            check.detail.contains("eml-ship"),
            "detail names the link: {}",
            check.detail
        );
    }

    #[test]
    fn cited_subjects_passes_when_every_link_resolves_to_an_expected_source() {
        let answer = "Write to [help@vendor.example](email://eml-claim) [3].";
        let check = check_cited_subjects(&["warranty claim".to_string()], answer, &shop_then_support_sources());
        assert!(check.passed, "{check:?}");
    }

    #[test]
    fn cited_subjects_fails_on_a_link_to_an_email_outside_the_sources() {
        let answer = "Write to [help@vendor.example](email://eml-unknown).";
        let check = check_cited_subjects(&["warranty claim".to_string()], answer, &shop_then_support_sources());
        assert!(!check.passed, "{check:?}");
    }

    #[test]
    fn cited_subjects_fails_on_a_citation_with_no_source() {
        let answer = "Write to help@vendor.example [7].";
        let check = check_cited_subjects(&["warranty claim".to_string()], answer, &shop_then_support_sources());
        assert!(!check.passed, "{check:?}");
    }
}
