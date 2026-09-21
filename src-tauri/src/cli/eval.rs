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
use crate::models::ChatTrace;
#[cfg(feature = "eval")]
use serde::Serialize;
#[cfg(feature = "eval")]
use std::path::PathBuf;

use crate::models::error::Result;

use super::session::CliSession;

#[cfg(feature = "eval")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckReport {
    pub name: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

/// One case in the `eval --json` envelope. `question`, `answer` and the
/// engine `trace` travel with every case so a failing check can be debugged
/// from the report alone, without re-running the model.
#[cfg(feature = "eval")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaseReport {
    pub id: String,
    pub tier: String,
    /// What the question exercises (thread_summary, pending_actions…), shown
    /// next to the case in the verification report.
    pub category: String,
    pub passed: bool,
    pub checks_passed: usize,
    pub checks_total: usize,
    pub latency_ms: i64,
    pub question: String,
    pub answer: String,
    pub trace: Option<ChatTrace>,
    pub checks: Vec<CheckReport>,
    /// Golden reference the judge compared against, when the case has one.
    pub expected_output: Option<String>,
    /// LLM-judge verdict, present only when `--judge` was given.
    pub judge: Option<JudgeReport>,
}

#[cfg(feature = "eval")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JudgeReport {
    pub model: String,
    pub passed: bool,
    pub threshold: f64,
    pub metrics: Vec<String>,
    pub scores: crate::evals::judge::JudgeScores,
}

/// Minimum score, per requested metric, for the judge to accept a case.
#[cfg(feature = "eval")]
pub(crate) const JUDGE_THRESHOLD: f64 = 0.7;

#[cfg(feature = "eval")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EvalRunReport {
    pub passed: bool,
    pub cases_total: usize,
    pub cases_passed: usize,
    pub cases_failed: usize,
    pub cases: Vec<CaseReport>,
}

#[cfg(feature = "eval")]
/// One failed row for a case the harness could not run (thread or account
/// missing from this DB, provider error): the suite goes on and the report
/// carries the reason instead of dying on the first broken case.
pub(crate) fn failed_case_report(case: &crate::evals::case_loader::EvalCase, error: &str) -> CaseReport {
    CaseReport {
        id: case.id.clone(),
        tier: case.tier.clone(),
        category: case.category.clone(),
        passed: false,
        checks_passed: 0,
        checks_total: 1,
        latency_ms: 0,
        question: case.question.clone(),
        answer: String::new(),
        trace: None,
        expected_output: case.expected_output.clone(),
        judge: None,
        checks: vec![CheckReport {
            name: "run".into(),
            passed: false,
            expected: "the case runs to an answer".into(),
            actual: "the harness could not run it".into(),
            detail: error.to_string(),
        }],
    }
}

