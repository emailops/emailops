use std::sync::Arc;

use base64::Engine;
use tauri::{AppHandle, State};

use crate::db::Database;
use crate::models::error::AppError;
use crate::models::{Attachment, AttachmentRule, EmailAttachmentMeta};
use crate::services;
use crate::AppState;

/// Load an attachment and confirm it belongs to `account_id`. Returns
/// `NotFound` when the attachment is missing OR owned by a different
/// account, so the IPC surface never reveals existence across accounts.
fn ensure_attachment_in_account(
    db: &Arc<Database>,
    account_id: &str,
    attachment_id: &str,
) -> Result<crate::models::Attachment, AppError> {
    let att = services::attachments::get_attachment(db, attachment_id)?
        .ok_or_else(|| AppError::NotFound(format!("Attachment {attachment_id} not found")))?;
    if att.account_id != account_id {
        return Err(AppError::NotFound(format!("Attachment {attachment_id} not found")));
    }
    Ok(att)
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
fn ensure_email_in_account(db: &Arc<Database>, account_id: &str, email_id: &str) -> Result<(), AppError> {
    let email = db
        .get_email(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email {email_id} not found")))?;
    if email.account_id != account_id {
        return Err(AppError::NotFound(format!("Email {email_id} not found")));
    }
    Ok(())
}

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
    rule_id: String,
    name: String,
    sender_email_pattern: Option<String>,
    subject_pattern: Option<String>,
    filename_pattern: Option<String>,
    tags: Vec<String>,
    enabled: bool,
) -> Result<AttachmentRule, AppError> {
    services::attachments::update_rule(
        &state.db,
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
    services::attachments::delete_rule(&state.db, &rule_id, &account_id, &state.app_data_dir)
}

#[tauri::command]
pub async fn count_attachments_for_rule(state: State<'_, AppState>, rule_id: String) -> Result<i32, AppError> {
    state.db.count_attachments_for_rule(&rule_id)
}

#[tauri::command]
pub async fn list_attachment_rules(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<AttachmentRule>, AppError> {
    services::attachments::list_rules(&state.db, &account_id)
}

#[tauri::command]
pub async fn get_attachments(
    state: State<'_, AppState>,
    account_id: String,
    tag: Option<String>,
    limit: Option<i32>,
    offset: Option<i32>,
) -> Result<Vec<Attachment>, AppError> {
    services::attachments::get_attachments(
        &state.db,
        &account_id,
        tag.as_deref(),
        limit.unwrap_or(50),
        offset.unwrap_or(0),
    )
}

#[tauri::command]
pub async fn count_attachments(
    state: State<'_, AppState>,
    account_id: String,
    tag: Option<String>,
) -> Result<i32, AppError> {
    services::attachments::count_attachments(&state.db, &account_id, tag.as_deref())
}

#[tauri::command]
pub async fn get_attachments_for_email(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<Vec<Attachment>, AppError> {
    ensure_email_in_account(&state.db, &account_id, &email_id)?;
    state.db.get_attachments_for_email(&email_id)
}

#[tauri::command]
pub async fn get_attachment(
    state: State<'_, AppState>,
    account_id: String,
    attachment_id: String,
) -> Result<Attachment, AppError> {
    ensure_attachment_in_account(&state.db, &account_id, &attachment_id)
}

#[tauri::command]
pub async fn get_attachment_tags(state: State<'_, AppState>, account_id: String) -> Result<Vec<String>, AppError> {
    services::attachments::get_tags(&state.db, &account_id)
}

#[tauri::command]
pub async fn get_attachment_file_path(
    state: State<'_, AppState>,
    account_id: String,
    attachment_id: String,
) -> Result<String, AppError> {
    let attachment = ensure_attachment_in_account(&state.db, &account_id, &attachment_id)?;
    let abs_path = services::attachments::safe_attachment_path(&state.app_data_dir, &attachment.file_path)?;
    Ok(abs_path.to_string_lossy().to_string())
}

/// Copy selected attachments to the system Downloads folder.
/// Returns the destination folder path.
#[tauri::command]
pub async fn bulk_download_attachments(
    state: State<'_, AppState>,
    account_id: String,
    attachment_ids: Vec<String>,
) -> Result<String, AppError> {
    let downloads_dir =
        dirs::download_dir().ok_or_else(|| AppError::IoError("Could not determine Downloads folder".to_string()))?;

    // Build a rule name lookup for prefixing filenames
    let mut rule_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let mut copied = 0u32;
    for id in &attachment_ids {
        let attachment = ensure_attachment_in_account(&state.db, &account_id, id)?;
        // Treat a missing file as "skip this row" but still surface a path
        // escape as a hard error — that signals a tampered row, not a stale
        // download.
        let src = match services::attachments::safe_attachment_path(&state.app_data_dir, &attachment.file_path) {
            Ok(p) => p,
            Err(AppError::NotFound(_)) => continue,
            Err(e) => return Err(e),
        };

        // Get rule name for prefix (cached)
        let rule_prefix = rule_names
            .entry(attachment.rule_id.clone())
            .or_insert_with(|| {
                state
                    .db
                    .get_attachment_rule(&attachment.rule_id)
                    .ok()
                    .flatten()
                    .map(|r| r.name.replace(' ', "_"))
                    .unwrap_or_default()
            })
            .clone();

        // Build prefixed filename: RuleName_original.pdf
        let download_name = if rule_prefix.is_empty() {
            attachment.filename.clone()
        } else {
            format!("{}_{}", rule_prefix, attachment.filename)
        };

        // Build destination path, dedup with (1), (2), etc.
        let mut dest = downloads_dir.join(&download_name);
        if dest.exists() {
            let stem = std::path::Path::new(&download_name)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let ext = std::path::Path::new(&download_name)
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            let mut n = 1u32;
            loop {
                dest = downloads_dir.join(format!("{} ({}){}", stem, n, ext));
                if !dest.exists() {
                    break;
                }
                n += 1;
            }
        }

        std::fs::copy(&src, &dest)
            .map_err(|e| AppError::IoError(format!("Failed to copy {}: {}", download_name, e)))?;
        copied += 1;
    }

    if copied == 0 {
        return Err(AppError::IoError("No files were copied".to_string()));
    }

    Ok(downloads_dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn apply_rule_retroactively(
    app: AppHandle,
    state: State<'_, AppState>,
    rule_id: String,
    account_id: String,
) -> Result<u32, AppError> {
    services::attachments::apply_rule_retroactively(&state.db, &rule_id, &account_id, &state.app_data_dir, Some(&app))
        .await
}

#[tauri::command]
pub async fn get_attachment_data(
    state: State<'_, AppState>,
    account_id: String,
    attachment_id: String,
) -> Result<String, AppError> {
    let attachment = ensure_attachment_in_account(&state.db, &account_id, &attachment_id)?;
    let abs_path = services::attachments::safe_attachment_path(&state.app_data_dir, &attachment.file_path)?;
    let bytes =
        std::fs::read(&abs_path).map_err(|e| AppError::IoError(format!("Failed to read attachment file: {}", e)))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(b64)
}

#[tauri::command]
pub async fn open_attachment_externally(
    state: State<'_, AppState>,
    account_id: String,
    attachment_id: String,
) -> Result<(), AppError> {
    let attachment = ensure_attachment_in_account(&state.db, &account_id, &attachment_id)?;
    let abs_path = services::attachments::safe_attachment_path(&state.app_data_dir, &attachment.file_path)?;
    open::that(&abs_path).map_err(|e| AppError::IoError(format!("Failed to open file: {}", e)))
}

/// Return all attachment metadata for an email (including non-rule-matched attachments).
#[tauri::command]
pub async fn get_email_attachment_metas(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<Vec<EmailAttachmentMeta>, AppError> {
    ensure_email_in_account(&state.db, &account_id, &email_id)?;
    state.db.get_email_attachment_metas(&email_id)
}

/// Fetch attachment bytes from the provider and return them as a base64 string.
/// The frontend uses this for on-demand downloads when the file is not cached locally.
#[tauri::command]
pub async fn fetch_email_attachment_bytes(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
    provider_attachment_id: String,
) -> Result<String, AppError> {
    use crate::services::emails::build_provider;

    // Verify the email belongs to this account before exposing any of its
    // attachment data — both the cached inline path and the provider fetch.
    ensure_email_in_account(&state.db, &account_id, &email_id)?;

    // IMAP attachments are stored inline in the DB (no provider fetch needed).
    // When provider_attachment_id is empty, look up the inline_data by email+filename.
    // The filename is encoded in provider_attachment_id for IMAP as "INLINE::<filename>".
    if let Some(filename) = provider_attachment_id.strip_prefix("INLINE::") {
        let inline = state
            .db
            .get_attachment_inline_data(&email_id, filename)?
            .ok_or_else(|| AppError::NotFound(format!("Inline data not found for {filename}")))?;
        return Ok(inline);
    }

    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;
    let provider = build_provider(&account, Some(app)).await?;
    let bytes = provider
        .fetch_attachment_bytes(&email_id, &provider_attachment_id)
        .await?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// Open a locally-cached attachment by its meta ID using the OS default application.
#[tauri::command]
pub async fn open_email_attachment_meta(
    state: State<'_, AppState>,
    account_id: String,
    meta_id: String,
) -> Result<(), AppError> {
    let meta = ensure_meta_in_account(&state.db, &account_id, &meta_id)?;
    let file_path = meta
        .file_path
        .ok_or_else(|| AppError::InvalidInput("Attachment has not been downloaded yet".to_string()))?;
    let abs_path = services::attachments::safe_attachment_path(&state.app_data_dir, &file_path)?;
    open::that(&abs_path).map_err(|e| AppError::IoError(format!("Failed to open file: {}", e)))
}
