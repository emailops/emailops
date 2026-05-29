// YAML case loader.
//
// Parses `*.yaml` files under `src-tauri/evals/chat/cases/` into typed
// `EvalCase` structs. The YAML format is documented inline in
// `user_queries.yaml`.

use std::path::Path;

use serde::Deserialize;

use crate::evals::{EvalError, EvalResult};
use crate::models::RouteMode;

/// Metrics the judge can run for a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    AnswerRelevancy,
    Faithfulness,
    ContextualRelevancy,
    ContextualRecall,
}

impl MetricKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MetricKind::AnswerRelevancy => "answer_relevancy",
            MetricKind::Faithfulness => "faithfulness",
            MetricKind::ContextualRelevancy => "contextual_relevancy",
            MetricKind::ContextualRecall => "contextual_recall",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub question: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_tier")]
    pub tier: String,
    #[serde(default)]
    pub model: Option<String>,

    /// Account override for this case — either the account id or the account's
    /// email address. When set, overrides the CLI --account flag and the
    /// auto-resolved default. Used so cases can target the mailbox where the
    /// test data actually lives.
    #[serde(default)]
    pub account: Option<String>,

    /// Expected router mode. Absence skips the route check.
    #[serde(default)]
    pub expected_route: Option<RouteMode>,

    /// Tool names that must appear in `trace.tool_calls` (in any order).
    #[serde(default)]
    pub expected_tools_called: Vec<String>,

    /// Case-insensitive substrings that must appear in the final assistant content.
    #[serde(default)]
    pub expected_answer_contains: Vec<String>,

    /// Regex pattern the auto-derived conversation title must match.
    #[serde(default)]
    pub expected_title_pattern: Option<String>,

    /// Golden reference answer used by the LLM judge (optional).
    #[serde(default)]
    pub expected_output: Option<String>,

    /// Metrics the judge should run for this case.
    #[serde(default)]
    pub metrics: Vec<MetricKind>,
}

fn default_category() -> String {
    "general".into()
}

fn default_tier() -> String {
    "smoke".into()
}

/// Load every `*.yaml` file in `dir` into a flat `Vec<EvalCase>`.
pub fn load_cases(dir: &Path) -> EvalResult<Vec<EvalCase>> {
    if !dir.exists() {
        return Err(EvalError::Config(format!(
            "cases directory does not exist: {}",
            dir.display()
        )));
    }

    let mut out: Vec<EvalCase> = Vec::new();
    let mut yaml_paths: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("yaml"))
        .collect();
    yaml_paths.sort();

    for path in yaml_paths {
        let text = std::fs::read_to_string(&path)?;
        let cases: Vec<EvalCase> =
            serde_yaml::from_str(&text).map_err(|e| EvalError::Config(format!("{}: {}", path.display(), e)))?;
        out.extend(cases);
    }

    // Sanity: unique ids.
    let mut seen = std::collections::HashSet::new();
    for c in &out {
        if !seen.insert(c.id.clone()) {
            return Err(EvalError::Config(format!("duplicate case id: {}", c.id)));
        }
    }

    Ok(out)
}
