// YAML schema for the query-planner harness.
//
// A case is a question plus what the plan behind it must (and must not) say.
// Nothing here runs a chat turn: the planner is one small completion, so its
// quality can be measured on its own, in seconds per case instead of minutes.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::evals::{EvalError, EvalResult};

/// One planner case.
#[derive(Debug, Clone, Deserialize)]
pub struct PlanCase {
    /// Stable id, used by `--case` and as the report anchor.
    pub id: String,

    /// The user question, verbatim (language included — the planner is
    /// expected to read any of them).
    pub question: String,

    /// Why this case exists. Shown in the report.
    #[serde(default)]
    pub note: String,

    /// Fields the plan must carry, as `field: expected`. String fields match
    /// case-insensitively on substring (`from: marisol` accepts
    /// `Marisol Vega <marisol@…>`); `limit` and booleans match exactly.
    #[serde(default)]
    pub expect: BTreeMap<String, serde_yaml::Value>,

    /// Fields the plan must NOT carry. This is where the planner's failure
    /// modes live: a date window on a question with no date, a classifier tag
    /// the question never named.
    #[serde(default)]
    pub absent: Vec<String>,

    /// Fields the case deliberately does not score — `order: newest` is the
    /// tool's own default, for instance. Anything the plan sets that is not in
    /// `expect`, `absent` or here fails the case: every field changes the
    /// search that runs, so a case has to account for all of them.
    #[serde(default)]
    pub ignore: Vec<String>,

    /// True when the right answer is "this is not a single email search" —
    /// the planner must defer to the tool loop.
    #[serde(default)]
    pub expect_defer: bool,

    /// True when the question is about EmailOps itself — the planner must
    /// answer `{"app_help": true}` so the turn skips mailbox retrieval and is
    /// answered from the bundled guides.
    #[serde(default)]
    pub expect_app_help: bool,

    /// With `expect_app_help`: the guide pages any of which the planner may
    /// name (`{"app_help": "<page>"}`). Empty accepts any page, or none.
    #[serde(default)]
    pub expect_help_page_any: Vec<String>,

    /// The app form the planner must route to (`{"form": "<id>"}`) — set for a
    /// question that asks to CREATE something the app has a form for. The
    /// counterpart of `expect_app_help`, which covers asking HOW to create it.
    #[serde(default)]
    pub expect_form: Option<String>,

    /// The form the user has open on screen while asking, if any (the
    /// `form/<id>` half of the chat panel's view context). Lets a case cover
    /// "añade una columna de IVA", which only means something with a form up.
    #[serde(default)]
    pub open_form: Option<String>,

    /// `{{today}}` for this case, so date expectations stay stable as the
    /// calendar moves. ISO `YYYY-MM-DD`; defaults to the runner's today.
    #[serde(default)]
    pub today: Option<String>,
}

/// Load every `*.yaml` under `dir`, newest-first by file name for stable
/// report ordering. Each file holds a list of cases.
pub fn load_plan_cases(dir: &Path) -> EvalResult<Vec<PlanCase>> {
    if !dir.is_dir() {
        return Err(EvalError::Config(format!(
            "planner cases directory not found: {}",
            dir.display()
        )));
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| EvalError::Config(format!("cannot read {}: {e}", dir.display())))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "yaml" || ext == "yml"))
        .collect();
    files.sort();

    let mut cases = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| EvalError::Config(format!("cannot read {}: {e}", path.display())))?;
        let parsed: Vec<PlanCase> = serde_yaml::from_str(&text)
            .map_err(|e| EvalError::Config(format!("invalid YAML in {}: {e}", path.display())))?;
        cases.extend(parsed);
    }
    if cases.is_empty() {
        return Err(EvalError::Config(format!("no planner cases in {}", dir.display())));
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_case_file_parses_into_expectations() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("cases.yaml"),
            r#"
- id: no_date_question
  question: "when did Marisol first write to me?"
  note: "the planner stamped today's date on a question with no date"
  expect:
    from: "marisol"
    order: "oldest"
  absent: [since, until]
- id: not_a_search
  question: "redacta una respuesta para Marisol"
  expect_defer: true
"#,
        )
        .expect("write cases");

        let cases = load_plan_cases(dir.path()).expect("load");

        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].id, "no_date_question");
        assert_eq!(cases[0].absent, vec!["since", "until"]);
        assert_eq!(cases[0].expect.len(), 2);
        assert!(cases[1].expect_defer);
    }

    #[test]
    fn a_missing_directory_is_a_config_error() {
        let err = load_plan_cases(Path::new("/nope/not/here")).unwrap_err();
        assert!(matches!(err, EvalError::Config(_)), "{err:?}");
    }
}
