//! One-shot command dispatch. Each arm is a thin call into `services::*`,
//! shared verbatim by the REPL's slash-commands (they map onto the same
//! [`Command`](super::Command) enum) so behaviour never diverges.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::models::error::{AppError, Result};

use super::output;
use super::session::CliSession;
use super::{Command, OutputMode, RenderStyle};

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
                Some(&account),
                limit,
                offset,
                mailbox.as_deref(),
                category.as_deref(),
            )?;
            output::render_emails(&emails, session.style)
        }

        Command::Show { id } => {
            let email = session
                .db
                .get_email_by_id(&id)?
                .ok_or_else(|| AppError::NotFound(format!("email '{}' not found", id)))?;
            // JSON keeps the single-email contract agents rely on.
            if session.style == RenderStyle::Json {
                let body = crate::services::emails::get_email_body(&session.db, &id)?;
                return output::render_email(&email, &body, session.style);
            }
            // Pretty: visualize the whole thread, each message body indented.
            let thread = crate::services::emails::get_thread(&session.db, &email.account_id, &email.thread_id)?;
            let account_email = session.db.get_account(&email.account_id)?.map(|a| a.email);
            let mut msgs: Vec<(crate::models::Email, String)> = Vec::with_capacity(thread.len());
            for e in thread {
                let body = crate::services::emails::get_email_body(&session.db, &e.id)?;
                msgs.push((e, body));
            }
            if msgs.is_empty() {
                // Thread lookup came back empty — fall back to the single email.
                let body = crate::services::emails::get_email_body(&session.db, &id)?;
                return output::render_email(&email, &body, session.style);
            }
            output::render_thread(&msgs, &email.id, account_email.as_deref(), session.style)
        }

        Command::Search {
            query,
            limit,
            offset,
            trace,
        } => {
            let account = session.require_account()?;
            let result =
                crate::services::search::search_emails(&session.db, Some(&account), &query, false, None, None).await?;
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
                    output::render_emails(&emails, session.style)
                }
            } else {
                output::render_emails(&emails, session.style)?;
                if trace {
                    output::render_search_trace(
                        &result.search_method,
                        result.ai_available,
                        result.parsed_query.as_ref(),
                        shown,
                        total,
                        session.style.color(),
                    );
                }
                Ok(())
            }
        }

        Command::Chat {
            questions,
            trace,
            conversation,
            fresh,
            thread,
            prewarm,
        } => run_chat(session, questions, trace, conversation, fresh, thread, prewarm).await,

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

        Command::Classify { all, id } => {
            let account = session.require_account()?;
            // The single-id path returns the persisted tags so the caller can
            // see what was decided (priority / intent / topic / confidence /
            // method) without a follow-up sqlite query. The batch paths still
            // return just a count — exposing per-email rows there would dump
            // hundreds of objects on a real mailbox.
            if let Some(email_id) = id {
                let outcome =
                    crate::services::classification::classify_email_by_id(&session.db, &account, &email_id).await?;
                let count = if outcome.is_some() { 1u32 } else { 0u32 };
                if session.mode == OutputMode::Json {
                    output::emit_ok(serde_json::json!({
                        "classified": count,
                        "result": outcome,
                    }))?;
                } else if let Some(o) = outcome {
                    let conf = o.confidence.map(|c| format!("{:.2}", c)).unwrap_or_else(|| "—".into());
                    println!(
                        "Classified {} as priority={} intent={} topic={} (confidence={}, method={})",
                        o.email_id, o.priority, o.intent, o.topic, conf, o.method
                    );
                } else {
                    println!("Email {email_id} not classified (master switch off, disabled, or skipped).");
                }
            } else {
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
            }
            Ok(())
        }

        Command::Junk {
            all,
            train,
            id,
            explain,
            bootstrap_labels,
            review,
            sample,
            label,
            measure,
            export_cases,
        } => {
            let account = session.require_account()?;

            if bootstrap_labels || review.is_some() || label.is_some() || measure || export_cases {
                return run_golden(session, &account, bootstrap_labels, review, sample, label, export_cases).await;
            }

            if let Some(email_id) = explain {
                // The "why was this flagged?" surface. Prints the materialized
                // signals alongside the verdict, so a wrong call can be traced
                // to the input that caused it rather than guessed at.
                let ctx = crate::services::junk::signals::AccountContext::load(&session.db, &account)?;
                let Some(signals) = crate::services::junk::signals::materialize(&session.db, &ctx, &email_id)? else {
                    return Err(AppError::NotFound(format!("email {email_id}")));
                };
                let verdict = crate::services::junk::verdict::judge(
                    &signals,
                    &crate::services::junk::verdict::Weights::default(),
                );
                if session.mode == OutputMode::Json {
                    output::emit_ok(serde_json::json!({
                        "email_id": email_id,
                        "signals": signals,
                        "verdict": verdict,
                    }))?;
                } else {
                    println!("{email_id}");
                    println!(
                        "  phishing {:?} ({:.2})  spam {:?} ({:.2})  graymail {:?} ({:.2})",
                        verdict.phishing.band,
                        verdict.phishing.score,
                        verdict.spam.band,
                        verdict.spam.score,
                        verdict.graymail.band,
                        verdict.graymail.score
                    );
                    println!("  primary: {:?}  method: {:?}", verdict.primary, verdict.method);
                    for reason in &verdict.reasons {
                        let detail = reason.detail.as_deref().unwrap_or("");
                        println!(
                            "    {:?} [{:?}] +{:.2} {}",
                            reason.code, reason.axis, reason.weight, detail
                        );
                    }
                }
                return Ok(());
            }

            if train {
                let trained = crate::services::junk::train_models(&session.db, &account).await?;
                if session.mode == OutputMode::Json {
                    let axes: Vec<_> = trained
                        .iter()
                        .map(|(axis, pos, neg)| serde_json::json!({ "axis": axis, "positives": pos, "negatives": neg }))
                        .collect();
                    output::emit_ok(serde_json::json!({ "trained": axes }))?;
                } else {
                    for (axis, pos, neg) in &trained {
                        println!("Trained {axis}: {pos} positive / {neg} negative samples");
                    }
                }
                return Ok(());
            }

            let scored = match (all, id) {
                (_, Some(email_id)) => {
                    crate::services::junk::score_email_by_id(&session.db, &account, &email_id).await?
                }
                (true, None) => crate::services::junk::backfill_account(&session.db, &account).await?,
                (false, None) => crate::services::junk::score_new_emails(&session.db, &account).await?,
            };
            if session.mode == OutputMode::Json {
                output::emit_ok(serde_json::json!({ "scored": scored }))?;
            } else {
                println!("Scored {scored} email(s).");
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

        Command::Stats => {
            // Same per-account aggregates as the app's dashboard cards.
            let dashboards = crate::services::dashboard::collect_dashboards(&session.db)?;
            output::render_stats(&dashboards, session.style)
        }

        Command::Compose {
            to,
            cc,
            subject,
            body,
            body_file,
            attach,
            send,
            draft,
        } => run_compose(session, to, cc, subject, body, body_file, attach, send, draft).await,

        Command::Calendar { days, next, sync } => {
            let account = session.require_account()?;
            let now = chrono::Utc::now().timestamp();
            if sync {
                let acct = session
                    .db
                    .get_account(&account)?
                    .ok_or_else(|| AppError::NotFound(format!("account '{account}' not found")))?;
                let provider = crate::services::calendar::sync::build_calendar_provider(&acct.id, &acct.provider)?;
                let count = crate::services::calendar::sync::sync_account_calendar(
                    &session.db,
                    &acct.id,
                    provider.as_ref(),
                    now,
                )
                .await?;
                crate::services::logger::log("info", "sync", format!("calendar sync: {count} events in window"));
            }
            // Agenda starts at local midnight so today's earlier events show.
            let start_of_today = chrono::Local::now()
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|naive| naive.and_local_timezone(chrono::Local).single())
                .map(|dt| dt.timestamp())
                .unwrap_or(now);
            let range_end = start_of_today + days.max(1).saturating_mul(86_400);
            let events = session
                .db
                .list_visible_calendar_events(&account, start_of_today, range_end)?;
            if next {
                // Events are start-time ordered; the first not-yet-started one
                // is the next meeting.
                let upcoming: Vec<crate::models::CalendarEvent> =
                    events.into_iter().filter(|e| e.start_time > now).take(1).collect();
                return output::render_calendar_events(&upcoming, session.style);
            }
            output::render_calendar_events(&events, session.style)
        }

        Command::Drafts => {
            let account = session.require_account()?;
            let drafts = crate::services::emails::list_drafts(&session.db, &account)?;
            output::render_drafts(&drafts, session.style)
        }

        Command::Draft { id } => {
            let draft = session
                .db
                .get_draft(&id)?
                .ok_or_else(|| AppError::NotFound(format!("draft '{}' not found", id)))?;
            output::render_draft_detail(&draft, session.style)
        }

        Command::Translate { id, to, detect_only } => {
            if detect_only {
                let result = crate::services::translation::detect_email_language(&session.db, &id).await?;
                if session.mode == OutputMode::Json {
                    return output::emit_ok(&result);
                }
                println!(
                    "language: {} (preferred: {}) — {}",
                    result.language,
                    result.preferred,
                    if result.needs_translation {
                        "translation available"
                    } else {
                        "no translation needed"
                    }
                );
                Ok(())
            } else {
                let result = crate::services::translation::translate_email(&session.db, &id, to.as_deref()).await?;
                if session.mode == OutputMode::Json {
                    return output::emit_ok(&result);
                }
                println!("── translated to {} ──", result.target_language);
                if result.truncated {
                    println!("(input was truncated to fit the model's context window)");
                }
                println!("{}", result.text);
                Ok(())
            }
        }

        Command::Config { action } => super::config::run_config(session, action),

        Command::Eval { case, tier, cases_dir } => super::eval::run_eval(session, case, tier, cases_dir).await,
    }
}

