// YAML schema for the Lens extraction harness.
//
// A case is one synthetic email, the built-in template that should read it,
// and what the resulting row must contain.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::evals::{EvalError, EvalResult};

/// One Lens extraction case.
#[derive(Debug, Clone, Deserialize)]
pub struct LensCase {
    /// Stable id, used by `--case` and as the report anchor.
    pub id: String,

    /// Built-in template key (`services::lenses::templates`).
    pub template: String,

    /// Why this case exists. Shown in the report.
    #[serde(default)]
    pub note: String,

    pub email: CaseEmail,

    /// Expected value per column. A string matches case-insensitively on
    /// substring (a summary need not match prose exactly); `null` means the
    /// column must come back empty; a bool or number matches exactly.
    /// Columns not listed are not checked.
    #[serde(default)]
    pub expect: BTreeMap<String, serde_yaml::Value>,

    /// Whether the template's default scope must pick this email up. Omitted
    /// means "not checked".
    #[serde(default)]
    pub expect_in_scope: Option<bool>,
}

/// The synthetic email, as a provider would have delivered it.
#[derive(Debug, Clone, Deserialize)]
pub struct CaseEmail {
    pub from_name: String,
    pub from_email: String,
    pub subject: String,
    pub body: String,
}

/// Load every `*.yaml` under `dir`, sorted by file name for stable report
/// ordering. Each file holds a list of cases.
pub fn load_lens_cases(dir: &Path) -> EvalResult<Vec<LensCase>> {
    if !dir.is_dir() {
        return Err(EvalError::Config(format!(
            "lens cases directory not found: {}",
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
        let parsed: Vec<LensCase> = serde_yaml::from_str(&text)
            .map_err(|e| EvalError::Config(format!("invalid YAML in {}: {e}", file.display())))?;
        cases.extend(parsed);
    }
    if cases.is_empty() {
        return Err(EvalError::Config(format!("no lens cases found in {}", dir.display())));
    }
    let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    if ids.len() != before {
        return Err(EvalError::Config("duplicate lens case id".into()));
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_case_parses_with_null_expectations() {
        let c: LensCase = serde_yaml::from_str(
            "id: x\ntemplate: contact_form_leads\nemail:\n  from_name: Site\n  from_email: noreply@site.example\n  subject: Hi\n  body: text\nexpect:\n  contact_email: ana@client.example\n  company: null\nexpect_in_scope: true\n",
        )
        .expect("parses");
        assert_eq!(c.template, "contact_form_leads");
        assert!(c.expect["company"].is_null());
        assert_eq!(c.expect_in_scope, Some(true));
    }

    #[test]
    fn shipped_cases_name_real_templates_and_columns() {
        // A typo in a case would otherwise surface as a confusing failure in
        // the middle of `make verify`.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evals/lenses");
        for case in load_lens_cases(&dir).expect("cases load") {
            let tpl = crate::services::lenses::templates::get(&case.template)
                .unwrap_or_else(|| panic!("{}: unknown template {}", case.id, case.template));
            for field in case.expect.keys() {
                assert!(
                    tpl.schema.columns.iter().any(|c| &c.key == field),
                    "{}: {} is not a column of {}",
                    case.id,
                    field,
                    case.template
                );
            }
        }
    }

    #[test]
    fn the_scope_check_is_optional() {
        let c: LensCase = serde_yaml::from_str(
            "id: x\ntemplate: t\nemail:\n  from_name: a\n  from_email: a@b.example\n  subject: s\n  body: b\n",
        )
        .expect("parses");
        assert_eq!(c.expect_in_scope, None);
        assert!(c.expect.is_empty());
    }
}
