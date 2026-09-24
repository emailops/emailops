// Orchestrates the extraction eval:
//   1. Copies/opens the benchmark SQLite DB. The extractor's eval entry point
//      does no writes; we never call mark_memory_extracted.
//   2. Samples N emails from the target account + category='primary'.
//   3. Runs `services::memory::extractor::extract_for_eval` against an
//      AiService built around the requested embedded model.
//   4. (Optional) calls the OpenRouter judge for each case.
//   5. Renders a 3-column HTML report.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use rusqlite::params;

use crate::db::Database;
use crate::evals::db_source::{prepare_eval_db, EvalDbMode};
use crate::evals::extraction::judge::{EmailSummary, ExtractionJudge};
use crate::evals::extraction::report::{render_report, ReportCase};
use crate::evals::extraction::ExtractionKind;
use crate::evals::json_report::{ItemResult, JsonRunReport};
use crate::evals::{EvalError, EvalResult};
use crate::services::ai::AiService;
use crate::services::memory::extractor;

#[derive(Debug, Clone)]
pub struct ExtractionRunnerConfig {
    pub kind: ExtractionKind,
    pub account_hint: String, // email or account id
    pub limit: usize,
    pub model: String,         // llamacpp model id (e.g. "gemma-4-e2b-it-q4_k_m")
    pub provider_name: String, // typically "llamacpp"
    pub no_judge: bool,
    pub yes: bool,
    pub prod_db_path: PathBuf,
    pub db_mode: EvalDbMode,
    pub out_dir: PathBuf,
    /// Sample only replies (emails answering an earlier message of their thread).
    pub replies_only: bool,
}

