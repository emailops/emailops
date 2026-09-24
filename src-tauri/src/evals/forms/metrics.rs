// Scoring one filled form against a case. Pure — no provider, no DB — so the
// rules that decide pass/fail are unit-tested here rather than inferred from a
// model run.

use super::case_loader::FormCase;
use crate::services::forms::FormFill;
use serde_json::Value;

/// One assertion about the filled form.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldCheck {
    pub field: String,
    pub expected: String,
    pub actual: String,
    pub ok: bool,
}

impl FieldCheck {
    fn new(field: impl Into<String>, expected: impl Into<String>, actual: impl Into<String>, ok: bool) -> Self {
        Self {
            field: field.into(),
            expected: expected.into(),
            actual: actual.into(),
            ok,
        }
    }
    pub fn passed(&self) -> bool {
        self.ok
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormReport {
    pub checks: Vec<FieldCheck>,
    pub passed: bool,
}

/// A value rendered for the report, short enough to read in a table.
fn show(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(59).collect::<String>())
    } else {
        s
    }
}

/// Does `actual` satisfy `expected`?
///
/// Strings match case-insensitively on substring, because a label the model
/// wrote ("Importe total") should satisfy an expectation of "importe" without
/// the case pinning its exact prose. Lists match as sets. Everything else
/// matches exactly.
fn value_matches(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::String(want), Value::String(got)) => got.to_lowercase().contains(&want.to_lowercase()),
        (Value::Array(want), Value::Array(got)) => {
            want.len() == got.len() && want.iter().all(|w| got.iter().any(|g| value_matches(w, g)))
        }
        (want, got) => want == got,
    }
}

/// Every `key` / `label` of the form's object list, lowercased, for concept
/// coverage. Non-object rows and rows without either field contribute nothing.
fn column_terms(values: &serde_json::Map<String, Value>) -> Vec<String> {
    values
        .get("columns")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_object)
                .flat_map(|row| {
                    ["key", "label"]
                        .iter()
                        .filter_map(|k| row.get(*k).and_then(Value::as_str))
                        .map(str::to_lowercase)
                        .collect::<Vec<_>>()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn column_types(values: &serde_json::Map<String, Value>) -> Vec<String> {
    values
        .get("columns")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_object)
                .filter_map(|row| row.get("type").and_then(Value::as_str))
                .map(str::to_lowercase)
                .collect()
        })
        .unwrap_or_default()
}

