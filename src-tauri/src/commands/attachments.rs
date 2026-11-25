use std::sync::Arc;

use base64::Engine;
use tauri::{AppHandle, State};

use crate::db::{AccountScope, Database};
use crate::models::error::AppError;
use crate::models::{
    Attachment, AttachmentRule, AttachmentRuleSuggestion, AttachmentRuleSuggestionStatus, BulkDownloadSummary,
    EmailAttachmentMeta,
};
use crate::services;
use crate::services::ownership::attachment_in_account;
use crate::AppState;

/// The attachments view and its rules modal read one account, or every
/// enabled account when the frontend sends no id ("All accounts").
fn scope_of(account_id: Option<&str>) -> AccountScope<'_> {
    match account_id {
        Some(id) => AccountScope::Account(id),
        None => AccountScope::AllEnabled,
    }
}

/// Same idea for the email-attachment-meta table (covers both rule-matched
/// and metadata-only attachments).
fn ensure_meta_in_account(
    db: &Arc<Database>,
    account_id: &str,
    meta_id: &str,
) -> Result<crate::models::EmailAttachmentMeta, AppError> {
    let meta = db
        .get_email_attachment_metas_by_id(meta_id)?
        .ok_or_else(|| AppError::NotFound(format!("Attachment meta {meta_id} not found")))?;
    if meta.account_id != account_id {
        return Err(AppError::NotFound(format!("Attachment meta {meta_id} not found")));
    }
    Ok(meta)
}

/// Confirm an email belongs to `account_id`. Mirrors the helper in
/// `commands::emails` so we can scope email-keyed attachment lookups too.

#[tauri::command]
pub async fn create_attachment_rule(
    state: State<'_, AppState>,
    account_id: String,
    name: String,
    sender_email_pattern: Option<String>,
    subject_pattern: Option<String>,
    filename_pattern: Option<String>,
    tags: Vec<String>,
) -> Result<AttachmentRule, AppError> {
    services::attachments::create_rule(
        &state.db,
        &account_id,
        &name,
        sender_email_pattern.as_deref(),
        subject_pattern.as_deref(),
        filename_pattern.as_deref(),
        tags,
    )
}

#[tauri::command]
pub async fn update_attachment_rule(
    state: State<'_, AppState>,
    account_id: String,
    rule_id: String,
    name: String,
    sender_email_pattern: Option<String>,
    subject_pattern: Option<String>,
    filename_pattern: Option<String>,
    tags: Vec<String>,
    enabled: bool,
) -> Result<AttachmentRule, AppError> {
    // A scan still collecting for the old patterns would keep files the edit
    // is about to drop.
    state.rule_applies.cancel(&rule_id);
    services::attachments::update_rule(
        &state.db,
        &account_id,
        &rule_id,
        &name,
        sender_email_pattern.as_deref(),
        subject_pattern.as_deref(),
        filename_pattern.as_deref(),
        tags,
        enabled,
        &state.app_data_dir,
    )
}

#[tauri::command]
pub async fn delete_attachment_rule(
    state: State<'_, AppState>,
    rule_id: String,
    account_id: String,
) -> Result<(), AppError> {
    state.rule_applies.cancel(&rule_id);
    services::attachments::delete_rule(&state.db, &rule_id, &account_id, &state.app_data_dir)
}

#[tauri::command]
pub async fn count_attachments_for_rule(
    state: State<'_, AppState>,
    account_id: String,
    rule_id: String,
) -> Result<i32, AppError> {
    services::attachments::count_rule_attachments(&state.db, &account_id, &rule_id)
}

// The rules modal reads one account, or every enabled account when the
// frontend sends no id ("All accounts"); each rule and suggestion carries its
// account, and the commands that change one take that account.

