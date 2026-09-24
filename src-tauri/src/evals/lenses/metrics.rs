// Scoring one extracted Lens row against a case. Pure — no provider, no DB —
// so the rules that decide pass/fail are unit-tested here.

use super::case_loader::LensCase;

/// One assertion about the extracted row.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldCheck {
    pub field: String,
    pub expected: String,
    pub actual: String,
    pub ok: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensReport {
    pub checks: Vec<FieldCheck>,
    pub passed: bool,
}

const EMPTY: &str = "(empty)";

/// A value rendered for the report, short enough to read in a table.
fn show(v: &serde_json::Value) -> String {
    let s = match v {
        serde_json::Value::Null => return EMPTY.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(79).collect::<String>())
    } else {
        s
    }
}

fn is_empty(v: &serde_json::Value) -> bool {
    v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty())
}

/// Does `actual` satisfy `expected`? See `LensCase::expect` for the rules.
fn value_matches(expected: &serde_yaml::Value, actual: &serde_json::Value) -> bool {
    use serde_yaml::Value as Y;
    match expected {
        Y::Null => is_empty(actual),
        Y::String(want) => actual
            .as_str()
            .is_some_and(|got| got.to_lowercase().contains(&want.to_lowercase())),
        Y::Bool(want) => actual.as_bool() == Some(*want),
        Y::Number(want) => match (want.as_f64(), actual.as_f64()) {
            (Some(w), Some(a)) => (w - a).abs() < 1e-9,
            _ => false,
        },
        // Lists and maps are not used by any column type the templates emit.
        _ => false,
    }
}

fn describe(expected: &serde_yaml::Value) -> String {
    match expected {
        serde_yaml::Value::Null => EMPTY.to_string(),
        serde_yaml::Value::String(s) => format!("contains \"{s}\""),
        other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
    }
}

/// Score one row. `row` is `None` when the extraction itself failed; every
/// value check then fails, because the user gets no row at all.
pub fn evaluate(case: &LensCase, row: Option<&serde_json::Value>, in_scope: bool) -> LensReport {
    let mut checks = Vec::new();
    if let Some(want) = case.expect_in_scope {
        checks.push(FieldCheck {
            field: "scope".into(),
            expected: if want { "picked up" } else { "left out" }.into(),
            actual: if in_scope { "picked up" } else { "left out" }.into(),
            ok: want == in_scope,
        });
    }
    for (field, expected) in &case.expect {
        let actual = row
            .and_then(|r| r.get(field))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        checks.push(FieldCheck {
            field: field.clone(),
            expected: describe(expected),
            actual: if row.is_some() {
                show(&actual)
            } else {
                "(extraction failed)".into()
            },
            ok: row.is_some() && value_matches(expected, &actual),
        });
    }
    let passed = checks.iter().all(|c| c.ok);
    LensReport { checks, passed }
}

