use std::path::Path;
use std::sync::Arc;

use crate::services::app_handle::AppHandle;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Email;

use crate::sync::provider::EmailProvider;

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

    let provider = build_provider_for_account(&account, Some(app)).await?;
    redownload_email_with_provider(db, email_id, provider.as_ref()).await
}

/// [`redownload_email`] with an already-built provider, so tests can drive it
/// with a `FakeEmailProvider`.
pub async fn redownload_email_with_provider(
    db: &Arc<Database>,
    email_id: &str,
    provider: &dyn EmailProvider,
) -> Result<Email> {
    let stored = db
        .get_email(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email {} not found", email_id)))?;

    let account = db
        .get_account(&stored.account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", stored.account_id)))?;

    emit_account_log(
        "info",
        "sync",
        &account.email,
        &format!("Re-downloading email: {}", email_id),
    );

    let updated_email = refetch_email(db, &stored, &account.email, provider).await?;

    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Re-downloaded email: {}", email_id),
    );

    Ok(updated_email)
}

/// Carry over what the app knows about a message that a fresh provider parse
/// cannot reproduce. The parse reports a default mailbox (Outlook and IMAP set
/// "inbox"; the sync passes override it, as `ingest_mailbox_refs` does), no
/// sent flag, no triage and an unread state — upserting it verbatim moved Sent
/// mail into the inbox and discarded the user's triage and read state.
fn keep_local_state(stored: &Email, fresh: &mut Email) {
    fresh.account_id = stored.account_id.clone();
    fresh.mailbox = stored.mailbox.clone();
    fresh.is_sent = stored.is_sent || fresh.is_sent;
    fresh.is_read = stored.is_read;
    fresh.triage_status = stored.triage_status.clone();
}

/// Fetch `stored` again from the provider and upsert the fresh parse over it,
/// together with its attachment metadata.
async fn refetch_email(
    db: &Arc<Database>,
    stored: &Email,
    account_email: &str,
    provider: &dyn EmailProvider,
) -> Result<Email> {
    let (mut updated_email, _category, attachment_infos) = provider.get_message(&stored.id).await?;
    keep_local_state(stored, &mut updated_email);

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
                    stored.account_id.clone(),
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
                account_email,
                &format!("Failed to save attachment metadata for {}: {}", stored.id, e),
            );
        }
    }

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
        let refetched = match db.get_email(email_id) {
            Ok(Some(stored)) => refetch_email(db, &stored, &account.email, provider.as_ref()).await,
            Ok(None) => Err(AppError::NotFound(format!("Email {} not found", email_id))),
            Err(e) => Err(e),
        };
        match refetched {
            Ok(_) => success += 1,
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