#[tauri::command]
pub async fn list_attachment_rules(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<AttachmentRule>, AppError> {
    services::attachments::for_each_account(&state.db, scope_of(account_id.as_deref()), |id| {
        services::attachments::list_rules(&state.db, id)
    })
}

#[tauri::command]
pub async fn list_attachment_rule_suggestions(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<AttachmentRuleSuggestion>, AppError> {
    services::attachments::for_each_account(&state.db, scope_of(account_id.as_deref()), |id| {
        services::attachment_suggestions::list_suggestions(&state.db, id)
    })
}

#[tauri::command]
pub async fn refresh_attachment_rule_suggestions(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<AttachmentRuleSuggestion>, AppError> {
    services::attachments::for_each_account(&state.db, scope_of(account_id.as_deref()), |id| {
        services::attachment_suggestions::refresh_suggestions(&state.db, id)
    })
}

#[tauri::command]
pub async fn dismiss_attachment_rule_suggestion(
    state: State<'_, AppState>,
    account_id: String,
    suggestion_id: String,
) -> Result<(), AppError> {
    services::attachment_suggestions::set_suggestion_status(
        &state.db,
        &account_id,
        &suggestion_id,
        AttachmentRuleSuggestionStatus::Dismissed,
    )
}

#[tauri::command]
pub async fn list_dismissed_attachment_rule_suggestions(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<AttachmentRuleSuggestion>, AppError> {
    services::attachments::for_each_account(&state.db, scope_of(account_id.as_deref()), |id| {
        services::attachment_suggestions::list_dismissed_suggestions(&state.db, id)
    })
}

/// Undo a dismissal; returns the pending suggestions after re-mining.
#[tauri::command]
pub async fn restore_attachment_rule_suggestion(
    state: State<'_, AppState>,
    account_id: String,
    suggestion_id: String,
) -> Result<Vec<AttachmentRuleSuggestion>, AppError> {
    services::attachment_suggestions::restore_suggestion(&state.db, &account_id, &suggestion_id)
}

#[tauri::command]
pub async fn accept_attachment_rule_suggestion(
    state: State<'_, AppState>,
    account_id: String,
    suggestion_id: String,
) -> Result<(), AppError> {
    services::attachment_suggestions::set_suggestion_status(
        &state.db,
        &account_id,
        &suggestion_id,
        AttachmentRuleSuggestionStatus::Accepted,
    )
}

#[tauri::command]
pub async fn get_attachments(
    state: State<'_, AppState>,
    account_id: Option<String>,
    tag: Option<String>,
    limit: Option<i32>,
    offset: Option<i32>,
) -> Result<Vec<Attachment>, AppError> {
    services::attachments::get_attachments(
        &state.db,
        scope_of(account_id.as_deref()),
        tag.as_deref(),
        limit.unwrap_or(50),
        offset.unwrap_or(0),
    )
}

#[tauri::command]
pub async fn count_attachments(
    state: State<'_, AppState>,
    account_id: Option<String>,
    tag: Option<String>,
) -> Result<i32, AppError> {
    services::attachments::count_attachments(&state.db, scope_of(account_id.as_deref()), tag.as_deref())
}

/// Every attachment id the list shows for this account (or all enabled
/// accounts) and tag, so "select all" covers rows not loaded yet.
#[tauri::command]
pub async fn list_attachment_ids(
    state: State<'_, AppState>,
    account_id: Option<String>,
    tag: Option<String>,
) -> Result<Vec<String>, AppError> {
    services::attachments::list_attachment_ids(&state.db, scope_of(account_id.as_deref()), tag.as_deref())
}

#[tauri::command]
pub async fn get_attachments_for_email(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<Vec<Attachment>, AppError> {
    services::ownership::email_in_account(&state.db, &account_id, &email_id)?;
    state.db.get_attachments_for_email(&email_id)
}

#[tauri::command]
pub async fn get_attachment(
    state: State<'_, AppState>,
    account_id: String,
    attachment_id: String,
) -> Result<Attachment, AppError> {
    attachment_in_account(&state.db, &account_id, &attachment_id)
}

#[tauri::command]
pub async fn get_attachment_tags(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<String>, AppError> {
    services::attachments::get_tags(&state.db, scope_of(account_id.as_deref()))
}

#[tauri::command]
pub async fn get_attachment_tag_vocabulary(state: State<'_, AppState>) -> Result<Vec<String>, AppError> {
    services::attachments::get_tag_vocabulary(&state.db)
}

#[tauri::command]
pub async fn get_attachment_file_path(
    state: State<'_, AppState>,
    account_id: String,
    attachment_id: String,
) -> Result<String, AppError> {
    let attachment = attachment_in_account(&state.db, &account_id, &attachment_id)?;
    let abs_path = services::attachments::safe_attachment_path(&state.app_data_dir, &attachment.file_path)?;
    Ok(abs_path.to_string_lossy().to_string())
}

/// Copy selected attachments, of one account or of every enabled account, to
/// the system Downloads folder. Returns what was copied and what was skipped.
#[tauri::command]
pub async fn bulk_download_attachments(
    state: State<'_, AppState>,
    account_id: Option<String>,
    attachment_ids: Vec<String>,
) -> Result<BulkDownloadSummary, AppError> {
    let downloads_dir =
        dirs::download_dir().ok_or_else(|| AppError::IoError("Could not determine Downloads folder".to_string()))?;
    let db = Arc::clone(&state.db);
    let app_data_dir = state.app_data_dir.clone();
    // File copies block; keep them off the async workers.
    tauri::async_runtime::spawn_blocking(move || {
        services::attachments::bulk_download(
            &db,
            &app_data_dir,
            &downloads_dir,
            scope_of(account_id.as_deref()),
            &attachment_ids,
        )
    })
    .await
    .map_err(|e| AppError::IoError(format!("Downloading attachments failed: {e}")))?
}

/// Save raw attachment bytes (base64, standard or URL-safe alphabet) into the
/// user's Downloads folder under a sanitized, collision-free name. Returns
/// the absolute path of the saved file so the frontend can offer to reveal it.
#[tauri::command]
pub async fn save_attachment_to_downloads(filename: String, data_base64: String) -> Result<String, AppError> {
    let downloads_dir =
        dirs::download_dir().ok_or_else(|| AppError::IoError("Could not determine Downloads folder".to_string()))?;
    let bytes = services::attachments::decode_inline_base64(&data_base64)?;
    let path = services::attachments::save_bytes_to_downloads(&downloads_dir, &filename, &bytes)?;
    Ok(path.to_string_lossy().to_string())
}

/// Reveal a previously downloaded file (or the Downloads folder itself) in
/// the OS file manager. Paths outside the Downloads folder are rejected.
#[tauri::command]
pub async fn reveal_in_finder(path: String) -> Result<(), AppError> {
    let downloads_dir =
        dirs::download_dir().ok_or_else(|| AppError::IoError("Could not determine Downloads folder".to_string()))?;
    let target = services::attachments::validate_reveal_path(&downloads_dir, std::path::Path::new(&path))?;
    services::attachments::reveal_in_file_manager(&target)
}

/// Queue applying a rule to the mail already stored and return at once. The
/// scan reports `attachment-rule-apply-progress` and, when it ends,
/// `attachment-rule-apply-finished` — both tagged with the caller's `run_id`.
/// Starting it cancels the apply of the same rule that is still running.
#[tauri::command]
pub async fn apply_rule_retroactively(
    app: AppHandle,
    state: State<'_, AppState>,
    rule_id: String,
    account_id: String,
    run_id: String,
) -> Result<(), AppError> {
    use crate::services::background_tasks::BackgroundTask;
    use tauri::Emitter;

    let core = state.core();
    let ticket = core.rule_applies.begin(&rule_id);
    let task = BackgroundTask::ApplyAttachmentRule {
        rule_id: rule_id.clone(),
        run_id: run_id.clone(),
    };
    state
        .dispatcher
        .dispatch(
            task,
            Box::new(move || {
                Box::pin(async move {
                    let emit = |name: &'static str, payload: serde_json::Value| {
                        if let Err(e) = app.emit(name, payload) {
                            eprintln!("[attachments] could not emit {name}: {e}");
                        }
                    };
                    services::attachments::run_rule_apply(
                        &core.db,
                        &rule_id,
                        &account_id,
                        &run_id,
                        &core.app_data_dir,
                        Some(&app),
                        &ticket,
                        &emit,
                    )
                    .await;
                })
            }),
        )
        .await;
    Ok(())
}

#[tauri::command]
pub async fn get_attachment_data(
    state: State<'_, AppState>,
    account_id: String,
    attachment_id: String,
) -> Result<String, AppError> {
    let attachment = attachment_in_account(&state.db, &account_id, &attachment_id)?;
    let abs_path = services::attachments::safe_attachment_path(&state.app_data_dir, &attachment.file_path)?;
    let bytes =
        std::fs::read(&abs_path).map_err(|e| AppError::IoError(format!("Failed to read attachment file: {}", e)))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(b64)
}

/// Open a rule-collected attachment with the OS default application.
/// `confirmed` is set by the frontend only after the user accepted the
/// dangerous-type dialog; without it such a file is refused with
/// `attachment_confirmation_required`.
#[tauri::command]
pub async fn open_attachment_externally(
    state: State<'_, AppState>,
    account_id: String,
    attachment_id: String,
    confirmed: bool,
) -> Result<(), AppError> {
    let attachment = attachment_in_account(&state.db, &account_id, &attachment_id)?;
    let abs_path = services::attachments::safe_attachment_path(&state.app_data_dir, &attachment.file_path)?;
    services::attachment_safety::open_attachment_file(
        &abs_path,
        &attachment.filename,
        Some(&attachment.mime_type),
        confirmed,
        &|path| open::that(path),
    )
}

/// Return all attachment metadata for an email (including non-rule-matched attachments).
#[tauri::command]
pub async fn get_email_attachment_metas(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<Vec<EmailAttachmentMeta>, AppError> {
    services::ownership::email_in_account(&state.db, &account_id, &email_id)?;
    state.db.get_email_attachment_metas(&email_id)
}

/// Re-fetch a single message from its provider and re-extract its attachment
/// metadata into the DB. Repairs emails whose attachments were missed at sync
/// time: incremental sync skips already-stored messages, so a normal re-sync
/// never revisits them and the gap is otherwise permanent. Returns the email's
/// attachment metas after the re-extract so the UI can refresh in place.
#[tauri::command]
pub async fn reextract_email_attachments(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<Vec<EmailAttachmentMeta>, AppError> {
    services::ownership::email_in_account(&state.db, &account_id, &email_id)?;
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {account_id} not found")))?;
    services::attachments::reextract_email_attachments(&state.db, &account, &email_id, Some(app)).await?;
    state.db.get_email_attachment_metas(&email_id)
}

/// The bytes of one attachment, as standard base64, for the viewer, downloads
/// and forwards. Read by the attachment's row, from its file, its stored bytes
/// or the provider: a row that holds its bytes may have no provider id.
#[tauri::command]
pub async fn fetch_email_attachment_bytes(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    meta_id: String,
) -> Result<String, AppError> {
    let meta = ensure_meta_in_account(&state.db, &account_id, &meta_id)?;
    let bytes = services::attachments::fetch_meta_bytes(&state.db, &meta, Some(app)).await?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// Open a locally-cached attachment by its meta ID using the OS default
/// application. `confirmed` as in [`open_attachment_externally`].
#[tauri::command]
pub async fn open_email_attachment_meta(
    state: State<'_, AppState>,
    account_id: String,
    meta_id: String,
    confirmed: bool,
) -> Result<(), AppError> {
    let meta = ensure_meta_in_account(&state.db, &account_id, &meta_id)?;
    let file_path = meta
        .file_path
        .ok_or_else(|| AppError::InvalidInput("Attachment has not been downloaded yet".to_string()))?;
    let abs_path = services::attachments::safe_attachment_path(&state.app_data_dir, &file_path)?;
    services::attachment_safety::open_attachment_file(
        &abs_path,
        &meta.filename,
        Some(&meta.mime_type),
        confirmed,
        &|path| open::that(path),
    )
}
