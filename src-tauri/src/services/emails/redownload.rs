use std::path::Path;
use std::sync::Arc;

use tauri::AppHandle;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Email;

use super::events::emit_account_log;
use super::provider::build_provider_for_account;

pub async fn redownload_email(
    db: &Arc<Database>,
    email_id: &str,
    _app_data_dir: &Path,
    app: AppHandle,
) -> Result<Email> {
    let email = db
        .get_email(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email {} not found", email_id)))?;

    let account = db
        .get_account(&email.account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", email.account_id)))?;

    emit_account_log(
        "info",
        "sync",
        &account.email,
        &format!("Re-downloading email: {}", email_id),
    );

    let provider = build_provider_for_account(&account, Some(app)).await?;
    let (mut updated_email, _category, attachment_infos) = provider.get_message(email_id).await?;
    let account_email_log = account.email.clone();
    updated_email.account_id = account.id.clone();

    db.insert_email(&updated_email)?;

    // Persist attachment metadata. Without this, re-downloading an email that
    // gained attachments (or whose attachments were missed by an earlier sync
    // bug) leaves `email_attachment_meta` empty and the UI shows no attachments.
    if !attachment_infos.is_empty() {
        let metas: Vec<_> = attachment_infos
            .iter()
            .map(|info| {
                (
                    updated_email.id.clone(),
                    account.id.clone(),
                    info.attachment_id.clone(),
                    info.filename.clone(),
                    info.mime_type.clone(),
                    info.size,
                    info.inline_data.clone(),
                )
            })
            .collect();
        if let Err(e) = db.insert_email_attachment_metas_batch(&metas) {
            emit_account_log(
                "error",
                "sync",
                &account_email_log,
                &format!("Failed to save attachment metadata for {}: {}", email_id, e),
            );
        }
    }

    emit_account_log(
        "success",
        "sync",
        &account_email_log,
        &format!("Re-downloaded email: {}", email_id),
    );

    Ok(updated_email)
}

/// Find all emails with an empty body for `account_id` and re-download them from the provider.
/// Emits `app-log` events for progress. Designed to run in a background task.
pub async fn redownload_empty_emails(db: &Arc<Database>, account_id: &str, app: AppHandle) -> Result<()> {
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    let empty_ids = db.get_emails_with_empty_body(account_id)?;
    let total = empty_ids.len();

    if total == 0 {
        emit_account_log(
            "info",
            "sync",
            &account.email,
            "No empty emails found — inbox is complete",
        );
        return Ok(());
    }

    emit_account_log(
        "info",
        "sync",
        &account.email,
        &format!("Found {} emails with empty body — re-downloading...", total),
    );

    let provider = build_provider_for_account(&account, Some(app)).await?;
    let mut success = 0usize;
    let mut failed = 0usize;

    for (i, email_id) in empty_ids.iter().enumerate() {
        match provider.get_message(email_id).await {
            Ok((mut updated_email, _category, attachment_infos)) => {
                updated_email.account_id = account.id.clone();
                if let Err(e) = db.insert_email(&updated_email) {
                    emit_account_log(
                        "error",
                        "sync",
                        &account.email,
                        &format!("Failed to save email {}: {}", email_id, e),
                    );
                    failed += 1;
                } else {
                    if !attachment_infos.is_empty() {
                        let metas: Vec<_> = attachment_infos
                            .iter()
                            .map(|info| {
                                (
                                    updated_email.id.clone(),
                                    account.id.clone(),
                                    info.attachment_id.clone(),
                                    info.filename.clone(),
                                    info.mime_type.clone(),
                                    info.size,
                                    info.inline_data.clone(),
                                )
                            })
                            .collect();
                        if let Err(e) = db.insert_email_attachment_metas_batch(&metas) {
                            emit_account_log(
                                "error",
                                "sync",
                                &account.email,
                                &format!("Failed to save attachment metadata for {}: {}", email_id, e),
                            );
                        }
                    }
                    success += 1;
                }
            }
            Err(e) => {
                emit_account_log(
                    "error",
                    "sync",
                    &account.email,
                    &format!("Failed to re-download email {}: {}", email_id, e),
                );
                failed += 1;
            }
        }

        if (i + 1) % 10 == 0 || i + 1 == total {
            emit_account_log(
                "debug",
                "sync",
                &account.email,
                &format!("Re-download progress: {}/{}", i + 1, total),
            );
        }
    }

    if failed == 0 {
        emit_account_log(
            "success",
            "sync",
            &account.email,
            &format!("Re-downloaded {} empty emails successfully", success),
        );
    } else {
        emit_account_log(
            "info",
            "sync",
            &account.email,
            &format!("Re-download complete: {} succeeded, {} failed", success, failed),
        );
    }

    Ok(())
}
