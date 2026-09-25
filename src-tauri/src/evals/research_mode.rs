// Research answer-form classifier harness.
//
// Runs the live `chat.research_mode` prompt — the same template, reply shape
// and provider the app uses — on each case and checks the form it picks. One
// short completion per case, so a prompt or model change is checked in
// seconds instead of a research run per question.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::{EvalError, EvalResult};
use crate::models::ReportMode;

/// One case: a question and the form its answer must take.
#[derive(Debug, Clone, Deserialize)]
pub struct ModeCase {
    pub id: String,
    pub question: String,
    pub expect: ReportMode,
}

#[derive(Debug, Clone)]
pub struct ModeRunnerConfig {
    pub only_case: Option<String>,
    pub model_override: Option<String>,
    pub cases_dir: PathBuf,
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
    pub json_stdout: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeCaseResult {
    pub id: String,
    pub expected: ReportMode,
    pub got: ReportMode,
    pub passed: bool,
    pub latency_ms: u128,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeSummary {
    pub model: String,
    pub total: usize,
    pub passed: usize,
    /// Questions that need a report but were given a bare list or count —
    /// the mistake that costs the user the answer.
    pub report_lost: usize,
    pub cases: Vec<ModeCaseResult>,
}

/// Every case in the `*.yaml` files of `dir`.
pub fn load_mode_cases(dir: &Path) -> EvalResult<Vec<ModeCase>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
        .collect();
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file)?;
        let parsed: Vec<ModeCase> =
            serde_yaml::from_str(&text).map_err(|e| EvalError::Config(format!("{}: {e}", file.display())))?;
        cases.extend(parsed);
    }
    Ok(cases)
}

pub async fn run(cfg: ModeRunnerConfig) -> EvalResult<ModeSummary> {
    let mut cases = load_mode_cases(&cfg.cases_dir)?;
    if let Some(id) = &cfg.only_case {
        cases.retain(|c| &c.id == id);
        if cases.is_empty() {
            return Err(EvalError::Config(format!("no research-mode case with id `{id}`")));
        }
    }
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "research-mode")?;
    let db = std::sync::Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);
    crate::evals::shared::apply_eval_model_override_from_env(&db)?;
    if let Some(model) = &cfg.model_override {
        db.set_preference("ai_model", model)?;
    }
    let provider = crate::services::ai::AiService::load_provider(&db)
        .map_err(|e| EvalError::Config(format!("no AI provider available: {e}")))?;
    let model = db.get_preference("ai_model")?.unwrap_or_default();

    let mut results = Vec::new();
    for case in cases {
        let started = Instant::now();
        let (got, _) = crate::services::chat::research::classify_question(&db, provider.as_ref(), &case.question).await;
        let passed = got == case.expect;
        if !cfg.json_stdout {
            println!(
                "[mode-eval] {} {} — expected {:?}, got {got:?}",
                if passed { "OK  " } else { "FAIL" },
                case.id,
                case.expect
            );
        }
        results.push(ModeCaseResult {
            id: case.id,
            expected: case.expect,
            got,
            passed,
            latency_ms: started.elapsed().as_millis(),
        });
    }
    let summary = ModeSummary {
        model,
        total: results.len(),
        passed: results.iter().filter(|r| r.passed).count(),
        report_lost: results
            .iter()
            .filter(|r| r.expected == ReportMode::Analysis && r.got != ReportMode::Analysis)
            .count(),
        cases: results,
    };
    if cfg.json_stdout {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "[mode-eval] {}/{} passed · {} report(s) lost to a bare list · model {}",
            summary.passed, summary.total, summary.report_lost, summary.model
        );
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_cases_load_and_cover_every_mode() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("evals/chat/research_mode");
        let cases = load_mode_cases(&dir).expect("cases load");
        for mode in [ReportMode::List, ReportMode::Count, ReportMode::Analysis] {
            assert!(cases.iter().any(|c| c.expect == mode), "no {mode:?} case");
        }
        let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), cases.len(), "case ids are unique");
    }
}