fn column_count(values: &serde_json::Map<String, Value>) -> usize {
    values
        .get("columns")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

/// Score one filled form. `None` means the model produced nothing usable,
/// which fails every case.
pub fn evaluate(case: &FormCase, fill: Option<&FormFill>) -> FormReport {
    let Some(fill) = fill else {
        return FormReport {
            checks: vec![FieldCheck::new("fill", "a parseable filled form", "(nothing)", false)],
            passed: false,
        };
    };
    let mut checks = Vec::new();

    for key in &case.expect_present {
        let got = fill.values.get(key);
        checks.push(FieldCheck::new(
            key,
            "filled",
            got.map(show).unwrap_or_else(|| "(absent)".into()),
            got.is_some(),
        ));
    }

    for key in &case.expect_absent {
        let got = fill.values.get(key);
        checks.push(FieldCheck::new(
            key,
            "not filled",
            got.map(show).unwrap_or_else(|| "(absent)".into()),
            got.is_none(),
        ));
    }

    for (key, want_yaml) in &case.expect_values {
        let want: Value = serde_json::to_value(want_yaml).unwrap_or(Value::Null);
        match fill.values.get(key) {
            Some(got) => checks.push(FieldCheck::new(key, show(&want), show(got), value_matches(&want, got))),
            None => checks.push(FieldCheck::new(key, show(&want), "(absent)", false)),
        }
    }

    if let Some(min) = case.expect_min_columns {
        let got = column_count(&fill.values);
        checks.push(FieldCheck::new(
            "columns",
            format!(">= {min}"),
            got.to_string(),
            got >= min,
        ));
    }

    if !case.expect_columns_covering.is_empty() {
        let terms = column_terms(&fill.values);
        for concept in &case.expect_columns_covering {
            let needle = concept.to_lowercase();
            let covered = terms.iter().any(|t| t.contains(&needle));
            checks.push(FieldCheck::new(
                format!("column:{concept}"),
                "a column for it",
                if covered { "covered" } else { "(missing)" },
                covered,
            ));
        }
    }

    for want_type in &case.expect_column_types {
        let types = column_types(&fill.values);
        let present = types.iter().any(|t| t == &want_type.to_lowercase());
        checks.push(FieldCheck::new(
            format!("type:{want_type}"),
            "used by some column",
            types.join(", "),
            present,
        ));
    }

    if case.expect_complete {
        let missing = fill.missing_required.join(", ");
        checks.push(FieldCheck::new(
            "required",
            "all filled",
            if missing.is_empty() {
                "all filled".into()
            } else {
                missing
            },
            fill.missing_required.is_empty(),
        ));
    }

    let passed = checks.iter().all(FieldCheck::passed);
    FormReport { checks, passed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn case(yaml: &str) -> FormCase {
        serde_yaml::from_str(yaml).expect("case parses")
    }

    fn fill(values: serde_json::Value, missing: &[&str]) -> FormFill {
        FormFill {
            form_id: "lens.create".into(),
            values: values.as_object().cloned().unwrap_or_default(),
            missing_required: missing.iter().map(|s| (*s).to_string()).collect(),
            dropped: Vec::new(),
        }
    }

    const BASE: &str = "id: x\nform: lens.create\nrequest: q\n";

    #[test]
    fn nothing_usable_fails_every_case() {
        let report = evaluate(&case(BASE), None);
        assert!(!report.passed);
        assert_eq!(report.checks[0].field, "fill");
    }

    #[test]
    fn a_required_key_that_is_filled_passes() {
        let c = case(&format!("{BASE}expect_present: [name]\nexpect_complete: false\n"));
        assert!(evaluate(&c, Some(&fill(json!({"name": "Facturas"}), &[]))).passed);
    }

    #[test]
    fn a_required_key_the_model_skipped_fails() {
        let c = case(&format!("{BASE}expect_present: [name]\nexpect_complete: false\n"));
        let report = evaluate(&c, Some(&fill(json!({"icon": "x"}), &[])));
        assert!(!report.passed);
        assert_eq!(report.checks[0].actual, "(absent)");
    }

    #[test]
    fn a_key_the_model_invented_fails_an_absent_expectation() {
        let c = case(&format!(
            "{BASE}expect_absent: [scopeSenderDomains]\nexpect_complete: false\n"
        ));
        let report = evaluate(&c, Some(&fill(json!({"scopeSenderDomains": ["acme.com"]}), &[])));
        assert!(!report.passed, "over-filling must fail: {report:?}");
    }

    #[test]
    fn a_string_expectation_matches_on_substring_case_insensitively() {
        let c = case(&format!(
            "{BASE}expect_values:\n  name: factura\nexpect_complete: false\n"
        ));
        assert!(evaluate(&c, Some(&fill(json!({"name": "Facturas de proveedores"}), &[]))).passed);
    }

    #[test]
    fn a_list_expectation_matches_as_a_set() {
        let c = case(&format!(
            "{BASE}expect_values:\n  scopeMailboxes: [inbox]\nexpect_complete: false\n"
        ));
        assert!(evaluate(&c, Some(&fill(json!({"scopeMailboxes": ["inbox"]}), &[]))).passed);
        assert!(!evaluate(&c, Some(&fill(json!({"scopeMailboxes": ["inbox", "sent"]}), &[]))).passed);
    }

    #[test]
    fn a_column_minimum_counts_the_object_list() {
        let c = case(&format!("{BASE}expect_min_columns: 2\nexpect_complete: false\n"));
        let two = json!({"columns": [{"key": "a"}, {"key": "b"}]});
        assert!(evaluate(&c, Some(&fill(two, &[]))).passed);
        let one = json!({"columns": [{"key": "a"}]});
        assert!(!evaluate(&c, Some(&fill(one, &[]))).passed);
    }

    #[test]
    fn a_concept_is_covered_by_a_column_key_or_label() {
        let c = case(&format!(
            "{BASE}expect_columns_covering: [importe]\nexpect_complete: false\n"
        ));
        let by_label = json!({"columns": [{"key": "amount", "label": "Importe total"}]});
        assert!(evaluate(&c, Some(&fill(by_label, &[]))).passed);
        let by_key = json!({"columns": [{"key": "importe", "label": "Total"}]});
        assert!(evaluate(&c, Some(&fill(by_key, &[]))).passed);
    }

    #[test]
    fn a_concept_no_column_mentions_fails() {
        let c = case(&format!(
            "{BASE}expect_columns_covering: [proveedor]\nexpect_complete: false\n"
        ));
        let report = evaluate(&c, Some(&fill(json!({"columns": [{"key": "amount"}]}), &[])));
        assert!(!report.passed);
        assert_eq!(report.checks[0].actual, "(missing)");
    }

    #[test]
    fn a_required_column_type_must_be_used_by_some_column() {
        let c = case(&format!(
            "{BASE}expect_column_types: [currency]\nexpect_complete: false\n"
        ));
        let money = json!({"columns": [{"key": "a", "type": "currency"}]});
        assert!(evaluate(&c, Some(&fill(money, &[]))).passed);
        let text = json!({"columns": [{"key": "a", "type": "string"}]});
        assert!(!evaluate(&c, Some(&fill(text, &[]))).passed);
    }

    #[test]
    fn completeness_is_checked_by_default() {
        let c = case(BASE);
        assert!(evaluate(&c, Some(&fill(json!({"name": "X"}), &[]))).passed);
        assert!(!evaluate(&c, Some(&fill(json!({"name": "X"}), &["columns"]))).passed);
    }

    #[test]
    fn a_case_that_opts_out_of_completeness_ignores_missing_fields() {
        let c = case(&format!("{BASE}expect_complete: false\n"));
        assert!(evaluate(&c, Some(&fill(json!({"name": "X"}), &["columns"]))).passed);
    }

    #[test]
    fn a_long_value_is_truncated_for_the_report() {
        let c = case(&format!("{BASE}expect_present: [promptText]\nexpect_complete: false\n"));
        let long = "x".repeat(200);
        let report = evaluate(&c, Some(&fill(json!({"promptText": long}), &[])));
        assert!(report.checks[0].actual.chars().count() <= 60);
    }
}
