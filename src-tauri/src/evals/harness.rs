// Per-case runner.
//
// Creates a fresh conversation in the (copied, throwaway) DB, inserts user and
// assistant message rows, invokes `services::chat::run_chat_turn`, and reads
// back the final assistant row + trace + auto-derived title.
//
// Cases that declare `as_of:` pin the global `services::clock` to a fixed
// instant around `run_chat_turn` so time-dependent prompts (e.g. "summarize
// today's emails") resolve deterministically against a static fixture. This
// uses the global clock registry, so eval cases must be driven sequentially —
// concurrent cases would race the swap. The current CLI/runner drivers loop
// over cases serially, which keeps that invariant trivially satisfied.

use std::sync::Arc;
use std::time::Instant;

use chrono::NaiveDate;

use crate::db::Database;
use crate::evals::case_loader::{AsOf, EvalCase};
use crate::evals::{EvalError, EvalResult};
use crate::models::error::AppError;
use crate::models::{ChatConversation, ChatMessage, ChatTrace};
use crate::services::chat;
use crate::services::chat::{smart_body_slice, FULL_BUDGET};
use crate::services::clock::{Clock, FixedClock, SystemClock};
use crate::util::html::strip_html_for_fts;

/// Resolve a case's `as_of` directive to the unix-seconds the harness should
/// install on the global clock, or `Ok(None)` when the case opted out (leave
/// the system clock alone).
///
/// `AsOf::Date(d)` anchors to midnight UTC of `d`.
///
/// `AsOf::Latest` reads `MAX(timestamp)` from the account's inbox **filtered
/// to the same Gmail categories the chat will see at retrieval time**
/// (`DEFAULT_RAG_CATEGORIES` — typically `["primary"]`). Without that filter,
/// pinning "today" to the newest email overall can land on an `updates` or
/// `social` mail that `search_emails` then refuses to surface because the
/// chat turn is scoped to primary — yielding the exact "no emails for today"
/// regression the demo case is meant to prevent. An empty `categories` slice
/// means "no category filter", which preserves the unscoped MAX semantics.
///
/// If no email in scope exists for the account we fail loudly instead of
/// silently falling back to the system clock — a `Latest` directive against
/// an empty inbox-in-scope is almost certainly an environment setup bug.
fn resolve_as_of(
    db: &Database,
    account_id: &str,
    categories: &[String],
    as_of: Option<&AsOf>,
) -> EvalResult<Option<i64>> {
    match as_of {
        None => Ok(None),
        Some(AsOf::Date(d)) => Ok(Some(date_midnight_utc_secs(*d))),
        Some(AsOf::Latest) => {
            let conn = db.reader();
            let max: Option<i64> = if categories.is_empty() {
                conn.query_row(
                    "SELECT MAX(timestamp) FROM emails WHERE account_id = ?1 AND is_deleted = 0",
                    rusqlite::params![account_id],
                    |row| row.get::<_, Option<i64>>(0),
                )?
            } else {
                // Build `category IN (?, ?, …)` dynamically so the scope tracks
                // whatever the chat turn will pass at retrieval time.
                let placeholders = (0..categories.len())
                    .map(|i| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT MAX(timestamp) FROM emails \
                     WHERE account_id = ?1 AND is_deleted = 0 AND category IN ({placeholders})"
                );
                let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(1 + categories.len());
                params.push(&account_id);
                for c in categories {
                    params.push(c);
                }
                conn.query_row(&sql, params.as_slice(), |row| row.get::<_, Option<i64>>(0))?
            };
            match max {
                Some(secs) => Ok(Some(secs)),
                None => Err(EvalError::Config(format!(
                    "as_of=latest requested but no emails found for account {account_id} in categories {categories:?} — ensure the fixture is seeded before running the case"
                ))),
            }
        }
    }
}

fn date_midnight_utc_secs(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc().timestamp())
        // Out-of-range dates are caught by NaiveDate's own range; this branch
        // exists so the helper stays infallible without an .expect().
        .unwrap_or(0)
}

/// RAII guard: installs `pinned` on the global clock at construction and
/// restores `SystemClock` on drop, so a panic or early-return inside the
/// chat turn cannot leave the registry pointing at a stale FixedClock.
struct ClockGuard;

impl ClockGuard {
    fn install(pinned_secs: i64) -> Self {
        crate::services::clock::install(Arc::new(FixedClock::new(pinned_secs)) as Arc<dyn Clock>);
        Self
    }
}

impl Drop for ClockGuard {
    fn drop(&mut self) {
        crate::services::clock::install(Arc::new(SystemClock));
    }
}

