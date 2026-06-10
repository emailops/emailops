//! One-shot command dispatch. Each arm is a thin call into `services::*`,
//! shared verbatim by the REPL's slash-commands (they map onto the same
//! [`Command`](super::Command) enum) so behaviour never diverges.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::models::error::{AppError, Result};

use super::output;
use super::session::CliSession;
use super::{Command, OutputMode};

/// Execute one parsed command against the session.
pub async fn dispatch(session: &mut CliSession, command: Command) -> Result<()> {
    match command {
        Command::Accounts { action } => super::accounts::run_account(session, action).await,

        Command::Emails {
            limit,
            offset,
            mailbox,
            category,
        } => {
            let account = session.require_account()?;
            let emails = crate::services::emails::get_emails(
                &session.db,
                &account,
                limit,
                offset,
                mailbox.as_deref(),
                category.as_deref(),
            )?;
            output::render_emails(&emails, session.mode)
        }

        Command::Show { id } => {
            let email = session
                .db
                .get_email_by_id(&id)?
                .ok_or_else(|| AppError::NotFound(format!("email '{}' not found", id)))?;
            let body = crate::services::emails::get_email_body(&session.db, &id)?;
            output::render_email(&email, &body, session.mode)
        }

        Command::Search {
            query,
            limit,
            offset,
            trace,
        } => {
            let account = session.require_account()?;
            let result =
                crate::services::search::search_emails(&session.db, &account, &query, false, None, None).await?;
            let total = result.emails.len();
            let emails: Vec<_> = result
                .emails
                .iter()
                .skip(offset)
                .take(limit)
                .map(|e| e.email.clone())
                .collect();
            let shown = emails.len();

            if session.mode == OutputMode::Json {
                if trace {
                    output::emit_ok(serde_json::json!({
                        "emails": emails,
                        "trace": {
                            "query": result.query,
                            "searchMethod": result.search_method,
                            "aiAvailable": result.ai_available,
                            "parsedQuery": result.parsed_query,
                            "shown": shown,
                            "offset": offset,
                            "totalHits": total,
                        }
                    }))
                } else {
                    output::render_emails(&emails, session.mode)
                }
            } else {
                output::render_emails(&emails, session.mode)?;
                if trace {
                    output::render_search_trace(
                        &result.search_method,
                        result.ai_available,
                        result.parsed_query.as_ref(),
                        shown,
                        total,
                    );
                }
                Ok(())
            }
        }

        Command::Chat {
            question,
            trace,
            conversation,
        } => run_chat(session, question, trace, conversation).await,

        Command::Sync { account } => {
            // The positional `account` arg overrides the session/global account.
            let account_id = match account {
                Some(hint) => {
                    let h = hint.trim();
                    crate::services::accounts::list_accounts(&session.db)?
                        .into_iter()
                        .find(|a| a.id.eq_ignore_ascii_case(h) || a.email.eq_ignore_ascii_case(h))
                        .map(|a| a.id)
                        .ok_or_else(|| AppError::NotFound(format!("account '{}' not found", hint)))?
                }
                None => session.require_account()?,
            };

            // No AppHandle from the CLI: progress routes through the events seam;
            // AI follow-ups (memory/tasks/embeddings) and attachment fetches are
            // skipped for v1 sync (download only).
            let ai_background = crate::services::task_queue::TaskQueue::new(1, "cli_sync_ai");
            let sync_abort_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>> = Arc::new(Mutex::new(HashMap::new()));
            let sync_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
                Arc::new(Mutex::new(HashMap::new()));

            crate::services::emails::sync_account(
                &session.db,
                &account_id,
                &session.data_dir,
                None,
                ai_background,
                sync_abort_flags,
                sync_locks,
            )
            .await?;

            if session.mode == OutputMode::Json {
                output::emit_ok(serde_json::json!({ "synced": account_id }))?;
            } else {
                println!("Sync complete for {account_id}.");
            }
            Ok(())
        }

        Command::Classify { all } => {
            let account = session.require_account()?;
            let count = if all {
                crate::services::classification::classify_all_emails(&session.db, &account).await?
            } else {
                crate::services::classification::classify_new_emails(&session.db, &account).await?
            };
            if session.mode == OutputMode::Json {
                output::emit_ok(serde_json::json!({ "classified": count }))?;
            } else {
                println!("Classified {count} email(s).");
            }
            Ok(())
        }

        Command::Embed { batch } => {
            let account = session.require_account()?;
            let count =
                crate::services::embeddings::generate_embeddings(&session.db, Some(&account), None, batch, None)
                    .await?;
            if session.mode == OutputMode::Json {
                output::emit_ok(serde_json::json!({ "embedded": count }))?;
            } else {
                println!("Embedded {count} email(s).");
            }
            Ok(())
        }

        Command::Doctor => {
            let report = super::doctor::build_report(&session.db, &session.data_dir, &session.model)?;
            super::doctor::render(&report, session.mode)
        }

        Command::Config { action } => super::config::run_config(session, action),

        Command::Eval { case, tier, cases_dir } => super::eval::run_eval(session, case, tier, cases_dir).await,
    }
}

