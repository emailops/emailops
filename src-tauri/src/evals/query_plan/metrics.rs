// Deterministic scoring for the query-planner harness.
//
// Pure: a case plus the plan the model produced in, a per-field verdict out.
// No judge — a plan is a JSON object with known fields, so "is this right?"
// is answerable without another model.

use crate::evals::query_plan::case_loader::PlanCase;
use crate::services::chat::planner::SearchPlan;

/// One field's verdict.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldCheck {
    pub field: String,
    pub expected: String,
    pub actual: String,
    pub passed: bool,
}

/// Every verdict for one case.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanReport {
    pub checks: Vec<FieldCheck>,
    pub passed: bool,
}

/// The plan the planner produced, or `None` when it deferred.
pub type PlannedOutcome<'a> = Option<&'a SearchPlan>;

/// Read one plan field as text, so expectations stay YAML-simple. `None` means
/// the plan does not carry that field; `Some` values render the way the case
/// file writes them.
pub fn field_value(plan: &SearchPlan, field: &str) -> Result<Option<String>, String> {
    let value = match field {
        "query" => plan.query.clone(),
        "from" => plan.from.clone(),
        "to" => plan.to.clone(),
        "subject" => plan.subject.clone(),
        "intent" => plan.intent.clone(),
        "topic" => plan.topic.clone(),
        "mode" => plan.mode.clone(),
        "since" => plan.since.clone(),
        "until" => plan.until.clone(),
        "order" => plan.order.clone(),
        "limit" => plan.limit.map(|n| n.to_string()),
        "unread" => plan.unread.map(|b| b.to_string()),
        other => return Err(format!("unknown plan field `{other}`")),
    };
    Ok(value)
}

fn matches(field: &str, expected: &str, actual: &str) -> bool {
    match field {
        // Numbers and flags are exact; text is a case-insensitive substring so
        // `from: marisol` accepts whatever spelling of the sender the planner
        // chose.
        "limit" | "unread" | "order" | "mode" => actual.eq_ignore_ascii_case(expected),
        _ => actual.to_lowercase().contains(&expected.to_lowercase()),
    }
}

fn render_expected(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        other => format!("{other:?}"),
    }
}

/// Score one case against the plan the model produced.
pub fn evaluate(case: &PlanCase, planned: PlannedOutcome<'_>) -> PlanReport {
    let mut checks = Vec::new();

    if case.expect_defer {
        let passed = planned.is_none();
        checks.push(FieldCheck {
            field: "defer".to_string(),
            expected: "defer to the tool loop".to_string(),
            actual: if passed { "deferred" } else { "planned a search" }.to_string(),
            passed,
        });
        let passed = checks.iter().all(|c| c.passed);
        return PlanReport { checks, passed };
    }

    let Some(plan) = planned else {
        checks.push(FieldCheck {
            field: "plan".to_string(),
            expected: "a search plan".to_string(),
            actual: "deferred".to_string(),
            passed: false,
        });
        return PlanReport { checks, passed: false };
    };

    for (field, expected) in &case.expect {
        let expected = render_expected(expected);
        match field_value(plan, field) {
            Err(message) => checks.push(FieldCheck {
                field: field.clone(),
                expected,
                actual: message,
                passed: false,
            }),
            Ok(actual) => {
                let actual = actual.unwrap_or_default();
                let passed = !actual.is_empty() && matches(field, &expected, &actual);
                checks.push(FieldCheck {
                    field: field.clone(),
                    expected,
                    actual: if actual.is_empty() {
                        "(absent)".to_string()
                    } else {
                        actual
                    },
                    passed,
                });
            }
        }
    }

    for field in &case.absent {
        match field_value(plan, field) {
            Err(message) => checks.push(FieldCheck {
                field: field.clone(),
                expected: "(absent)".to_string(),
                actual: message,
                passed: false,
            }),
            Ok(actual) => checks.push(FieldCheck {
                field: field.clone(),
                expected: "(absent)".to_string(),
                actual: actual.clone().unwrap_or_else(|| "(absent)".to_string()),
                passed: actual.is_none(),
            }),
        }
    }

    let passed = checks.iter().all(|c| c.passed);
    PlanReport { checks, passed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::chat::planner::{parse_plan, Plan};

    fn case(yaml: &str) -> PlanCase {
        serde_yaml::from_str(yaml).expect("case")
    }

    fn plan(json: &str) -> SearchPlan {
        match parse_plan(json) {
            Plan::Search(p) => *p,
            Plan::Defer => panic!("expected a plan for {json}"),
        }
    }

    #[test]
    fn an_expected_field_matches_on_substring() {
        let c = case("id: x\nquestion: q\nexpect:\n  from: marisol\n");
        let p = plan(r#"{"from": "Marisol Vega"}"#);

        let report = evaluate(&c, Some(&p));

        assert!(report.passed, "{:?}", report.checks);
        assert_eq!(report.checks[0].actual, "Marisol Vega");
    }

    #[test]
    fn a_forbidden_field_fails_when_the_planner_sets_it() {
        // The reported failure: a date window on a question with no date.
        let c = case("id: x\nquestion: q\nabsent: [since, until]\n");
        let p = plan(r#"{"from": "x", "since": "2026-09-18", "until": "2026-09-19"}"#);

        let report = evaluate(&c, Some(&p));

        assert!(!report.passed);
        let since = report.checks.iter().find(|c| c.field == "since").expect("since check");
        assert_eq!(since.actual, "2026-09-18");
        assert!(report.checks.iter().any(|c| c.field == "until" && !c.passed));
    }

    #[test]
    fn a_forbidden_field_passes_when_the_plan_leaves_it_out() {
        let c = case("id: x\nquestion: q\nabsent: [intent, topic]\n");
        let p = plan(r#"{"from": "x"}"#);

        assert!(evaluate(&c, Some(&p)).passed);
    }

    #[test]
    fn limit_and_order_compare_exactly() {
        let c = case("id: x\nquestion: q\nexpect:\n  limit: 1\n  order: oldest\n");
        let p = plan(r#"{"from": "x", "limit": 1, "order": "oldest"}"#);
        assert!(evaluate(&c, Some(&p)).passed);

        let p = plan(r#"{"from": "x", "limit": 5, "order": "oldest"}"#);
        let report = evaluate(&c, Some(&p));
        assert!(!report.passed);
        assert!(report.checks.iter().any(|c| c.field == "limit" && c.actual == "5"));
    }

    #[test]
    fn a_defer_case_wants_no_plan() {
        let c = case("id: x\nquestion: q\nexpect_defer: true\n");
        assert!(evaluate(&c, None).passed);
        assert!(!evaluate(&c, Some(&plan(r#"{"from": "x"}"#))).passed);
    }

    #[test]
    fn a_search_case_fails_when_the_planner_defers() {
        let c = case("id: x\nquestion: q\nexpect:\n  from: marisol\n");
        let report = evaluate(&c, None);
        assert!(!report.passed);
        assert_eq!(report.checks[0].actual, "deferred");
    }

    #[test]
    fn an_unknown_field_name_fails_loudly_instead_of_passing() {
        let c = case("id: x\nquestion: q\nexpect:\n  sender: marisol\n");
        let report = evaluate(&c, Some(&plan(r#"{"from": "x"}"#)));
        assert!(!report.passed);
        assert!(
            report.checks[0].actual.contains("unknown plan field"),
            "{:?}",
            report.checks
        );
    }
}
