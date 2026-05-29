// Deterministic (non-LLM) heuristic assertions.
//
// These run unconditionally on every case and are what decides pass/fail in
// CI. The judge metrics (see `judge.rs`) are separate and only influence the
// score numbers shown in the report.

use regex::Regex;

use crate::evals::case_loader::EvalCase;
use crate::evals::harness::CaseOutcome;
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

    if !case.expected_answer_contains.is_empty() {
        checks.push(check_answer_contains(
            &case.expected_answer_contains,
            &outcome.assistant_content,
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
