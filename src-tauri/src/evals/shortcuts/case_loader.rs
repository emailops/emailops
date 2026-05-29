// YAML schema for the shortcut-variant harness.
//
// Each `*.yaml` file under `evals/chat/shortcuts/` defines one shortcut and
// its variants. The structural rubric is parsed both deterministically
// (metrics.rs) and passed as guidance to the judge (judge.rs).

use std::path::Path;

use serde::Deserialize;

use crate::evals::{EvalError, EvalResult};

/// One shortcut (e.g. "Resumen de hoy") plus N prompt variants to A/B/C test.
#[derive(Debug, Clone, Deserialize)]
pub struct ShortcutCase {
    /// Stable id — matches the id in ChatView.tsx so reports are easy to
    /// cross-reference with the production shortcut buttons.
    pub shortcut_id: String,

    /// Human-readable label (the button text). Shown in the report only.
    pub label: String,

    /// Account id OR email address. Resolved against the DB at runtime; lets
    /// different shortcuts target different mailboxes if needed.
    pub account: String,

    /// Ollama model for the chat turn.
    pub model: String,

    /// Deterministic rubric — decides CI pass/fail per variant.
    pub rubric: StructuralRubric,

    /// Ordered list of prompt variants. Each runs end-to-end in isolation.
    pub variants: Vec<ShortcutVariant>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShortcutVariant {
    /// Short slug (e.g. "v0_current", "v1_xml_terse"). Must be unique within
    /// the parent shortcut.
    pub id: String,

    /// Short human description shown in the report (what's being tested).
    #[serde(default)]
    pub description: String,

    /// The full user-message text sent to the chat pipeline.
    pub prompt: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StructuralRubric {
    /// Expected primary output language. Currently "es" or "en"; used by the
    /// deterministic language heuristic and by the judge's tone rubric.
    #[serde(default = "default_language")]
    pub language: String,

    /// If true, the final answer must contain a markdown table.
    #[serde(default)]
    pub must_contain_table: bool,

    /// Column headers that must appear (case-insensitive) in the table header row.
    #[serde(default)]
    pub required_columns: Vec<String>,

    /// Minimum number of data rows (excluding header and separator).
    #[serde(default)]
    pub min_rows: usize,

    /// If true, the answer must end with a paragraph of prose (not a table row).
    #[serde(default)]
    pub must_end_with_summary_paragraph: bool,

    /// If true, every row in the table must contain an inline `[n]` citation.
    #[serde(default)]
    pub require_row_citations: bool,
}

fn default_language() -> String {
    "es".into()
}

/// Load every `*.yaml` file in `dir` into a flat `Vec<ShortcutCase>`.
pub fn load_shortcut_cases(dir: &Path) -> EvalResult<Vec<ShortcutCase>> {
    if !dir.exists() {
        return Err(EvalError::Config(format!(
            "shortcuts directory does not exist: {}",
            dir.display()
        )));
    }

    let mut out: Vec<ShortcutCase> = Vec::new();
    let mut yaml_paths: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("yaml"))
        .collect();
    yaml_paths.sort();

    for path in yaml_paths {
        let text = std::fs::read_to_string(&path)?;
        // Each file is ONE shortcut (not a list) — this keeps diffs focused
        // when editing variants and prevents merge conflicts between shortcuts.
        let case: ShortcutCase =
            serde_yaml::from_str(&text).map_err(|e| EvalError::Config(format!("{}: {}", path.display(), e)))?;

        // Sanity: unique variant ids within a shortcut.
        let mut seen = std::collections::HashSet::new();
        for v in &case.variants {
            if !seen.insert(v.id.clone()) {
                return Err(EvalError::Config(format!(
                    "{}: duplicate variant id '{}' in shortcut '{}'",
                    path.display(),
                    v.id,
                    case.shortcut_id
                )));
            }
        }
        out.push(case);
    }

    // Sanity: unique shortcut ids across files.
    let mut seen = std::collections::HashSet::new();
    for c in &out {
        if !seen.insert(c.shortcut_id.clone()) {
            return Err(EvalError::Config(format!("duplicate shortcut_id: {}", c.shortcut_id)));
        }
    }

    Ok(out)
}
