/// Standardised machine-readable JSON report for all eval runs.
///
/// Why a shared schema
/// -------------------
/// Every eval binary previously wrote its own ad-hoc output format (HTML only,
/// or CSV, or nothing). Dashboards that aggregate runs across eval types
/// (draft, chat, lens-extract, …) need one common schema to parse.
///
/// Schema
/// ------
/// ```json
/// {
///   "run_id":   "draft_eval_20240520_123456",
///   "eval_name": "draft_eval",
///   "model":    "gemma-4-e2b-it-q4_k_m",
///   "timestamp": "2024-05-20T12:34:56Z",
///   "total":     20,
///   "succeeded": 16,
///   "failed":     4,
///   "judge_scores": { "answer_relevancy": 0.82, "faithfulness": null },
///   "per_item_results": [
///     { "id": "case-1", "passed": true, "score": 0.9, "detail": "…" },
///     …
///   ]
/// }
/// ```
///
/// Migration note
/// --------------
/// Each eval bin calls `JsonRunReport::write()` after its run loop, writing a
/// sibling `.json` file next to the HTML report. The migration path is:
///   1. Construct `JsonRunReport` from whatever the runner produces.
///   2. Call `report.write(&out_dir)` — file name matches the HTML report.
///   3. Dashboards scrape `reports/evaluations/**/*.json`.
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::EvalResult;

/// Top-level machine-readable report for one eval run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRunReport {
    /// Globally unique ID for this run. Use for deduplication in dashboards.
    pub run_id: String,
    /// Short stable name for the eval (e.g. "draft_eval", "lens_extract_eval").
    pub eval_name: String,
    /// Model used for generation (not the judge model).
    pub model: String,
    /// ISO-8601 UTC timestamp.
    pub timestamp: String,
    /// Total number of test items.
    pub total: usize,
    /// Items that passed all heuristic checks.
    pub succeeded: usize,
    /// Items that failed at least one heuristic check.
    pub failed: usize,
    /// Averaged LLM-judge scores. `None` for a dimension means no judge data.
    pub judge_scores: JudgeScoresSummary,
    /// Per-item results.
    pub per_item_results: Vec<ItemResult>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JudgeScoresSummary {
    pub answer_relevancy: Option<f64>,
    pub faithfulness: Option<f64>,
    pub contextual_relevancy: Option<f64>,
    pub contextual_recall: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemResult {
    /// Case ID (matches the `id` field in YAML case files, or a derived label).
    pub id: String,
    /// Did the item pass all heuristic checks?
    pub passed: bool,
    /// Optional aggregate score \[0.0, 1.0\]. None when no score is available.
    pub score: Option<f64>,
    /// Human-readable summary (heuristic failures, judge rationale, etc.).
    pub detail: String,
}

impl JsonRunReport {
    /// Create a new report, generating a run_id from `eval_name` + timestamp.
    pub fn new(eval_name: impl Into<String>, model: impl Into<String>) -> Self {
        let eval_name = eval_name.into();
        let now = Utc::now();
        let stamp = now.format("%Y%m%d_%H%M%S");
        let run_id = format!("{eval_name}_{stamp}_{}", &Uuid::new_v4().to_string()[..8]);
        Self {
            run_id,
            eval_name,
            model: model.into(),
            timestamp: now.to_rfc3339(),
            total: 0,
            succeeded: 0,
            failed: 0,
            judge_scores: JudgeScoresSummary::default(),
            per_item_results: Vec::new(),
        }
    }

    /// Add one item result.
    pub fn push(&mut self, result: ItemResult) {
        self.total += 1;
        if result.passed {
            self.succeeded += 1;
        } else {
            self.failed += 1;
        }
        self.per_item_results.push(result);
    }

    /// Pass rate in \[0.0, 1.0\].
    pub fn pass_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.succeeded as f64 / self.total as f64
    }

    /// Write `{out_dir}/{run_id}.json`. Returns the path written.
    pub fn write(&self, out_dir: &Path) -> EvalResult<PathBuf> {
        std::fs::create_dir_all(out_dir)?;
        let path = out_dir.join(format!("{}.json", self.run_id));
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a previously-written report from disk.
    pub fn load(path: &Path) -> EvalResult<Self> {
        let bytes = std::fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn new_report_has_zero_counts() {
        let r = JsonRunReport::new("test_eval", "model-x");
        assert_eq!(r.total, 0);
        assert_eq!(r.succeeded, 0);
        assert_eq!(r.failed, 0);
        assert!(r.run_id.starts_with("test_eval_"));
    }

    #[test]
    fn push_updates_counts() {
        let mut r = JsonRunReport::new("e", "m");
        r.push(ItemResult {
            id: "a".into(),
            passed: true,
            score: Some(0.9),
            detail: String::new(),
        });
        r.push(ItemResult {
            id: "b".into(),
            passed: false,
            score: Some(0.2),
            detail: "fail".into(),
        });
        assert_eq!(r.total, 2);
        assert_eq!(r.succeeded, 1);
        assert_eq!(r.failed, 1);
        assert!((r.pass_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn write_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let mut r = JsonRunReport::new("roundtrip_eval", "test-model");
        r.push(ItemResult {
            id: "x".into(),
            passed: true,
            score: None,
            detail: String::new(),
        });

        let path = r.write(dir.path()).unwrap();
        assert!(path.exists());

        let loaded = JsonRunReport::load(&path).unwrap();
        assert_eq!(loaded.run_id, r.run_id);
        assert_eq!(loaded.total, 1);
        assert_eq!(loaded.succeeded, 1);
    }
}
