//! `emailops-cli eval` — a thin bridge onto the shared eval harness
//! (`crate::evals`) so an agent can re-run chat eval cases headlessly and read a
//! structured pass/fail report.
//!
//! This is the **heuristic** path only: it reuses `case_loader` + `harness` +
//! `metrics` (no LLM-as-judge, no HTML report, and — unlike `evals::runner` — it
//! does **not** pin provider preferences on the live DB). Each case runs in a
//! throwaway conversation that is deleted afterwards, so running it against a
//! real install leaves no chat-history residue.
//!
//! The subcommand is gated behind the `eval` cargo feature (which pulls in
//! `crate::evals`). Without it, [`run_eval`] returns a helpful error telling the
//! caller how to rebuild.

#[cfg(feature = "eval")]
use std::path::PathBuf;

use crate::models::error::Result;

use super::session::CliSession;

/// Run eval cases filtered by `case` (exact id) and/or `tier`. `cases_dir`
/// overrides the default case location. Emits one report envelope.
#[cfg(feature = "eval")]
pub async fn run_eval(
    session: &mut CliSession,
    case: Option<String>,
    tier: Option<String>,
    cases_dir: Option<PathBuf>,
) -> Result<()> {
    use serde::Serialize;

    use crate::evals::{case_loader, harness, metrics};
    use crate::models::error::AppError;

    use super::output;
    use super::OutputMode;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CheckReport {
        name: String,
        passed: bool,
        expected: String,
        actual: String,
        detail: String,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CaseReport {
        id: String,
        tier: String,
        passed: bool,
        checks_passed: usize,
        checks_total: usize,
        latency_ms: i64,
        checks: Vec<CheckReport>,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct EvalRunReport {
        passed: bool,
        cases_total: usize,
        cases_passed: usize,
        cases_failed: usize,
        cases: Vec<CaseReport>,
    }

    // eval-side failures (load/run/evaluate) are infrastructure errors from the
    // CLI's perspective — surface them as a typed AppError.
    fn map_eval_err(e: crate::evals::EvalError) -> AppError {
        AppError::AiError(format!("eval: {e}"))
    }

    let dir = resolve_cases_dir(cases_dir);
    let all_cases = case_loader::load_cases(&dir).map_err(map_eval_err)?;

    let selected: Vec<_> = all_cases
        .into_iter()
        .filter(|c| case.as_deref().map(|id| c.id == id).unwrap_or(true))
        .filter(|c| tier.as_deref().map(|t| c.tier == t).unwrap_or(true))
        .collect();

    if selected.is_empty() {
        return Err(AppError::NotFound(format!(
            "no eval cases matched (dir={}, case={:?}, tier={:?})",
            dir.display(),
            case,
            tier
        )));
    }

    let session_account = session.require_account()?;
    let mut case_reports: Vec<CaseReport> = Vec::with_capacity(selected.len());

    for c in &selected {
        let account = c.account.as_deref().unwrap_or(&session_account);
        let model = c.model.as_deref().unwrap_or(&session.model);

        let outcome = harness::run_case(session.db.clone(), account, model, c)
            .await
            .map_err(map_eval_err)?;
        let report = metrics::evaluate(c, &outcome).map_err(map_eval_err)?;

        // Keep the live DB clean: the eval conversation is throwaway.
        session.db.delete_chat_conversation(&outcome.conversation_id)?;

        case_reports.push(CaseReport {
            id: c.id.clone(),
            tier: c.tier.clone(),
            passed: report.all_passed(),
            checks_passed: report.passed_count(),
            checks_total: report.total(),
            latency_ms: outcome.wall_elapsed_ms,
            checks: report
                .checks
                .iter()
                .map(|chk| CheckReport {
                    name: chk.name.clone(),
                    passed: chk.passed,
                    expected: chk.expected.clone(),
                    actual: chk.actual.clone(),
                    detail: chk.detail.clone(),
                })
                .collect(),
        });
    }

    let cases_passed = case_reports.iter().filter(|c| c.passed).count();
    let run = EvalRunReport {
        passed: cases_passed == case_reports.len(),
        cases_total: case_reports.len(),
        cases_passed,
        cases_failed: case_reports.len() - cases_passed,
        cases: case_reports,
    };

    if session.mode == OutputMode::Json {
        return output::emit_ok(run);
    }

    println!(
        "eval: {}/{} cases passed{}",
        run.cases_passed,
        run.cases_total,
        if run.passed { "" } else { "  (FAIL)" }
    );
    for c in &run.cases {
        let mark = if c.passed { "ok  " } else { "FAIL" };
        println!(
            "  {mark} {} [{}] {}/{} checks ({} ms)",
            c.id, c.tier, c.checks_passed, c.checks_total, c.latency_ms
        );
        for chk in c.checks.iter().filter(|chk| !chk.passed) {
            println!(
                "       ✗ {}: expected {}, got {} — {}",
                chk.name, chk.expected, chk.actual, chk.detail
            );
        }
    }
    Ok(())
}

/// Resolve the eval-cases directory: an explicit flag wins; otherwise probe the
/// usual locations (so the command works whether invoked from the repo root or
/// from `src-tauri/`), preferring private cases when present.
#[cfg(feature = "eval")]
fn resolve_cases_dir(flag: Option<PathBuf>) -> PathBuf {
    use std::path::Path;
    if let Some(dir) = flag {
        return dir;
    }
    for candidate in [
        "private-evals/chat/cases",
        "evals/chat/cases",
        "src-tauri/evals/chat/cases",
    ] {
        let p = Path::new(candidate);
        if p.is_dir() {
            return p.to_path_buf();
        }
    }
    PathBuf::from("evals/chat/cases")
}

#[cfg(all(test, feature = "eval"))]
mod tests {
    use super::*;

    #[test]
    fn resolve_cases_dir_prefers_explicit_flag() {
        let p = PathBuf::from("/tmp/explicit-cases");
        assert_eq!(resolve_cases_dir(Some(p.clone())), p);
    }
}

/// Stub when the `eval` feature is off: the harness isn't compiled in, so tell
/// the caller exactly how to get it.
#[cfg(not(feature = "eval"))]
pub async fn run_eval(
    _session: &mut CliSession,
    _case: Option<String>,
    _tier: Option<String>,
    _cases_dir: Option<std::path::PathBuf>,
) -> Result<()> {
    Err(crate::models::error::AppError::InvalidInput(
        "the `eval` subcommand requires the 'eval' feature — rebuild with: \
         cargo run --no-default-features --features cli,eval --bin emailops-cli -- eval ..."
            .to_string(),
    ))
}
