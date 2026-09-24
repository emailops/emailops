// YAML schema for the form-filling harness.
//
// A case is a request plus what the filled form must (and must not) contain.
// Nothing here runs a chat turn: filling a form is one small completion, so its
// quality is measured on its own, in seconds per case.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::evals::{EvalError, EvalResult};

/// One form-filling case.
#[derive(Debug, Clone, Deserialize)]
pub struct FormCase {
    /// Stable id, used by `--case` and as the report anchor.
    pub id: String,

    /// Which registered form to fill (`lens.create`).
    pub form: String,

    /// The user's request, verbatim, language included.
    pub request: String,

    /// Why this case exists. Shown in the report.
    #[serde(default)]
    pub note: String,

    /// Language the model must write human-readable values in. English name,
    /// as `Language::english_name()` produces it. Defaults to English.
    #[serde(default)]
    pub language: Option<String>,

    /// `{{today}}` for this case, so date expectations stay stable.
    #[serde(default)]
    pub today: Option<String>,

    /// Values already on screen — set this to cover editing an open form
    /// ("añade una columna de IVA").
    #[serde(default)]
    pub current_values: Option<serde_yaml::Value>,

    /// Keys that must come back filled. The core assertion: a form the user
    /// cannot submit because the model skipped `name` is a failed fill.
    #[serde(default)]
    pub expect_present: Vec<String>,

    /// Keys that must NOT come back. This is where over-filling lives: a model
    /// that invents `scopeSenderDomains` from a request that named no sender
    /// silently narrows the lens to nothing.
    #[serde(default)]
    pub expect_absent: Vec<String>,

    /// Exact expectations per key. A string matches case-insensitively on
    /// substring; a list must match as a set, order-insensitively; a bool or
    /// number matches exactly.
    #[serde(default)]
    pub expect_values: BTreeMap<String, serde_yaml::Value>,

    /// Minimum number of rows in the form's object list (`columns`). The
    /// request names the data the user wants; one column per item is the job.
    #[serde(default)]
    pub expect_min_columns: Option<usize>,

    /// Concepts that must each appear in some column's `key` or `label`,
    /// matched case-insensitively on substring. "importe", "fecha",
    /// "proveedor" → three columns, whatever the model chose to call them.
    #[serde(default)]
    pub expect_columns_covering: Vec<String>,

    /// Column types that must appear, one per entry. `currency` on an amount
    /// column is the difference between a usable lens and a text field.
    #[serde(default)]
    pub expect_column_types: Vec<String>,

    /// Whether the fill must leave no required field missing. Default true —
    /// a case that deliberately under-specifies sets it to false.
    #[serde(default = "default_true")]
    pub expect_complete: bool,
}

fn default_true() -> bool {
    true
}

/// Load every `*.yaml` under `dir`, sorted by file name for stable report
/// ordering. Each file holds a list of cases.
pub fn load_form_cases(dir: &Path) -> EvalResult<Vec<FormCase>> {
    if !dir.is_dir() {
        return Err(EvalError::Config(format!(
            "form cases directory not found: {}",
            dir.display()
        )));
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| EvalError::Config(format!("cannot read {}: {e}", dir.display())))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "yaml" || e == "yml"))
        .collect();
    files.sort();

    let mut cases = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .map_err(|e| EvalError::Config(format!("cannot read {}: {e}", file.display())))?;
        let parsed: Vec<FormCase> = serde_yaml::from_str(&text)
            .map_err(|e| EvalError::Config(format!("invalid YAML in {}: {e}", file.display())))?;
        cases.extend(parsed);
    }
    if cases.is_empty() {
        return Err(EvalError::Config(format!("no form cases found in {}", dir.display())));
    }
    // A duplicate id makes `--case` ambiguous and the report misleading.
    let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    if ids.len() != before {
        return Err(EvalError::Config("duplicate form case id".into()));
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_case_parses_and_defaults_to_requiring_completeness() {
        let c: FormCase = serde_yaml::from_str("id: x\nform: lens.create\nrequest: crea una lens\n").expect("parses");
        assert_eq!(c.form, "lens.create");
        assert!(c.expect_complete, "completeness is the default expectation");
        assert!(c.expect_present.is_empty());
    }

    #[test]
    fn a_case_can_opt_out_of_completeness() {
        let c: FormCase =
            serde_yaml::from_str("id: x\nform: lens.create\nrequest: q\nexpect_complete: false\n").expect("parses");
        assert!(!c.expect_complete);
    }

    #[test]
    fn a_case_carries_its_open_form_values() {
        let c: FormCase =
            serde_yaml::from_str("id: x\nform: lens.create\nrequest: añade IVA\ncurrent_values:\n  name: Facturas\n")
                .expect("parses");
        assert!(c.current_values.is_some());
    }
}