pub async fn run(mut cfg: ExtractionRunnerConfig) -> EvalResult<PathBuf> {
    // 1. Judge env gate
    let api_key = std::env::var("OPENROUTER_API_KEY").ok();
    let judge_model_env = std::env::var("OPENROUTER_JUDGE_MODEL").ok();
    let judge_enabled = !cfg.no_judge;
    if judge_enabled {
        if api_key.is_none() {
            return Err(EvalError::Config(
                "OPENROUTER_API_KEY is not set. Export it or pass --no-judge.".into(),
            ));
        }
        if !cfg.yes {
            return Err(EvalError::Aborted(
                "judge requires --yes (sends email excerpts to OpenRouter) or --no-judge".into(),
            ));
        }
        eprintln!(
            "[extract-eval] judge ENABLED — model={}",
            judge_model_env
                .clone()
                .unwrap_or_else(|| "anthropic/claude-sonnet-4.5".into())
        );
    } else {
        eprintln!("[extract-eval] judge DISABLED (--no-judge) — extraction only.");
    }

    // 2. Prepare DB
    let prepared_db = prepare_eval_db(&cfg.prod_db_path, cfg.db_mode, "extraction")?;
    let db = Arc::new(Database::new(prepared_db.db_dir().to_path_buf())?);

    // Env-var override (set by `make eval-all MODEL=…`). When present we
    // write the override to the temp DB prefs *and* mirror it into cfg so the
    // provider built below uses the requested model rather than the
    // example-supplied default.
    if let Some((provider, model)) = crate::evals::shared::apply_eval_model_override_from_env(&db)? {
        cfg.provider_name = provider;
        cfg.model = model;
    }

    // 3. Resolve account
    let accounts = db.list_accounts()?;
    let hint = cfg.account_hint.trim();
    let account = accounts
        .iter()
        .find(|a| a.id.eq_ignore_ascii_case(hint) || a.email.eq_ignore_ascii_case(hint))
        .ok_or_else(|| EvalError::Config(format!("account '{}' not found in DB", hint)))?;
    eprintln!(
        "[extract-eval] account = {} ({}), kind={}, limit={}",
        account.email, account.id, cfg.kind, cfg.limit
    );

    // 4. Sample N primary-category email ids, honouring the user's persisted
    //    config so the eval reflects what prod would actually do (backfill
    //    window + tag exclusions). The backfill window now lives on TaskConfig
    //    (MemoryConfig has no time window of its own); sender/tag exclusions
    //    used for skipping below still come from MemoryConfig — those filters
    //    seed from the legacy shared keys so they match TaskConfig in practice.
    let mem_cfg = crate::services::memory::config::get_config(&db)?;
    let task_cfg = crate::services::tasks::config::get_config(&db)?;
    let min_ts = task_cfg.backfill_min_timestamp(chrono::Utc::now().timestamp());
    let email_ids = sample_primary_email_ids(&db, &account.id, cfg.limit, min_ts, cfg.replies_only)?;
    if email_ids.is_empty() {
        return Err(EvalError::Config(format!(
            "no primary-category emails found for {}",
            account.email
        )));
    }
    eprintln!(
        "[extract-eval] sampled {} email(s){}",
        email_ids.len(),
        match min_ts {
            Some(ts) => format!(" (since {})", ts),
            None => String::new(),
        }
    );

    // 5. Build provider + AiService
    let provider = AiService::build_provider(&db, &cfg.provider_name, &cfg.model)?;
    let ai = AiService::with_provider(db.clone(), provider);

    // 6. Judge
    let judge = if judge_enabled {
        Some(ExtractionJudge::new(
            api_key.expect("api_key is Some when judge_enabled"),
            judge_model_env,
        ))
    } else {
        None
    };

    // 7. Run cases
    let mut cases: Vec<ReportCase> = Vec::new();
    for (i, email_id) in email_ids.iter().enumerate() {
        eprintln!("[extract-eval]   [{}/{}] {}", i + 1, email_ids.len(), email_id);
        match run_case(&db, &ai, judge.as_ref(), cfg.kind, email_id, &mem_cfg).await {
            Ok(c) => cases.push(c),
            Err(e) => {
                eprintln!("[extract-eval]     ERROR: {}", e);
                cases.push(ReportCase::error(email_id, format!("{}", e)));
            }
        }
    }

    // 8. Render HTML report
    let judge_model_for_report = judge
        .as_ref()
        .map(|j| j.model_name().to_string())
        .unwrap_or_else(|| "(judge disabled)".into());
    let path = render_report(
        &cfg.out_dir,
        &cases,
        cfg.kind,
        &account.email,
        &cfg.model,
        judge_enabled,
        &judge_model_for_report,
    )?;
    eprintln!("[extract-eval] report → {}", path.display());

    // 9. Write standardised JSON report alongside the HTML.
    let eval_name = format!("{}_eval", cfg.kind);
    let mut json_report = JsonRunReport::new(&eval_name, &cfg.model);
    for c in &cases {
        let (passed, detail) = match &c.status {
            crate::evals::extraction::report::CaseStatus::Ok => {
                let verdict_pass = c
                    .verdict
                    .as_ref()
                    .map(|v| v.verdict.as_deref().unwrap_or("").eq_ignore_ascii_case("pass"))
                    .unwrap_or(true); // no judge → heuristic pass
                (verdict_pass, String::new())
            }
            crate::evals::extraction::report::CaseStatus::Skipped(reason) => (false, format!("skipped: {reason}")),
            crate::evals::extraction::report::CaseStatus::Error => {
                (false, c.error.clone().unwrap_or_else(|| "error".into()))
            }
        };
        json_report.push(ItemResult {
            id: c.email_id.clone(),
            passed,
            score: None,
            detail,
        });
    }
    match json_report.write(&cfg.out_dir) {
        Ok(jp) => eprintln!("[extract-eval] json  → {}", jp.display()),
        Err(e) => eprintln!("[extract-eval] WARNING: failed to write JSON report: {e}"),
    }

    Ok(path)
}

