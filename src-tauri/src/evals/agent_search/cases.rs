// YAML case loader for the agent-search eval.
//
// Format (see `src-tauri/evals/agent_search/cases.yaml`):
//
//   - id: proposals_sent
//     question: "emails en los que he enviado propuestas a clientes"
//     account: alex@northwindlabs.io      # account email or id
//     judge_criteria: |
//       An email is RELEVANT iff:
//         - it was sent BY the user (not received), AND
//         - it contains or transmits a commercial proposal / "propuesta" /
//           offer / quote to a client (paid work scope, pricing, deliverables).
//       Wedding planning, internal HR offers, generic newsletters do NOT count.
//     tags: [spanish, sent_by_user, proposals]

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::evals::{EvalError, EvalResult};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentSearchCase {
    pub id: String,
    pub question: String,
    /// Account override — accepts account id or email address.
    pub account: Option<String>,
    /// Free-form rubric the judge uses to decide relevance. Spelled out so
    /// "proposal" doesn't accidentally include irrelevant offers etc.
    pub judge_criteria: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub fn load_cases(dir: &Path) -> EvalResult<Vec<AgentSearchCase>> {
    if !dir.exists() {
        return Err(EvalError::Config(format!(
            "cases directory does not exist: {}",
            dir.display()
        )));
    }
    let mut yaml_paths: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("yaml"))
        .collect();
    yaml_paths.sort();

    let mut out: Vec<AgentSearchCase> = Vec::new();
    for path in yaml_paths {
        let text = std::fs::read_to_string(&path)?;
        let cases: Vec<AgentSearchCase> =
            serde_yaml::from_str(&text).map_err(|e| EvalError::Config(format!("{}: {}", path.display(), e)))?;
        out.extend(cases);
    }

    let mut seen = std::collections::HashSet::new();
    for c in &out {
        if !seen.insert(c.id.clone()) {
            return Err(EvalError::Config(format!("duplicate case id: {}", c.id)));
        }
    }
    Ok(out)
}