/// Run eval cases filtered by `case` (exact id) and/or `tier`. `cases_dir`
/// overrides the default case location. Emits one report envelope.
#[cfg(feature = "eval")]
pub async fn run_eval(
    session: &mut CliSession,
    case: Option<String>,
    tier: Option<String>,
    cases_dir: Option<PathBuf>,
    judge: bool,
    judge_model: Option<String>,
) -> Result<()> {
    use crate::evals::{case_loader, harness, metrics};
    use crate::models::error::AppError;

    use super::output;
    use super::OutputMode;

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

    // Fail before the first case rather than once per case at turn time: a
    // missing GGUF used to surface as N identical "model file not found"
    // failures, attributed to the feature each case belonged to.
    let mut models: Vec<&str> = selected
        .iter()
        .map(|c| c.model.as_deref().unwrap_or(&session.model))
        .collect();
    models.sort_unstable();
    models.dedup();
    crate::evals::shared::preflight_models(&session.db, models).map_err(map_eval_err)?;

    let session_account = session.require_account()?;
    let mut case_reports: Vec<CaseReport> = Vec::with_capacity(selected.len());

    // Build the guides index (text + vectors) once up front, as the app's
    // prewarm does, so the app-help cases see the same corpus a user would.
    {
        let provider = crate::services::ai::AiService::load_provider_with_model(&session.db, Some(&session.model))?;
        crate::services::help_docs::ensure_index(&session.db, provider.as_ref()).await?;
    }

    // The judge runs on the app's own provider (embedded llama.cpp by default);
    // with the same model as the chat it shares the loaded weights.
    let judge_provider = if judge {
        let model = judge_model.clone().unwrap_or_else(|| session.model.clone());
        let provider_name = session
            .db
            .get_preference("ai_provider")?
            .unwrap_or_else(|| "llamacpp".to_string());
        Some((
            model.clone(),
            crate::services::ai::AiService::build_provider(&session.db, &provider_name, &model)?,
        ))
    } else {
        None
    };

    for c in &selected {
        let account = match resolve_case_account(&session.db, c.account.as_deref(), &session_account) {
            Ok(account) => account,
            Err(e) => {
                case_reports.push(failed_case_report(c, &e.to_string()));
                continue;
            }
        };
        let model = c.model.as_deref().unwrap_or(&session.model);

        let outcome = match harness::run_case(session.db.clone(), &account, model, c).await {
            Ok(outcome) => outcome,
            Err(e) => {
                case_reports.push(failed_case_report(c, &map_eval_err(e).to_string()));
                continue;
            }
        };
        let report = metrics::evaluate(c, &outcome).map_err(map_eval_err)?;

        let judge_report = match &judge_provider {
            Some((model, provider)) => {
                let scores = crate::evals::judge::score_with_provider(provider.as_ref(), c, &outcome).await;
                Some(JudgeReport {
                    model: model.clone(),
                    passed: crate::evals::judge::judge_passes(&scores, c, JUDGE_THRESHOLD),
                    threshold: JUDGE_THRESHOLD,
                    metrics: c.metrics.iter().map(|m| m.as_str().to_string()).collect(),
                    scores,
                })
            }
            None => None,
        };

        // Keep the live DB clean: the eval conversation is throwaway.
        session.db.delete_chat_conversation(&outcome.conversation_id)?;

        case_reports.push(CaseReport {
            id: c.id.clone(),
            tier: c.tier.clone(),
            category: c.category.clone(),
            passed: report.all_passed() && judge_report.as_ref().is_none_or(|j| j.passed),
            checks_passed: report.passed_count(),
            checks_total: report.total(),
            latency_ms: outcome.wall_elapsed_ms,
            question: c.question.clone(),
            answer: outcome.assistant_content.clone(),
            trace: outcome.assistant_trace.clone(),
            expected_output: c.expected_output.clone(),
            judge: judge_report,
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

/// Resolve the account a case should run against: the case's `account:`
/// override (an account id OR email — YAML authors use emails) resolved to a
/// real account id, falling back to the session account when absent.
///
/// Without resolution an email override flows straight into
/// `create_chat_conversation`, whose FOREIGN KEY on `accounts.id` rejects it.
#[cfg(feature = "eval")]
fn resolve_case_account(
    db: &std::sync::Arc<crate::db::Database>,
    case_account: Option<&str>,
    session_account: &str,
) -> Result<String> {
    match case_account {
        Some(hint) => match super::session::resolve_account(db, Some(hint))? {
            Some(id) => Ok(id),
            // resolve_account errs on unknown hints; None is unreachable for
            // Some(hint), but map it defensively instead of unwrapping.
            None => Ok(session_account.to_string()),
        },
        None => Ok(session_account.to_string()),
    }
}

#[cfg(test)]
#[cfg(feature = "eval")]
mod tests {
    use super::{failed_case_report, CaseReport, CheckReport};
    use crate::evals::case_loader::EvalCase;

    /// A case the harness cannot even start (thread id that no longer exists,
    /// account not in this DB) is one failed row, not the end of the suite:
    /// the other cases still run and the report says why this one did not.
    #[test]
    fn a_case_that_cannot_run_is_reported_as_failed_not_fatal() {
        let case: EvalCase =
            serde_yaml::from_str("id: broken\nquestion: q\ncategory: c\ntier: smoke\n").expect("minimal case");
        let report = failed_case_report(&case, "Not found: thread REPLACE_ME");
        assert!(!report.passed);
        assert_eq!(report.id, "broken");
        assert_eq!((report.checks_passed, report.checks_total), (0, 1));
        assert_eq!(report.checks[0].name, "run");
        assert!(report.checks[0].detail.contains("REPLACE_ME"));
        assert!(report.answer.is_empty());
        assert!(report.judge.is_none());
    }

    /// The report header shows what each chat question exercises
    /// (thread_summary, pending_actions…), so every row carries its case's
    /// category — a case that could not run included.
    #[test]
    fn case_report_carries_the_case_category() {
        let case: EvalCase =
            serde_yaml::from_str("id: c1\nquestion: q\ncategory: thread_summary\ntier: smoke\n").expect("minimal case");
        let report = failed_case_report(&case, "boom");
        let json = serde_json::to_value(&report).expect("serializes");
        assert_eq!(json["category"], "thread_summary");
    }

    #[test]
    fn case_report_carries_question_answer_and_trace_for_debugging() {
        let report = CaseReport {
            id: "demo_case".into(),
            tier: "smoke".into(),
            category: "thread_summary".into(),
            passed: false,
            checks_passed: 0,
            checks_total: 1,
            latency_ms: 12,
            question: "¿Qué dijo Marisol?".into(),
            answer: "Marisol pidió el informe.".into(),
            trace: None,
            expected_output: Some("Kwame Boateng preguntó por Ollama.".into()),
            judge: None,
            checks: vec![CheckReport {
                name: "contains".into(),
                passed: false,
                expected: "factura".into(),
                actual: "".into(),
                detail: "missing".into(),
            }],
        };
        let json = serde_json::to_value(&report).expect("serializes");
        assert_eq!(json["question"], "¿Qué dijo Marisol?");
        assert_eq!(json["answer"], "Marisol pidió el informe.");
        assert!(
            json.get("trace").is_some(),
            "trace key is present even when the engine recorded none"
        );
        assert_eq!(json["checks"][0]["name"], "contains");
        assert_eq!(json["expectedOutput"], "Kwame Boateng preguntó por Ollama.");
        assert!(json["judge"].is_null(), "no judge unless --judge was given");
    }

    use super::*;
    use crate::db::Database;
    use std::sync::Arc;

    fn seed_account(db: &Arc<Database>, id: &str, email: &str) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES (?1, 'gmail', ?2, ?2, 0, 0, 1)",
                rusqlite::params![id, email],
            )
            .expect("seed account");
    }

    #[test]
    fn resolve_cases_dir_prefers_explicit_flag() {
        let p = PathBuf::from("/tmp/explicit-cases");
        assert_eq!(resolve_cases_dir(Some(p.clone())), p);
    }

    #[test]
    fn case_account_email_override_resolves_to_id() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "acct-1", "alex@northwindlabs.io");
        let got = resolve_case_account(&db, Some("alex@northwindlabs.io"), "session-acct").expect("resolve");
        assert_eq!(got, "acct-1");
    }

    #[test]
    fn case_account_id_override_passes_through() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "acct-1", "alex@northwindlabs.io");
        let got = resolve_case_account(&db, Some("acct-1"), "session-acct").expect("resolve");
        assert_eq!(got, "acct-1");
    }

    #[test]
    fn no_override_falls_back_to_session_account() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let got = resolve_case_account(&db, None, "session-acct").expect("resolve");
        assert_eq!(got, "session-acct");
    }

    #[test]
    fn unknown_override_is_an_error() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "acct-1", "alex@northwindlabs.io");
        assert!(resolve_case_account(&db, Some("ghost@nowhere.io"), "session-acct").is_err());
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
    _judge: bool,
    _judge_model: Option<String>,
) -> Result<()> {
    Err(crate::models::error::AppError::InvalidInput(
        "the `eval` subcommand requires the 'eval' feature — rebuild with: \
         cargo run --no-default-features --features cli,eval --bin emailops-cli -- eval ..."
            .to_string(),
    ))
}
