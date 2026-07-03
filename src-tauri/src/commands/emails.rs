use tauri::{AppHandle, Emitter, State};

use crate::db::Database;
use crate::models::error::AppError;
use crate::models::{Email, SyncStatus};
use crate::services;
use crate::AppState;

/// Look up an email and confirm it belongs to `account_id`. Used by every
/// command that exposes email-scoped data (body, attachments, …) so that a
/// bug or compromised frontend cannot pass a foreign account's email_id and
/// read across accounts.
///
/// Returns `AppError::NotFound` whether the email is missing OR belongs to
/// another account — the IPC surface deliberately doesn't tell the caller
/// the email exists under a different owner.
fn ensure_email_in_account(db: &Database, account_id: &str, email_id: &str) -> Result<Email, AppError> {
    let email = db
        .get_email(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email {email_id} not found")))?;
    if email.account_id != account_id {
        return Err(AppError::NotFound(format!("Email {email_id} not found")));
    }
    Ok(email)
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncProgressEvent {
    account_id: String,
    status: String,
    current: u32,
    total: u32,
    message: String,
}

fn emit_sync_error(app: &AppHandle, account_id: &str, message: &str) {
    let _ = app.emit(
        "sync-progress",
        SyncProgressEvent {
            account_id: account_id.to_string(),
            status: "error".to_string(),
            current: 0,
            total: 0,
            message: message.to_string(),
        },
    );
}

fn emit_log(_app: &AppHandle, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

#[tauri::command]
pub async fn get_emails(
    state: State<'_, AppState>,
    account_id: String,
    limit: Option<i32>,
    offset: Option<i32>,
    mailbox: Option<String>,
) -> Result<Vec<Email>, AppError> {
    services::emails::get_emails(
        &state.db,
        &account_id,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        mailbox.as_deref(),
        None,
    )
}

#[tauri::command]
pub async fn get_thread(
    state: State<'_, AppState>,
    account_id: String,
    thread_id: String,
) -> Result<Vec<Email>, AppError> {
    services::emails::get_thread(&state.db, &account_id, &thread_id)
}

#[tauri::command]
pub async fn get_email_body(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<String, AppError> {
    ensure_email_in_account(&state.db, &account_id, &email_id)?;
    state.db.get_email_body(&email_id)
}

#[tauri::command]
pub async fn mark_as_read(state: State<'_, AppState>, email_id: String) -> Result<(), AppError> {
    services::emails::mark_as_read(&state.db, &email_id)?;
    services::tasks::on_email_read(&state.db, &email_id);
    Ok(())
}

#[tauri::command]
pub async fn delete_email(state: State<'_, AppState>, email_id: String) -> Result<(), AppError> {
    // Log to memory BEFORE deletion so we can still resolve thread_id/account_id.
    services::tasks::on_archived(&state.db, &email_id);
    state.db.delete_email(&email_id)
}

#[tauri::command]
pub async fn send_reply(
    app: AppHandle,
    state: State<'_, AppState>,
    email_id: String,
    body: String,
    body_html: Option<String>,
    inline_images: Option<Vec<crate::sync::provider::EmailAttachment>>,
    from_account_id: Option<String>,
    to_emails: Option<Vec<String>>,
    cc_emails: Option<Vec<String>>,
    attachments: Option<Vec<crate::sync::provider::EmailAttachment>>,
) -> Result<(), AppError> {
    let email_body = build_email_body(body, body_html, inline_images)?;
    let sent_account_id = services::emails::send_reply(
        &state.db,
        &email_id,
        &email_body,
        from_account_id.as_deref(),
        to_emails,
        cc_emails,
        attachments.unwrap_or_default(),
        app.clone(),
    )
    .await?;

    // Refresh the sending account so the Sent copy (IMAP-appended, or filed
    // server-side by Gmail/Graph) lands in the Sent view without waiting for
    // the next periodic sync.
    enqueue_account_sync(&app, &state, sent_account_id).await;
    Ok(())
}

#[tauri::command]
pub async fn send_new_email(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    to_emails: Vec<String>,
    cc_emails: Option<Vec<String>>,
    subject: String,
    body: String,
    body_html: Option<String>,
    inline_images: Option<Vec<crate::sync::provider::EmailAttachment>>,
    attachments: Option<Vec<crate::sync::provider::EmailAttachment>>,
) -> Result<(), AppError> {
    let email_body = build_email_body(body, body_html, inline_images)?;
    let sent_account_id = services::emails::send_new_email(
        &state.db,
        &account_id,
        to_emails,
        cc_emails.unwrap_or_default(),
        &subject,
        &email_body,
        attachments.unwrap_or_default(),
        app.clone(),
    )
    .await?;

    // Refresh the sending account so the Sent copy (IMAP-appended, or filed
    // server-side by Gmail/Graph) lands in the Sent view without waiting for
    // the next periodic sync.
    enqueue_account_sync(&app, &state, sent_account_id).await;
    Ok(())
}

/// Construct an `EmailBody` from the wire payload. When `body_html` is present
/// we sanitize it server-side via `crate::services::emails::sanitize_outgoing_html`
/// — frontends are not trusted to produce safe HTML, even though the compose
/// editor only emits an allowlisted subset.
///
/// `inline_images` entries are normalized: anything passed via this parameter
/// is forced to `is_inline = true` and must carry a non-empty `content_id`,
/// otherwise the `cid:` references in the HTML body would dangle.
fn build_email_body(
    body: String,
    body_html: Option<String>,
    inline_images: Option<Vec<crate::sync::provider::EmailAttachment>>,
) -> Result<crate::sync::provider::EmailBody, AppError> {
    use crate::sync::provider::EmailBody;
    let html = body_html.and_then(|h| {
        let trimmed = h.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(crate::services::emails::sanitize_outgoing_html(trimmed))
        }
    });
    let mut inline = inline_images.unwrap_or_default();
    for att in &mut inline {
        att.is_inline = true;
        if att.content_id.as_deref().map(str::is_empty).unwrap_or(true) {
            return Err(AppError::InvalidInput("Inline image is missing contentId".to_string()));
        }
    }
    if html.is_none() && !inline.is_empty() {
        return Err(AppError::InvalidInput("Inline images require an HTML body".to_string()));
    }
    Ok(EmailBody {
        text: body,
        html,
        inline_images: inline,
        // Footer language is resolved from the user's UI preference in the send
        // service; default here keeps this builder free of DB access.
        language: crate::services::i18n::Language::default(),
        append_footer: true,
    })
}

/// Frontend-facing payload for `draft-generated`. Sent once the AI finishes
/// producing a draft; matched to its initiating request via `request_id`.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DraftGeneratedEvent {
    request_id: String,
    email_id: String,
    body: String,
    sources: Vec<services::emails::DraftSource>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DraftFailedEvent {
    request_id: String,
    email_id: String,
    error: String,
}

/// Kick off AI draft generation for `email_id`.
///
/// Returns a `request_id` immediately and runs the actual work on `ai_queue`
/// so the UI thread is not blocked. The frontend listens for the matching
/// `draft-generated` (success) or `draft-failed` (error) event to render
/// the result. Progress is emitted on `app-log` with `source = "drafts"`.
#[tauri::command]
pub async fn generate_draft(
    app: AppHandle,
    state: State<'_, AppState>,
    email_id: String,
    instructions: Option<String>,
) -> Result<String, AppError> {
    // Hard gate: respect both the master AI switch and the per-feature
    // `ai_drafts_enabled` preference so a user who disabled drafts in
    // Settings cannot still trigger a generation via the keyboard.
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    if !state.db.is_ai_drafts_enabled()? {
        return Err(AppError::InvalidInput("AI drafts are disabled".into()));
    }

    let request_id = uuid::Uuid::new_v4().to_string();
    let db = state.db.clone();
    let app_for_task = app.clone();
    let request_id_for_task = request_id.clone();
    let email_id_for_task = email_id.clone();
    let task_label = format!("draft:{}", email_id);

    state
        .ai_queue
        .submit_named(&task_label, async move {
            match services::emails::generate_draft(&db, &email_id_for_task, instructions.as_deref()).await {
                Ok(result) => {
                    if let Err(e) = app_for_task.emit(
                        "draft-generated",
                        DraftGeneratedEvent {
                            request_id: request_id_for_task.clone(),
                            email_id: email_id_for_task.clone(),
                            body: result.body,
                            sources: result.sources,
                        },
                    ) {
                        emit_log(
                            &app_for_task,
                            "error",
                            "drafts",
                            &format!("failed to emit draft-generated: {}", e),
                        );
                    }
                }
                Err(err) => {
                    let message = err.to_string();
                    emit_log(
                        &app_for_task,
                        "error",
                        "drafts",
                        &format!("draft generation failed: {}", message),
                    );
                    let _ = app_for_task.emit(
                        "draft-failed",
                        DraftFailedEvent {
                            request_id: request_id_for_task.clone(),
                            email_id: email_id_for_task.clone(),
                            error: message,
                        },
                    );
                }
            }
        })
        .await;

    Ok(request_id)
}

#[tauri::command]
pub async fn redownload_email(app: AppHandle, state: State<'_, AppState>, email_id: String) -> Result<Email, AppError> {
    services::emails::redownload_email(&state.db, &email_id, &state.app_data_dir, app).await
}

#[tauri::command]
pub async fn start_redownload_empty_emails(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), AppError> {
    let db = state.db.clone();
    let app_for_task = app.clone();
    let app_for_errors = app.clone();

    tauri::async_runtime::spawn(async move {
        if let Err(e) = services::emails::redownload_empty_emails(&db, &account_id, app_for_task).await {
            emit_log(
                &app_for_errors,
                "error",
                "sync",
                &format!("Re-download empty emails failed: {}", e),
            );
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn sync_account(app: AppHandle, state: State<'_, AppState>, account_id: String) -> Result<(), AppError> {
    services::emails::sync_account(
        &state.db,
        &account_id,
        &state.app_data_dir,
        Some(app),
        state.ai_background.clone(),
        state.sync_abort_flags.clone(),
        state.sync_locks.clone(),
    )
    .await
}

/// Manually trigger a full re-scan of one extra mailbox (Sent / Spam / Trash)
/// for the given account. Walks the entire mailbox history paginated to
/// exhaustion and inserts whatever's missing. Use to recover from sync gaps
/// left by older versions that lacked a dedicated Sent pass.
///
/// Submits the work to the per-account sync queue and returns immediately —
/// progress and completion are reported via `app-log` events.
#[tauri::command]
pub async fn start_resync_mailbox(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    mailbox: String,
) -> Result<(), AppError> {
    let extra = match mailbox.as_str() {
        "sent" => crate::sync::provider::ExtraMailbox::Sent,
        "spam" => crate::sync::provider::ExtraMailbox::Spam,
        "trash" => crate::sync::provider::ExtraMailbox::Trash,
        other => {
            return Err(AppError::SyncError(format!(
                "Unsupported mailbox for resync: {other} (expected sent / spam / trash)"
            )));
        }
    };

    let db = state.db.clone();
    let account_id_for_task = account_id.clone();
    let app_for_task = app.clone();
    let app_for_errors = app.clone();
    let task_label = format!("resync-mailbox:{}:{}", account_id, mailbox);
    let account_queue = state.sync_queue_for(&account_id);

    account_queue
        .submit_named(&task_label, async move {
            let account = match db.get_account(&account_id_for_task) {
                Ok(Some(a)) => a,
                Ok(None) => {
                    emit_log(
                        &app_for_errors,
                        "error",
                        "sync",
                        &format!("Resync mailbox: account {} not found", account_id_for_task),
                    );
                    return;
                }
                Err(e) => {
                    emit_log(
                        &app_for_errors,
                        "error",
                        "sync",
                        &format!("Resync mailbox: failed to load account: {}", e),
                    );
                    return;
                }
            };

            let provider = match services::emails::build_provider(&account, Some(app_for_task.clone())).await {
                Ok(p) => p,
                Err(e) => {
                    emit_log(
                        &app_for_errors,
                        "error",
                        "sync",
                        &format!("Resync mailbox: failed to build provider for {}: {}", account.email, e),
                    );
                    return;
                }
            };

            emit_log(
                &app_for_task,
                "info",
                "sync",
                &format!(
                    "Resyncing {} mailbox for {} — this may take a few minutes",
                    mailbox, account.email
                ),
            );

            match services::emails::resync_mailbox_full(&db, &account, extra, provider.as_ref()).await {
                Ok(inserted) => {
                    emit_log(
                        &app_for_task,
                        "success",
                        "sync",
                        &format!(
                            "Resync of {} mailbox for {} done — {} new email(s) recovered",
                            mailbox, account.email, inserted
                        ),
                    );
                }
                Err(e) => {
                    emit_log(
                        &app_for_errors,
                        "error",
                        "sync",
                        &format!("Resync of {} mailbox for {} failed: {}", mailbox, account.email, e),
                    );
                }
            }
        })
        .await;

    Ok(())
}

/// Enqueue an incremental sync of one account on its dedicated per-account
/// sync queue.
///
/// Submitting to the per-account queue keeps different accounts on independent
/// FIFOs — one slow provider can no longer block syncs of other accounts.
/// Within an account, `sync_locks` (try_lock) still guarantees only one
/// download is in-flight at any moment.
///
/// Shared by the manual `start_sync_account` command and the post-send refresh
/// (see `send_reply` / `send_new_email`), so a just-sent message — which IMAP
/// must `APPEND` to the Sent folder itself — shows up in the Sent view without
/// waiting for the next periodic sync.
async fn enqueue_account_sync(app: &AppHandle, state: &AppState, account_id: String) {
    let db = state.db.clone();
    let app_data_dir = state.app_data_dir.clone();
    let ai_background = state.ai_background.clone();
    let sync_abort_flags = state.sync_abort_flags.clone();
    let sync_locks = state.sync_locks.clone();
    let account_id_for_task = account_id.clone();
    let app_for_task = app.clone();
    let app_for_errors = app.clone();

    let task_label = format!("sync:{}", account_id);
    let account_queue = state.sync_queue_for(&account_id);
    account_queue
        .submit_named(&task_label, async move {
            if let Err(error) = services::emails::sync_account(
                &db,
                &account_id_for_task,
                &app_data_dir,
                Some(app_for_task),
                ai_background,
                sync_abort_flags,
                sync_locks,
            )
            .await
            {
                let message = error.to_string();

                if let Err(status_error) = db.upsert_sync_status(&account_id_for_task, "error", None, Some(&message)) {
                    emit_log(
                        &app_for_errors,
                        "error",
                        "sync",
                        &format!("Failed to persist sync error state: {}", status_error),
                    );
                }

                emit_sync_error(&app_for_errors, &account_id_for_task, &message);
            }
        })
        .await;
}

#[tauri::command]
pub async fn start_sync_account(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), AppError> {
    enqueue_account_sync(&app, &state, account_id).await;
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SenderSuggestion {
    pub email: String,
    pub name: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipientSuggestion {
    pub email: String,
    pub name: String,
    pub domain_match: bool,
}

#[tauri::command]
pub async fn autocomplete_recipients(
    state: State<'_, AppState>,
    account_id: String,
    prefix: String,
    context_domain: Option<String>,
    limit: Option<i32>,
) -> Result<Vec<RecipientSuggestion>, AppError> {
    let results =
        state
            .db
            .autocomplete_recipients(&account_id, &prefix, context_domain.as_deref(), limit.unwrap_or(8))?;
    Ok(results
        .into_iter()
        .map(|(email, name, domain_match)| RecipientSuggestion {
            email,
            name,
            domain_match,
        })
        .collect())
}

#[tauri::command]
pub async fn autocomplete_senders(
    state: State<'_, AppState>,
    account_id: String,
    prefix: String,
    limit: Option<i32>,
) -> Result<Vec<SenderSuggestion>, AppError> {
    let results = state
        .db
        .autocomplete_senders(&account_id, &prefix, limit.unwrap_or(8))?;
    Ok(results
        .into_iter()
        .map(|(email, name)| SenderSuggestion { email, name })
        .collect())
}

#[tauri::command]
pub async fn get_email_inbox_position(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<i32, AppError> {
    state.db.get_email_inbox_position(&account_id, &email_id)
}

#[tauri::command]
pub async fn get_email_by_id(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<Email, AppError> {
    ensure_email_in_account(&state.db, &account_id, &email_id)
}

#[tauri::command]
pub fn get_email_count(state: State<'_, AppState>, account_id: String) -> Result<i32, AppError> {
    state.db.count_emails(&account_id)
}

#[tauri::command]
pub fn get_sync_status(state: State<'_, AppState>, account_id: String) -> Result<SyncStatus, AppError> {
    state.db.get_sync_status(&account_id)
}