/// One row per template column (plus the scope check first), for the report:
/// asserted columns carry their verdict, the rest show what was extracted with
/// `—` as the expectation.
pub fn field_table(
    report: &LensReport,
    schema: &crate::models::lens::LensSchema,
    row: Option<&serde_json::Value>,
) -> Vec<FieldCheck> {
    let mut table: Vec<FieldCheck> = report.checks.iter().filter(|c| c.field == "scope").cloned().collect();
    for col in &schema.columns {
        if let Some(check) = report.checks.iter().find(|c| c.field == col.key) {
            table.push(check.clone());
            continue;
        }
        table.push(FieldCheck {
            field: col.key.clone(),
            expected: "—".into(),
            actual: match row {
                Some(r) => show(r.get(&col.key).unwrap_or(&serde_json::Value::Null)),
                None => "(extraction failed)".into(),
            },
            ok: true,
        });
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn case(yaml_expect: &str, in_scope: Option<bool>) -> LensCase {
        let mut c: LensCase = serde_yaml::from_str(&format!(
            "id: x\ntemplate: t\nemail:\n  from_name: a\n  from_email: a@b.example\n  subject: s\n  body: b\nexpect:\n{yaml_expect}"
        ))
        .expect("case parses");
        c.expect_in_scope = in_scope;
        c
    }

    fn failed(report: &LensReport) -> Vec<&str> {
        report
            .checks
            .iter()
            .filter(|c| !c.ok)
            .map(|c| c.field.as_str())
            .collect()
    }

    fn schema(keys: &[&str]) -> crate::models::lens::LensSchema {
        crate::models::lens::LensSchema {
            columns: keys
                .iter()
                .map(|k| crate::models::lens::LensColumn {
                    key: (*k).into(),
                    label: (*k).into(),
                    column_type: crate::models::lens::LensColumnType::String,
                    description: String::new(),
                    enum_values: None,
                    required: false,
                    is_unique_key: false,
                })
                .collect(),
        }
    }

    #[test]
    fn the_field_table_lists_every_column_with_unchecked_ones_marked() {
        // The report shows the whole row, not just the asserted columns: a
        // reader needs to see what the template actually wrote down.
        let c = case("  contact_email: ana@client.example\n", Some(true));
        let row = json!({"contact_email": "ana@client.example", "phone": null, "summary": "Pide presupuesto"});
        let report = evaluate(&c, Some(&row), true);
        let table = field_table(&report, &schema(&["contact_email", "phone", "summary"]), Some(&row));
        let got: Vec<(&str, &str, &str, bool)> = table
            .iter()
            .map(|r| (r.field.as_str(), r.expected.as_str(), r.actual.as_str(), r.ok))
            .collect();
        assert_eq!(
            got,
            vec![
                ("scope", "picked up", "picked up", true),
                (
                    "contact_email",
                    "contains \"ana@client.example\"",
                    "ana@client.example",
                    true
                ),
                ("phone", "—", "(empty)", true),
                ("summary", "—", "Pide presupuesto", true),
            ]
        );
    }

    #[test]
    fn the_field_table_keeps_the_failing_verdict() {
        let c = case("  phone: \"600\"\n", None);
        let row = json!({"phone": null});
        let report = evaluate(&c, Some(&row), true);
        let table = field_table(&report, &schema(&["phone"]), Some(&row));
        assert_eq!(table.len(), 1);
        assert!(!table[0].ok);
    }

    #[test]
    fn a_row_matching_every_expectation_passes() {
        let c = case(
            "  contact_email: Ana@Client.example\n  request_type: quote_request\n",
            Some(true),
        );
        let row = json!({"contact_email": "ana@client.example", "request_type": "quote_request", "summary": "x"});
        let r = evaluate(&c, Some(&row), true);
        assert!(r.passed, "{:?}", r.checks);
        assert_eq!(r.checks.len(), 3, "two fields plus the scope check");
    }

    #[test]
    fn strings_match_on_a_case_insensitive_substring() {
        // A summary need not reproduce prose; it must mention the point.
        let c = case("  summary: presupuesto\n", None);
        let row = json!({"summary": "Piden un Presupuesto para la auditoría"});
        assert!(evaluate(&c, Some(&row), true).passed);
    }

    #[test]
    fn null_means_the_column_must_stay_empty() {
        let c = case("  company: null\n", None);
        assert!(evaluate(&c, Some(&json!({"company": null})), true).passed);
        assert!(evaluate(&c, Some(&json!({})), true).passed, "a missing column is empty");
        let r = evaluate(&c, Some(&json!({"company": "Gmail"})), true);
        assert_eq!(failed(&r), vec!["company"]);
    }

    #[test]
    fn an_empty_column_fails_a_value_expectation() {
        let c = case("  contact_email: sam@mail.example\n", None);
        let r = evaluate(&c, Some(&json!({"contact_email": null})), true);
        assert_eq!(failed(&r), vec!["contact_email"]);
        assert_eq!(r.checks[0].actual, "(empty)");
    }

    #[test]
    fn booleans_and_numbers_match_exactly() {
        let c = case("  paid: true\n  amount: 12.5\n", None);
        assert!(evaluate(&c, Some(&json!({"paid": true, "amount": 12.5})), true).passed);
        let r = evaluate(&c, Some(&json!({"paid": false, "amount": 12.0})), true);
        assert_eq!(failed(&r), vec!["amount", "paid"]);
    }

    #[test]
    fn the_scope_check_fails_when_the_template_misses_the_email() {
        let c = case("  summary: x\n", Some(true));
        let r = evaluate(&c, Some(&json!({"summary": "x"})), false);
        assert_eq!(failed(&r), vec!["scope"]);
    }

    #[test]
    fn a_failed_extraction_fails_every_value_check() {
        let c = case("  contact_email: sam@mail.example\n", None);
        let r = evaluate(&c, None, true);
        assert!(!r.passed);
        assert_eq!(failed(&r), vec!["contact_email"]);
    }
}