/// Outcome of running one eval case end to end.
pub struct CaseOutcome {
    pub conversation_id: String,
    pub conversation_title: String,
    pub assistant_message_id: String,
    pub assistant_content: String,
    pub assistant_trace: Option<ChatTrace>,
    pub assistant_token_count: Option<i32>,
    pub assistant_latency_ms: Option<i64>,
    /// Wall-clock time inside the harness (turn dispatch + DB readback).
    pub wall_elapsed_ms: i64,
    pub sources_used: Vec<SourceSummary>,
}

/// Lightweight view of a `ChatMessageSource` for the report.
#[derive(Debug, Clone)]
pub struct SourceSummary {
    pub citation_number: i32,
    pub email_id: String,
    pub subject: String,
    pub sender: String,
    pub sender_email: String,
    pub relevance_score: Option<f32>,
    /// The exact body snippet that was fed to the model (post HTML-strip, smart-sliced).
    pub body_snippet: String,
}

/// Run a single case against `services::chat::run_chat_turn`.
///
/// Returns `Err` only on infrastructure-level failures (DB unavailable,
/// assistant row missing, etc). LLM-side failures are captured inside the
/// assistant content / trace and reported as-is.
pub async fn run_case(db: Arc<Database>, account_id: &str, model: &str, case: &EvalCase) -> EvalResult<CaseOutcome> {
    let start = Instant::now();

    // Category scope the chat turn will use at retrieval time — computed up
    // front so `resolve_as_of` can apply the same filter when resolving
    // `AsOf::Latest`. Eval runs use the service default (primary only);
    // callers that need a wider scope should extend the case schema.
    let categories: Vec<String> = crate::services::chat::DEFAULT_RAG_CATEGORIES
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Cases name accounts the way humans do; the grounding lookup keys on the
    // account *id*. `account:` already accepts either form (see
    // `runner.rs`'s per-case override), so `ambient_account:` must too —
    // passing an email straight through silently finds no thread and the turn
    // falls back to retrieval, i.e. it reproduces the very bug the case exists
    // to catch and looks like a product regression.
    fn resolve_ambient_account(db: &Database, hint: Option<&str>) -> Option<String> {
        let hint = hint.map(str::trim).filter(|h| !h.is_empty())?;
        let resolved = db.list_accounts().ok().and_then(|accounts| {
            accounts
                .into_iter()
                .find(|a| a.id.eq_ignore_ascii_case(hint) || a.email.eq_ignore_ascii_case(hint))
                .map(|a| a.id)
        });
        // Unresolved: hand the raw hint through so the case fails on its own
        // assertions rather than the harness silently dropping the context.
        Some(resolved.unwrap_or_else(|| hint.to_string()))
    }

    // 0. Pin "today" if the case opted in via `as_of:`. The guard restores
    //    `SystemClock` on drop, so a panic or early-return inside the chat
    //    turn cannot leave the global registry pointing at a stale clock.
    let _clock_guard = resolve_as_of(&db, account_id, &categories, case.as_of.as_ref())?.map(ClockGuard::install);

    // 1. Fresh conversation. Thread-bound cases seed it with the cleaned
    //    thread (role='system' message) so `run_chat_turn` takes the
    //    thread-bound short-circuit; everything else starts as an empty chat
    //    whose title defaults to "New chat" so the auto-title logic triggers.
    let binding =
        crate::evals::case_loader::plan_thread_binding(case.thread_id.as_deref(), case.thread_subject.as_deref())
            .map_err(|e| AppError::InvalidInput(format!("case '{}': {e}", case.id)))?;
    let thread_id: Option<String> = match binding {
        crate::evals::case_loader::ThreadBinding::None => None,
        crate::evals::case_loader::ThreadBinding::Id(id) => Some(id),
        crate::evals::case_loader::ThreadBinding::Subject(subject) => {
            Some(db.find_thread_id_by_subject(account_id, &subject)?.ok_or_else(|| {
                AppError::InvalidInput(format!(
                    "case '{}': no thread with subject '{subject}' in account '{account_id}' — \
                     the case names a thread the DB does not contain",
                    case.id
                ))
            })?)
        }
    };
    let conv: ChatConversation = match thread_id.as_deref() {
        Some(thread_id) => crate::services::chat::create_conversation_with_thread(&db, account_id, thread_id)?,
        None => db.create_chat_conversation(account_id, "New chat")?,
    };

    // 2. User + empty assistant rows, matching the production command flow.
    let user_msg: ChatMessage = db.insert_chat_message(&conv.id, "user", &case.question, None)?;
    let assistant_msg: ChatMessage = db.insert_chat_message(&conv.id, "assistant", "", Some(model))?;

    // 3. Single-turn cases: history is empty (the service re-adds the user
    //    question as the final message itself).
    let history: Vec<ChatMessage> = Vec::new();

    // 4. Run the turn. Errors here are service-level (e.g. Ollama unreachable),
    //    so we wrap them into EvalError but do not swallow — the runner decides
    //    whether that aborts the whole suite. `categories` was computed up
    //    front so `resolve_as_of` could share the same scope.

    // run_chat_turn now takes an injected ToolRegistry so production code
    // can hold a single Arc on AppState. Evals don't share state with the
    // running app, so build a fresh default registry per case.
    let registry = std::sync::Arc::new(crate::services::chat::tools::default_registry());
    chat::run_chat_turn(
        db.clone(),
        registry,
        conv.id.clone(),
        user_msg.id.clone(),
        assistant_msg.id.clone(),
        account_id.to_string(),
        case.question.clone(),
        model.to_string(),
        history,
        categories,
        // Ambient view context. Normally absent — a case drives the retrieval
        // pipeline and there is no open view — but a case may set
        // `ambient_thread_id` to reproduce the chat panel's context chip,
        // including the cross-account shape where the thread belongs to an
        // account other than the one the chat runs on.
        case.ambient_thread_id.clone(),
        resolve_ambient_account(&db, case.ambient_account.as_deref()),
    )
    .await?;

    let wall_elapsed_ms = start.elapsed().as_millis() as i64;

    // 5. Read back the assistant row (final content + trace + stats + sources).
    let messages = db.get_chat_messages(&conv.id)?;
    let assistant = messages.into_iter().find(|m| m.id == assistant_msg.id).ok_or_else(|| {
        EvalError::Config(format!(
            "assistant message {} missing from conversation {}",
            assistant_msg.id, conv.id
        ))
    })?;

    // 6. Re-read the conversation to pick up the auto-derived title.
    let final_title = db
        .get_chat_conversation(&conv.id)?
        .map(|c| c.title)
        .unwrap_or_else(|| "New chat".into());

    let sources_used = assistant
        .sources
        .iter()
        .map(|s| {
            // Re-derive the exact body snippet that was fed to the model so
            // the report can show exactly what context the LLM received for
            // each source. Mirrors the logic in services::chat::build_prompt.
            let raw_body = db.get_email_body(&s.email_id).unwrap_or_default();
            let plain = strip_html_for_fts(&raw_body);
            let snippet = smart_body_slice(&plain, &case.question, FULL_BUDGET.source_body_chars);
            SourceSummary {
                citation_number: s.citation_number,
                email_id: s.email_id.clone(),
                subject: s.subject.clone(),
                sender: s.sender.clone(),
                sender_email: s.sender_email.clone(),
                relevance_score: s.relevance_score,
                body_snippet: snippet,
            }
        })
        .collect();

    Ok(CaseOutcome {
        conversation_id: conv.id,
        conversation_title: final_title,
        assistant_message_id: assistant.id,
        assistant_content: assistant.content,
        assistant_trace: assistant.trace,
        assistant_token_count: assistant.token_count,
        assistant_latency_ms: assistant.latency_ms,
        wall_elapsed_ms,
        sources_used,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    /// Serialise tests that touch the global clock registry; otherwise the
    /// guard install / restore can race a parallel test reading `now_secs`.
    fn clock_lock() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn seed_account(db: &Database, id: &str) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                     VALUES (?1, 'gmail', ?1, 'Test', 0)",
                rusqlite::params![id],
            )
            .expect("seed account");
    }

    fn seed_email(db: &Database, account_id: &str, id: &str, timestamp: i64) {
        seed_email_cat(db, account_id, id, timestamp, "primary");
    }

    fn seed_email_cat(db: &Database, account_id: &str, id: &str, timestamp: i64, category: &str) {
        db.connection()
            .execute(
                "INSERT INTO emails (
                    id, account_id, thread_id, subject, sender, sender_email,
                    sender_domain, recipients_json, cc_json, snippet,
                    timestamp, is_read, is_deleted, category, mailbox, raw_json, created_at
                ) VALUES (?1, ?2, ?3, 'Subject', 'Alice', 'alice@ex.com',
                    'ex.com', '[]', '[]', '', ?4, 0, 0, ?5, 'inbox', NULL, ?4)",
                rusqlite::params![id, account_id, format!("th-{id}"), timestamp, category],
            )
            .expect("seed email");
    }

    fn primary_only() -> Vec<String> {
        vec!["primary".to_string()]
    }

    #[test]
    fn resolve_as_of_returns_none_when_not_set() {
        let db = Database::new_for_testing().expect("test db");
        let got = resolve_as_of(&db, "acct", &primary_only(), None).expect("resolve");
        assert_eq!(got, None);
    }

    #[test]
    fn resolve_as_of_date_anchors_to_midnight_utc() {
        let db = Database::new_for_testing().expect("test db");
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid date");
        let got = resolve_as_of(&db, "acct", &primary_only(), Some(&AsOf::Date(d))).expect("resolve");
        // 2024-01-15T00:00:00Z = 1_705_276_800.
        assert_eq!(got, Some(1_705_276_800));
    }

    #[test]
    fn resolve_as_of_latest_uses_max_timestamp_for_account() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        seed_account(&db, "acct-2");
        seed_email(&db, "acct-1", "e1", 100);
        seed_email(&db, "acct-1", "e2", 5_000);
        seed_email(&db, "acct-1", "e3", 1_500);
        seed_email(&db, "acct-2", "other", 9_999); // unrelated account, ignored

        let got = resolve_as_of(&db, "acct-1", &primary_only(), Some(&AsOf::Latest)).expect("resolve");
        assert_eq!(got, Some(5_000));
    }

    /// Regression for the demo_daily_summary bug: with `categories=["primary"]`
    /// the resolver must IGNORE newer-but-out-of-scope emails, because the
    /// today-shortcut later filters `search_emails` by the same category set.
    /// Without this filter, `as_of: latest` lands on an `updates` mail and
    /// the chat then surfaces "no emails found for today".
    #[test]
    fn resolve_as_of_latest_respects_category_scope() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        seed_email_cat(&db, "acct-1", "p-old", 1_000, "primary");
        seed_email_cat(&db, "acct-1", "u-new", 9_999, "updates");
        seed_email_cat(&db, "acct-1", "p-mid", 5_000, "primary");

        // Scoped to primary: must skip the `updates` mail even though it is newer.
        let got = resolve_as_of(&db, "acct-1", &primary_only(), Some(&AsOf::Latest)).expect("resolve");
        assert_eq!(got, Some(5_000), "must pin to the newest primary, not the updates mail");

        // Empty categories slice means "no filter" — falls back to unscoped MAX.
        let unscoped = resolve_as_of(&db, "acct-1", &[], Some(&AsOf::Latest)).expect("resolve");
        assert_eq!(unscoped, Some(9_999));
    }

    /// Soft-deleted mails must not anchor `Latest` either — the chat won't
    /// retrieve them, so neither should the clock pinning.
    #[test]
    fn resolve_as_of_latest_ignores_soft_deleted_emails() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        seed_email(&db, "acct-1", "alive", 1_000);
        // Insert a deleted email with a newer timestamp.
        db.connection()
            .execute(
                "INSERT INTO emails (
                    id, account_id, thread_id, subject, sender, sender_email,
                    sender_domain, recipients_json, cc_json, snippet,
                    timestamp, is_read, is_deleted, category, mailbox, raw_json, created_at
                ) VALUES ('dead', 'acct-1', 'th-dead', '', '', '', '', '[]', '[]', '',
                          9_999, 0, 1, 'primary', 'inbox', NULL, 9_999)",
                [],
            )
            .expect("seed deleted email");

        let got = resolve_as_of(&db, "acct-1", &primary_only(), Some(&AsOf::Latest)).expect("resolve");
        assert_eq!(got, Some(1_000));
    }

    #[test]
    fn resolve_as_of_latest_errors_when_account_inbox_is_empty() {
        let db = Database::new_for_testing().expect("test db");
        seed_account(&db, "acct-1");
        let err =
            resolve_as_of(&db, "acct-1", &primary_only(), Some(&AsOf::Latest)).expect_err("must fail on empty inbox");
        match err {
            EvalError::Config(msg) => {
                assert!(msg.contains("as_of=latest"), "msg should explain the failure: {msg}");
                assert!(msg.contains("acct-1"), "msg should name the account: {msg}");
                assert!(msg.contains("primary"), "msg should name the category scope: {msg}");
            }
            other => panic!("expected EvalError::Config, got {other:?}"),
        }
    }

    #[test]
    fn clock_guard_installs_pinned_and_restores_system_clock_on_drop() {
        let _g = clock_lock();
        // Baseline: system clock is whatever was last installed (default
        // SystemClock at module init). Snapshot it so we can assert we
        // restored *something near wall-clock* after the guard drops.
        crate::services::clock::install(Arc::new(SystemClock));
        let baseline = crate::services::clock::now_secs();

        {
            let _guard = ClockGuard::install(1_705_276_800);
            assert_eq!(crate::services::clock::now_secs(), 1_705_276_800);
        }

        // After drop, the SystemClock is back — the pinned value cannot
        // survive the guard's scope.
        let after = crate::services::clock::now_secs();
        assert!(
            after >= baseline,
            "clock should advance forward after restore, baseline={baseline} after={after}"
        );
        assert_ne!(after, 1_705_276_800, "pinned value must not leak past the guard");
    }
}