/// Resolve the [`Draft`](crate::models::Draft)s named by an assistant message's
/// `referenced_draft_ids` into full draft records. Ids with no matching row
/// (e.g. a since-deleted draft) are skipped rather than erroring.
fn collect_referenced_drafts(
    db: &crate::db::Database,
    referenced_draft_ids: &[String],
) -> Result<Vec<crate::models::Draft>> {
    let mut drafts = Vec::new();
    for id in referenced_draft_ids {
        if let Some(d) = db.get_draft(id)? {
            drafts.push(d);
        }
    }
    Ok(drafts)
}

/// Run a single chat turn end-to-end. Tokens stream to stdout via the installed
/// [`CliEventSink`](super::output::CliEventSink) in pretty mode; in JSON mode the
/// final answer is read back from the DB and printed as one envelope. When
/// `trace` is set the assistant's [`ChatTrace`] and retrieval sources are
/// surfaced too — under `data.trace` / `data.sources` in JSON, or as a dim
/// trace block after the answer in pretty mode.
///
/// When `conversation` names an existing conversation the turn continues it
/// (its prior turns become history, so context carries across one-shot
/// invocations — the same multi-turn behaviour as the REPL); otherwise a new
/// conversation is created. Either way the id is returned as `conversationId`.
async fn run_chat(session: &mut CliSession, question: String, trace: bool, conversation: Option<String>) -> Result<()> {
    let account_id = session.require_account()?;
    let model = session.model.clone();

    let conversation = match conversation {
        Some(id) => session
            .db
            .get_chat_conversation(&id)?
            .ok_or_else(|| AppError::NotFound(format!("conversation '{}' not found", id)))?,
        None => session.db.create_chat_conversation(&account_id, "New chat")?,
    };
    let user_message = session
        .db
        .insert_chat_message(&conversation.id, "user", &question, None)?;
    let assistant_message = session
        .db
        .insert_chat_message(&conversation.id, "assistant", "", Some(&model))?;

    let mut history = session.db.get_recent_chat_turns(&conversation.id, 20)?;
    history.retain(|m| m.id != assistant_message.id && m.id != user_message.id);

    let registry = Arc::new(crate::services::chat::tools::default_registry());
    let categories: Vec<String> = crate::services::chat::DEFAULT_RAG_CATEGORIES
        .iter()
        .map(|s| s.to_string())
        .collect();

    crate::services::chat::run_chat_turn(
        session.db.clone(),
        registry,
        conversation.id.clone(),
        assistant_message.id.clone(),
        account_id,
        question.clone(),
        model,
        history,
        categories,
    )
    .await?;

    let assistant = session
        .db
        .get_chat_messages(&conversation.id)?
        .into_iter()
        .find(|m| m.id == assistant_message.id);
    let answer = assistant.as_ref().map(|m| m.content.clone()).unwrap_or_default();
    let chat_trace = assistant.as_ref().and_then(|m| m.trace.clone());
    let sources = assistant.as_ref().map(|m| m.sources.clone()).unwrap_or_default();

    // Drafts the assistant created this turn are linked to its message via
    // `referenced_draft_ids` and persisted in the `drafts` table; surface them so
    // a terminal/agent user sees the draft body, not just the `draft://` chip.
    let drafts = match assistant.as_ref() {
        Some(m) => collect_referenced_drafts(&session.db, &m.referenced_draft_ids)?,
        None => Vec::new(),
    };

    if session.mode == OutputMode::Json {
        let mut data = serde_json::json!({
            "question": question,
            "answer": answer,
            "conversationId": conversation.id,
            "sources": sources,
            "drafts": drafts,
        });
        if trace {
            data["trace"] = serde_json::to_value(&chat_trace)?;
        }
        output::emit_ok(data)?;
    } else {
        for draft in &drafts {
            output::render_draft(draft);
        }
        if trace {
            output::render_chat_trace(chat_trace.as_ref(), &sources);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::db::Database;

    use super::super::session::CliSession;
    use super::super::OutputMode;
    use super::*;

    fn seed_account(db: &Arc<Database>, id: &str, email: &str, enabled: bool) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES (?1, 'gmail', ?2, ?2, 0, 0, ?3)",
                rusqlite::params![id, email, enabled as i32],
            )
            .expect("seed account");
    }

    /// Build a session straight from parts (bypassing `bootstrap`, which would
    /// touch the keychain and install global seams) so dispatch can be exercised
    /// against an in-memory DB.
    fn test_session(db: Arc<Database>, account: Option<&str>) -> CliSession {
        CliSession {
            db,
            account: account.map(str::to_string),
            model: "test-model".to_string(),
            mode: OutputMode::Json,
            quiet: true,
            data_dir: PathBuf::from("/tmp/emailops-cli-test"),
            conversation_id: None,
        }
    }

    #[tokio::test]
    async fn dispatch_accounts_succeeds_against_seeded_db() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "solo@example.com", true);
        let mut session = test_session(db, Some("a1"));
        dispatch(&mut session, Command::Accounts { action: None })
            .await
            .expect("accounts ok");
    }

    #[tokio::test]
    async fn dispatch_show_missing_id_is_not_found() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let mut session = test_session(db, None);
        let err = dispatch(&mut session, Command::Show { id: "ghost".into() })
            .await
            .expect_err("missing email must error");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn dispatch_emails_without_resolvable_account_errors() {
        // No accounts and no --account hint → require_account fails before any query.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let mut session = test_session(db, None);
        let err = dispatch(
            &mut session,
            Command::Emails {
                limit: 10,
                offset: 0,
                mailbox: None,
                category: None,
            },
        )
        .await
        .expect_err("ambiguous account must error");
        assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn dispatch_sync_unknown_account_is_not_found() {
        // The positional hint doesn't match any account, so it fails fast in the
        // lookup — never reaching the network/provider path.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "real@example.com", true);
        let mut session = test_session(db, Some("a1"));
        let err = dispatch(
            &mut session,
            Command::Sync {
                account: Some("ghost@example.com".into()),
            },
        )
        .await
        .expect_err("unknown sync account must error");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }

    fn save_draft(db: &Arc<Database>, id: &str, account_id: &str) {
        db.save_draft(&crate::models::SaveDraftRequest {
            id: Some(id.to_string()),
            email_id: None,
            account_id: account_id.to_string(),
            to_addresses: vec!["alina@example.com".to_string()],
            subject: "Confirmar reunión".to_string(),
            body: "Hola Alina".to_string(),
        })
        .expect("save draft");
    }

    #[test]
    fn collect_referenced_drafts_resolves_known_ids_and_skips_missing() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "solo@example.com", true);
        save_draft(&db, "d1", "a1");

        let got = collect_referenced_drafts(&db, &["d1".to_string(), "ghost".to_string()]).expect("collect ok");
        assert_eq!(got.len(), 1, "missing id skipped, known id resolved");
        assert_eq!(got[0].id, "d1");
        assert_eq!(got[0].subject, "Confirmar reunión");
    }

    #[test]
    fn collect_referenced_drafts_empty_when_no_refs() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        assert!(collect_referenced_drafts(&db, &[]).expect("collect ok").is_empty());
    }

    #[tokio::test]
    async fn dispatch_doctor_succeeds_and_is_read_only() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "solo@example.com", true);
        let mut session = test_session(db, Some("a1"));
        dispatch(&mut session, Command::Doctor).await.expect("doctor ok");
    }
}
