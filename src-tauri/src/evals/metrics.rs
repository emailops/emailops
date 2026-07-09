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

/// Case-insensitive substrings that must NOT appear in the final assistant
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
}