async fn run_case(
    db: &Arc<Database>,
    ai: &AiService,
    judge: Option<&ExtractionJudge>,
    kind: ExtractionKind,
    email_id: &str,
    mem_cfg: &crate::services::memory::config::MemoryConfig,
) -> EvalResult<ReportCase> {
    let email = db
        .get_email_by_id(email_id)?
        .ok_or_else(|| EvalError::Config(format!("email {} missing", email_id)))?;
    let body = db.get_email_body(email_id).unwrap_or_default();

    let summary = EmailSummary {
        subject: email.subject.clone(),
        sender: email.sender.clone(),
        sender_email: email.sender_email.clone(),
        body_plain: crate::util::html::strip_html_for_fts(&body),
    };

    // Short-circuit calendar invites: in prod these are skipped upstream.
    // Showing them in the report as SKIPPED lets the reviewer confirm the
    // filter is behaving. Using 8KB for the Teams/Outlook body heuristics.
    let body_head = body.get(..body.len().min(8192)).unwrap_or(&body);
    if extractor::looks_like_calendar_invite_parts(&email.subject, &email.sender_email, body_head) {
        return Ok(ReportCase::skipped(
            email_id,
            &email,
            &summary,
            "calendar invite (skipped in prod)",
        ));
    }

    // Excluded sender — mirrors prod short-circuit.
    if mem_cfg.is_sender_excluded(&email.sender_email) {
        return Ok(ReportCase::skipped(email_id, &email, &summary, "excluded sender"));
    }

    // Excluded tag — mirrors the new tag-based prod filter.
    if !mem_cfg.excluded_tags.is_empty() {
        let tags = db.get_email_tags(email_id).unwrap_or_default();
        if mem_cfg.is_tag_excluded(tags.iter().map(|t| t.tag_value.as_str())) {
            let matched: Vec<&str> = tags
                .iter()
                .map(|t| t.tag_value.as_str())
                .filter(|v| mem_cfg.is_tag_excluded([*v]))
                .collect();
            let reason = format!("excluded tag: {}", matched.join(", "));
            return Ok(ReportCase::skipped(email_id, &email, &summary, &reason));
        }
    }

    let started = Instant::now();
    let mut payload = extractor::extract_for_eval(db, ai, &email).await?;
    let mut thread_summary: Option<String> = None;
    let mut commitment: Option<String> = None;
    let mut deadline_iso: Option<String> = None;
    if matches!(kind, ExtractionKind::Tasks) {
        let tasks_payload = crate::services::tasks::extractor::extract_for_eval(db, ai, &email).await?;
        payload.tasks = tasks_payload.tasks;
        thread_summary = tasks_payload.thread_summary;
        commitment = tasks_payload.commitment;
        deadline_iso = tasks_payload.deadline_iso;
    }
    let extract_ms = started.elapsed().as_millis() as i64;

    let verdict = if let Some(j) = judge {
        Some(j.score(kind, &summary, &payload.tasks, &payload.facts).await)
    } else {
        None
    };

    Ok(ReportCase::ok(
        email_id,
        &email,
        &summary,
        &payload,
        thread_summary,
        commitment,
        deadline_iso,
        extract_ms,
        verdict,
    ))
}

fn sample_primary_email_ids(
    db: &Database,
    account_id: &str,
    limit: usize,
    min_timestamp: Option<i64>,
    // Only emails that answer an earlier message of their thread — where
    // quoted history lives, so where an extractor could re-read it.
    replies_only: bool,
) -> EvalResult<Vec<String>> {
    let conn = db.reader();
    let mut stmt = conn.prepare(
        "SELECT e.id FROM emails e \
         WHERE e.account_id = ?1 \
           AND e.is_deleted = 0 \
           AND e.category = 'primary' \
           AND (?2 IS NULL OR e.timestamp >= ?2) \
           AND (?3 = 0 OR EXISTS ( \
                 SELECT 1 FROM emails p \
                 WHERE p.account_id = e.account_id AND p.thread_id = e.thread_id \
                   AND p.is_deleted = 0 AND p.timestamp < e.timestamp)) \
         ORDER BY e.timestamp DESC \
         LIMIT ?4",
    )?;
    let rows = stmt
        .query_map(
            params![account_id, min_timestamp, replies_only as i64, limit as i64],
            |r| r.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn seed(db: &Database, id: &str, thread: &str, ts: i64) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
             VALUES ('acct', 'gmail', 'me@example.com', 'Me', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails
             (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
              recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
             VALUES (?1,'acct',?2,'s','a','a@x.com','x.com','[]','[]','snip',?3,0,'primary',0)",
            params![id, thread, ts],
        )
        .unwrap();
    }

    #[test]
    fn replies_only_samples_emails_that_answer_an_earlier_message() {
        // Replies are where quoted history lives: the sample that shows whether
        // an extractor re-reads earlier messages.
        let db = Database::new_for_testing().unwrap();
        seed(&db, "first", "t1", 100);
        seed(&db, "reply", "t1", 200);
        seed(&db, "alone", "t2", 300);
        let all = sample_primary_email_ids(&db, "acct", 10, None, false).unwrap();
        assert_eq!(all, vec!["alone", "reply", "first"]);
        let replies = sample_primary_email_ids(&db, "acct", 10, None, true).unwrap();
        assert_eq!(replies, vec!["reply"]);
    }
}