/// Compose flow shared by the one-shot command and the REPL: resolve the body,
/// save the draft (pushing to the provider when supported), and optionally send.
#[allow(clippy::too_many_arguments)]
async fn run_compose(
    session: &mut CliSession,
    to: Vec<String>,
    cc: Vec<String>,
    subject: String,
    body: Option<String>,
    body_file: Option<std::path::PathBuf>,
    attach: Vec<std::path::PathBuf>,
    send: bool,
    draft_id: Option<String>,
) -> Result<()> {
    let account_id = session.require_account()?;
    let account = session
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account '{}' not found", account_id)))?;

    // Body: --body-file wins if given, else --body, else empty.
    let body = match body_file {
        Some(path) => std::fs::read_to_string(&path)
            .map_err(|e| AppError::InvalidInput(format!("cannot read --body-file {}: {e}", path.display())))?,
        None => body.unwrap_or_default(),
    };

    let attachments: Vec<crate::models::DraftAttachmentInput> = attach
        .iter()
        .map(|p| crate::models::DraftAttachmentInput {
            file_path: p.to_string_lossy().to_string(),
            filename: None,
            mime_type: None,
        })
        .collect();

    let input = crate::services::emails::ComposeInput {
        draft_id,
        account_id: account_id.clone(),
        email_id: None,
        to: to.clone(),
        cc,
        subject: subject.clone(),
        body,
        body_html: None,
        // CLI compose explicitly manages the attachment set (`--attach`), so
        // always send `Some` — omitting `--attach` on an update clears them.
        attachments: Some(attachments),
    };

    // Build a provider when the account supports server-side drafts, or when we
    // are about to send (which always needs one). `--send` propagates a build
    // error; a draft-only save degrades to local-only.
    let supports_drafts = crate::sync::provider::provider_supports_drafts(&account.provider);
    let provider = if send {
        Some(crate::services::emails::build_provider(&account, None).await?)
    } else if supports_drafts {
        crate::services::emails::build_provider(&account, None).await.ok()
    } else {
        None
    };

    let saved = crate::services::emails::compose_draft(&session.db, &account, input, provider.as_deref()).await?;

    if send {
        // A provider is guaranteed here (built above or we returned early).
        let provider = provider.ok_or_else(|| AppError::SyncError("provider unavailable for send".to_string()))?;
        crate::services::emails::send_draft(&session.db, &account, &saved.id, provider.as_ref()).await?;
        if session.mode == OutputMode::Json {
            output::emit_ok(serde_json::json!({
                "sent": true,
                "to": to,
                "subject": subject,
            }))
        } else {
            println!("Sent to {}.", to.join(", "));
            Ok(())
        }
    } else if session.mode == OutputMode::Json {
        output::emit_ok(saved)
    } else {
        output::render_draft_detail(&saved, session.style)
    }
}

