// YAML schema for the tag-classification harness.
//
// A case is a synthetic email plus the tags a careful human would give it.
// Every axis lists the gold label first and then the alternatives that are
// also defensible, because "is this a request or a question?" has a real
// answer band and a harness that ignored it would report noise as regression.

use std::path::Path;

use serde::Deserialize;

use crate::evals::{EvalError, EvalResult};

/// The three urgency levels the classifier prompt fixes.
pub const URGENCIES: &[&str] = &["urgent", "normal", "low"];

/// One labelled email.
#[derive(Debug, Clone, Deserialize)]
pub struct TagCase {
    /// Stable id, used by `--case` and as the report anchor.
    pub id: String,

    /// `en` or `es` — the corpus covers both because the app classifies both
    /// and the prompt carries a "respond in <language>" clause.
    pub lang: String,

    pub from_name: String,
    /// Always a `.test` address: the corpus is synthetic by construction.
    pub from_email: String,
    pub subject: String,
    /// The body preview, as the classifier sees it (first 300 chars).
    pub snippet: String,

    pub expect: Expect,

    /// Free-form markers for slicing the report (`hard`, `injection`,
    /// `ambiguous`…).
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Accepted labels per axis, gold first.
#[derive(Debug, Clone, Deserialize)]
pub struct Expect {
    pub intent: Vec<String>,
    pub topic: Vec<String>,
    pub urgency: Vec<String>,
}

impl TagCase {
    /// The gold label for one axis.
    pub fn gold(labels: &[String]) -> &str {
        labels.first().map(String::as_str).unwrap_or_default()
    }
}

/// Load every `*.yaml` under `dir` (each file holds a list of cases), sorted
/// by file name for a stable report order, and validate the labels against
/// the taxonomy the classifier actually offers.
pub fn load_tag_cases(dir: &Path, intents: &[String], topics: &[String]) -> EvalResult<Vec<TagCase>> {
    if !dir.is_dir() {
        return Err(EvalError::Config(format!(
            "classification cases directory not found: {}",
            dir.display()
        )));
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| EvalError::Config(format!("cannot read {}: {e}", dir.display())))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "yaml" || ext == "yml"))
        .collect();
    files.sort();

    let mut cases: Vec<TagCase> = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file)?;
        let parsed: Vec<TagCase> =
            serde_yaml::from_str(&text).map_err(|e| EvalError::Config(format!("{}: {e}", file.display())))?;
        cases.extend(parsed);
    }

    if cases.is_empty() {
        return Err(EvalError::Config(format!(
            "no classification cases found in {}",
            dir.display()
        )));
    }

    validate(&cases, intents, topics)?;
    Ok(cases)
}

/// Reject a corpus that scores against labels the classifier can never
/// return — a typo in a case file would otherwise read as a model failure.
fn validate(cases: &[TagCase], intents: &[String], topics: &[String]) -> EvalResult<()> {
    let mut seen: Vec<&str> = Vec::with_capacity(cases.len());
    for case in cases {
        if seen.contains(&case.id.as_str()) {
            return Err(EvalError::Config(format!("duplicate case id `{}`", case.id)));
        }
        seen.push(&case.id);

        check_axis(&case.id, "intent", &case.expect.intent, intents)?;
        check_axis(&case.id, "topic", &case.expect.topic, topics)?;
        let urgencies: Vec<String> = URGENCIES.iter().map(|s| s.to_string()).collect();
        check_axis(&case.id, "urgency", &case.expect.urgency, &urgencies)?;
    }
    Ok(())
}

fn check_axis(id: &str, axis: &str, expected: &[String], allowed: &[String]) -> EvalResult<()> {
    if expected.is_empty() {
        return Err(EvalError::Config(format!("case `{id}`: expect.{axis} is empty")));
    }
    for label in expected {
        if !allowed.contains(label) {
            return Err(EvalError::Config(format!(
                "case `{id}`: expect.{axis} lists `{label}`, which is not in the configured taxonomy"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case_yaml(id: &str, intent: &str) -> String {
        format!(
            r#"- id: {id}
  lang: en
  from_name: Sam Rivers
  from_email: sam@northwind.test
  subject: "Invoice 42"
  snippet: "Could you send the invoice?"
  expect:
    intent: [{intent}]
    topic: [billing]
    urgency: [normal]
"#
        )
    }

    fn taxonomy() -> (Vec<String>, Vec<String>) {
        (
            vec!["request".to_string(), "question".to_string()],
            vec!["billing".to_string()],
        )
    }

    #[test]
    fn loads_and_validates_a_well_formed_corpus() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.yaml"), case_yaml("one", "request")).expect("write");
        let (intents, topics) = taxonomy();

        let cases = load_tag_cases(dir.path(), &intents, &topics).expect("load");

        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].id, "one");
        assert_eq!(TagCase::gold(&cases[0].expect.intent), "request");
    }

    #[test]
    fn rejects_a_label_outside_the_taxonomy() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.yaml"), case_yaml("one", "banana")).expect("write");
        let (intents, topics) = taxonomy();

        let err = load_tag_cases(dir.path(), &intents, &topics).expect_err("must reject");

        assert!(format!("{err}").contains("not in the configured taxonomy"), "{err}");
    }

    #[test]
    fn rejects_duplicate_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.yaml"), case_yaml("dup", "request")).expect("write");
        std::fs::write(dir.path().join("b.yaml"), case_yaml("dup", "question")).expect("write");
        let (intents, topics) = taxonomy();

        let err = load_tag_cases(dir.path(), &intents, &topics).expect_err("must reject");

        assert!(format!("{err}").contains("duplicate case id"), "{err}");
    }

    #[test]
    fn rejects_an_empty_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (intents, topics) = taxonomy();

        let err = load_tag_cases(dir.path(), &intents, &topics).expect_err("must reject");

        assert!(format!("{err}").contains("no classification cases"), "{err}");
    }
}