/// Resolve the [`Draft`](crate::models::Draft)s named by an assistant message's
/// `referenced_draft_ids` into full draft records. Ids with no matching row
/// (e.g. a since-deleted draft) are skipped rather than erroring.
pub(super) fn collect_referenced_drafts(
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

/// Run one or more chat turns end-to-end in a single conversation. Tokens
/// stream to stdout via the installed
/// [`CliEventSink`](super::output::CliEventSink) in pretty mode; in JSON mode
/// each answer is read back from the DB and printed as one envelope at the
/// end. When `trace` is set the assistant's [`ChatTrace`] and retrieval
/// sources are surfaced too — under `data.trace` / `data.sources` (single
/// question) or per entry in `data.turns` (multiple), or as a dim trace block
/// after each answer in pretty mode.
///
/// When `conversation` names an existing conversation the turns continue it
/// (its prior turns become history, so context carries across one-shot
/// invocations — the same multi-turn behaviour as the REPL); otherwise a new
/// conversation is created. Either way the id is returned as `conversationId`.
///
/// Multiple questions run sequentially in ONE process so the model stays
/// loaded between turns — that's what makes per-turn prefill numbers
/// comparable (`make cli-bench`).
async fn run_chat(
    session: &mut CliSession,
    questions: Vec<String>,
    trace: bool,
    conversation: Option<String>,
    fresh: bool,
    thread: Option<String>,
    prewarm: bool,
) -> Result<()> {
    let account_id = session.require_account()?;
    let model = session.model.clone();

    // The per-turn diagnostic app-log stream (route / retrieval / kv / stage)
    // is "the trace" for chat: keep stdout a clean answer channel and only
    // surface it when `--trace` is passed (errors always pass through).
    session.apply_chat_log_quiet(trace);

    // `fresh` starts a brand-new conversation per question (cross-conversation
    // cache-reuse measurement); otherwise all questions share one conversation
    // (the default multi-turn behaviour). `--conversation` always pins one
    // existing conversation, so it wins over `--fresh`.
    let pinned = conversation.is_some();
    let mut conversation = match &conversation {
        Some(id) => session
            .db
            .get_chat_conversation(id)?
            .ok_or_else(|| AppError::NotFound(format!("conversation '{}' not found", id)))?,
        None => session.db.create_chat_conversation(&account_id, "New chat")?,
    };
    // `--conversation` pins one conversation, so it wins over `--fresh`.
    let fresh = fresh && !pinned;

    let registry = Arc::new(crate::services::chat::tools::default_registry());
    let categories: Vec<String> = crate::services::chat::DEFAULT_RAG_CATEGORIES
        .iter()
        .map(|s| s.to_string())
        .collect();

    // `--thread` is documented as grounding "exactly as the app's chat panel"
    // does, and the panel sends the thread together with the account that owns
    // it — the two need not match the account the chat itself runs on (unified
    // mode picks the first enabled one). Mirror that here by finding the owner,
    // so `--thread X --account Y` grounds correctly instead of silently
    // dropping the context. `None` (thread not found under any account) leaves
    // the existing behaviour: fall back to the turn's own account.
    let ambient_account: Option<String> = match thread.as_deref() {
        Some(thread_id) => session
            .db
            .list_accounts()
            .unwrap_or_default()
            .into_iter()
            .find(|a| {
                session
                    .db
                    .get_thread(&a.id, thread_id)
                    .map(|emails| !emails.is_empty())
                    .unwrap_or(false)
            })
            .map(|a| a.id),
        None => None,
    };

    // Mirror the app's idle-time prompt-prefix prewarm before the first turn
    // so turn-1 prefill numbers match the in-app first-turn experience.
    // Best-effort: a prewarm failure just means a normal cold prefill.
    if prewarm {
        let provider = crate::services::ai::AiService::load_provider_with_model(&session.db, Some(&model))?;
        if let Err(e) =
            crate::services::chat::prewarm_chat(&session.db, &registry, provider.as_ref(), &account_id).await
        {
            eprintln!("prewarm failed (continuing cold): {e}");
        }
    }

    let total = questions.len();
    let mut turns: Vec<serde_json::Value> = Vec::with_capacity(total);

    for (i, question) in questions.into_iter().enumerate() {
        if session.mode == OutputMode::Pretty && total > 1 {
            println!("\n>>> [{}/{}] {}", i + 1, total, question);
        }

        // In `--fresh` mode every question after the first opens its own new
        // conversation so no prior history is carried over — only the resident
        // model/KV cache is shared across them.
        if fresh && i > 0 {
            conversation = session.db.create_chat_conversation(&account_id, "New chat")?;
        }

        let user_message = session
            .db
            .insert_chat_message(&conversation.id, "user", &question, None)?;
        let assistant_message = session
            .db
            .insert_chat_message(&conversation.id, "assistant", "", Some(&model))?;

        let mut history = session.db.get_recent_chat_turns(&conversation.id, 20)?;
        history.retain(|m| m.id != assistant_message.id && m.id != user_message.id);

        crate::services::chat::run_chat_turn(
            session.db.clone(),
            registry.clone(),
            conversation.id.clone(),
            user_message.id.clone(),
            assistant_message.id.clone(),
            account_id.clone(),
            question.clone(),
            model.clone(),
            history,
            categories.clone(),
            thread.clone(),
            ambient_account.clone(),
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
        // `referenced_draft_ids` and persisted in the `drafts` table; surface
        // them so a terminal/agent user sees the draft body, not just the
        // `draft://` chip.
        let drafts = match assistant.as_ref() {
            Some(m) => collect_referenced_drafts(&session.db, &m.referenced_draft_ids)?,
            None => Vec::new(),
        };

        if session.mode == OutputMode::Json {
            let mut turn = serde_json::json!({
                "question": question,
                "answer": answer,
                "sources": sources,
                "drafts": drafts,
            });
            if trace {
                turn["trace"] = serde_json::to_value(&chat_trace)?;
            }
            turns.push(turn);
        } else {
            // Re-render the finished answer as aligned, styled markdown — in Rich
            // mode this replaces the dim live preview the sink just cleared; in
            // Plain mode it's the answer's first (ANSI-free) appearance.
            output::render_final_answer(&answer, session.style);
            for draft in &drafts {
                output::render_draft(draft, session.style.color());
            }
            if trace {
                output::render_chat_trace(chat_trace.as_ref(), &sources, session.style.color());
            }
        }
    }

    if session.mode == OutputMode::Json {
        // Single question keeps the flat envelope agents already parse;
        // multi-question runs report one entry per turn under `turns`.
        let data = if total == 1 {
            let mut data = turns.remove(0);
            data["conversationId"] = serde_json::Value::String(conversation.id.clone());
            data
        } else {
            serde_json::json!({
                "conversationId": conversation.id,
                "turns": turns,
            })
        };
        output::emit_ok(data)?;
    }

    Ok(())
}

/// Private golden-set operations: seed, review, hand-label, measure.
///
/// The label file holds pointers only — id, account, label, source — so the
/// user's mail never leaves SQLite.
async fn run_golden(
    session: &mut CliSession,
    account: &str,
    bootstrap_labels: bool,
    review: Option<usize>,
    sample: bool,
    label: Option<String>,
    export_cases: bool,
) -> Result<()> {
    use crate::services::junk::golden;

    let path = golden::default_path();
    let mut entries = golden::load(&path)?;

    if bootstrap_labels {
        let seeded = golden::bootstrap(&session.db, account, 5_000)?;
        let before = entries.len();
        entries = golden::merge(entries, seeded);
        golden::save(&path, &entries)?;
        let added = entries.len().saturating_sub(before);
        if session.mode == OutputMode::Json {
            output::emit_ok(serde_json::json!({
                "path": path.display().to_string(),
                "total": entries.len(),
                "added": added,
            }))?;
        } else {
            println!("Seeded {added} label(s); {} total → {}", entries.len(), path.display());
            println!("Only provider-folder and user-override labels are seeded — everything else");
            println!("would grade the detector against its own inputs. Use --review to add more.");
        }
        return Ok(());
    }

    if let Some(spec) = label {
        let (email_id, value) = spec
            .split_once('=')
            .ok_or_else(|| AppError::InvalidInput("expected --label <email-id>=<label>".into()))?;
        let parsed = golden::GoldenLabel::parse(value)
            .ok_or_else(|| AppError::InvalidInput(format!("unknown label {value:?}")))?;
        entries = golden::merge(
            entries,
            vec![golden::GoldenEntry {
                email_id: email_id.to_string(),
                account_id: account.to_string(),
                label: parsed,
                source: golden::LabelSource::Manual,
                labeled_at: crate::services::clock::now_secs(),
            }],
        );
        golden::save(&path, &entries)?;
        if session.mode == OutputMode::Json {
            output::emit_ok(serde_json::json!({ "email_id": email_id, "label": parsed.as_str() }))?;
        } else {
            println!("Labelled {email_id} as {}", parsed.as_str());
        }
        return Ok(());
    }

    if let Some(n) = review {
        let ids = golden::unlabelled(&session.db, account, &entries, n, sample)?;
        let mut rows = Vec::new();
        for id in &ids {
            let Some(email) = session.db.get_email_by_id(id)? else {
                continue;
            };
            rows.push(serde_json::json!({
                "email_id": id,
                "from": email.sender_email,
                "subject": email.subject,
                "mailbox": email.mailbox,
            }));
        }
        if session.mode == OutputMode::Json {
            output::emit_ok(serde_json::json!({ "unlabelled": rows }))?;
        } else {
            for row in &rows {
                println!(
                    "{}\n    {}  {}",
                    row["email_id"].as_str().unwrap_or_default(),
                    row["from"].as_str().unwrap_or_default(),
                    row["subject"].as_str().unwrap_or_default(),
                );
            }
            println!("\nLabel with: junk --label <email-id>=<legit|spam|phishing|graymail>");
        }
        return Ok(());
    }

    if export_cases {
        let path = std::path::PathBuf::from("private-evals/junk/cases/real.yaml");
        let (written, missing) = golden::export_cases(&session.db, &entries, &path)?;
        if session.mode == OutputMode::Json {
            output::emit_ok(serde_json::json!({
                "path": path.display().to_string(),
                "written": written,
                "missing": missing,
            }))?;
        } else {
            println!("Exported {written} case(s) → {}", path.display());
            if missing > 0 {
                println!("{missing} label(s) pointed at messages no longer in the database.");
                println!("That is the dangling-pointer problem this export exists to prevent.");
            }
            println!();
            println!("⚠️  This file contains REAL MAIL — subjects, addresses, headers, bodies.");
            println!("    Never commit or share it. Run it with:");
            println!("      make eval-junk ARGS=\"--cases-dir private-evals/junk/cases\"");
        }
        return Ok(());
    }

    // measure
    let report = golden::measure(&session.db, &entries)?;
    if session.mode == OutputMode::Json {
        output::emit_ok(serde_json::to_value(&report).map_err(|e| AppError::InvalidInput(e.to_string()))?)?;
    } else {
        println!("Golden set: {} labelled message(s) with a stored verdict", report.total);
        println!("  by source: {:?}", report.by_source);
        println!();
        println!(
            "  TP {}   FP {}   TN {}   FN {}",
            report.true_pos, report.false_pos, report.true_neg, report.false_neg
        );
        let fmt = |v: Option<f64>| v.map(|x| format!("{x:.3}")).unwrap_or_else(|| "n/a".into());
        println!(
            "  precision {}   recall {}",
            fmt(report.precision()),
            fmt(report.recall())
        );
        println!(
            "  legit_fp_rate {}   <- the number that decides if this is usable",
            fmt(report.legit_fp_rate())
        );
        if !report.false_positive_ids.is_empty() {
            println!("\n  FALSE POSITIVES (labelled legit, detector flagged) — go look at these:");
            for id in report.false_positive_ids.iter().take(20) {
                println!("    junk --explain {id}");
            }
        }
        if !report.false_negative_ids.is_empty() {
            println!("\n  missed ({}):", report.false_negative_ids.len());
            for id in report.false_negative_ids.iter().take(10) {
                println!("    {id}");
            }
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
            style: crate::cli::RenderStyle::Json,
            quiet: true,
            log_quiet: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
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
            cc_addresses: Vec::new(),
            subject: "Confirmar reunión".to_string(),
            body: "Hola Alina".to_string(),
            body_html: None,
            provider_draft_id: None,
            attachments: None,
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
