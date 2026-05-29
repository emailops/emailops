use std::sync::Arc;

use tauri::AppHandle;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::sync::provider::{EmailBody, EmailProvider};

use super::events::emit_account_log;
use super::provider::{build_provider_for_account, map_send_error};

/// Send a reply using an already-built `EmailProvider`. No `AppHandle`
/// required — suitable for integration tests via `FakeEmailProvider`.
pub async fn send_reply_with_provider(
    db: &Arc<Database>,
    email_id: &str,
    body: &EmailBody,
    from_account_id: Option<&str>,
    to_emails: Option<Vec<String>>,
    cc_emails: Option<Vec<String>>,
    provider: &dyn EmailProvider,
) -> Result<()> {
    let email = db
        .get_email(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email {} not found", email_id)))?;

    let account_id = from_account_id.unwrap_or(&email.account_id);
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    let to = to_emails.unwrap_or_else(|| vec![email.sender_email.clone()]);
    let cc = cc_emails.unwrap_or_default();

    emit_account_log(
        "info",
        "sync",
        &account.email,
        &format!("Sending reply to {}...", to.join(", ")),
    );

    provider
        .send_reply(
            &account.email,
            &to,
            &cc,
            &email.thread_id,
            email.message_id.as_deref(),
            &email.subject,
            body,
        )
        .await
        .map_err(|e| map_send_error(e, &account.email))?;

    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Reply sent to {}", to.join(", ")),
    );

    crate::services::tasks::on_reply_sent(db, &email.account_id, &email.thread_id, Some(email_id), &to);

    Ok(())
}

/// Send a reply for `email_id`, building the OAuth provider from the account's
/// stored credentials. The `AppHandle` is used for mid-send token-refresh UI
/// events; pass `None` (via `send_reply_with_provider`) in tests.
pub async fn send_reply(
    db: &Arc<Database>,
    email_id: &str,
    body: &EmailBody,
    from_account_id: Option<&str>,
    to_emails: Option<Vec<String>>,
    cc_emails: Option<Vec<String>>,
    app: AppHandle,
) -> Result<()> {
    let email = db
        .get_email(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email {} not found", email_id)))?;

    let account_id = from_account_id.unwrap_or(&email.account_id);
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    let to = to_emails.unwrap_or_else(|| vec![email.sender_email.clone()]);
    let cc = cc_emails.unwrap_or_default();

    emit_account_log(
        "info",
        "sync",
        &account.email,
        &format!("Sending reply to {}...", to.join(", ")),
    );

    let provider = build_provider_for_account(&account, Some(app))
        .await
        .map_err(|e| map_send_error(e, &account.email))?;
    provider
        .send_reply(
            &account.email,
            &to,
            &cc,
            &email.thread_id,
            email.message_id.as_deref(),
            &email.subject,
            body,
        )
        .await
        .map_err(|e| map_send_error(e, &account.email))?;

    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Reply sent to {}", to.join(", ")),
    );

    crate::services::tasks::on_reply_sent(db, &email.account_id, &email.thread_id, Some(email_id), &to);

    Ok(())
}

/// Send a new email using an already-built `EmailProvider`. No `AppHandle`
/// required — suitable for integration tests via `FakeEmailProvider`.
pub async fn send_new_email_with_provider(
    db: &Arc<Database>,
    account_id: &str,
    to_emails: Vec<String>,
    cc_emails: Vec<String>,
    subject: &str,
    body: &EmailBody,
    attachments: Vec<crate::sync::provider::EmailAttachment>,
    provider: &dyn EmailProvider,
) -> Result<()> {
    // Guard: at least one recipient required.
    if to_emails.is_empty() {
        return Err(AppError::InvalidInput(
            "At least one recipient (To) is required".to_string(),
        ));
    }
    // Guard: reject headers with embedded newlines to prevent header injection.
    if subject.contains('\n') || subject.contains('\r') {
        return Err(AppError::InvalidInput(
            "Subject must not contain newline characters".to_string(),
        ));
    }

    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    let attachment_count = attachments.len();
    let log_msg = if attachment_count > 0 {
        format!(
            "Sending email to {} ({} attachment{})...",
            to_emails.join(", "),
            attachment_count,
            if attachment_count == 1 { "" } else { "s" }
        )
    } else {
        format!("Sending email to {}...", to_emails.join(", "))
    };
    emit_account_log("info", "sync", &account.email, &log_msg);

    provider
        .send_new_email(&account.email, &to_emails, &cc_emails, subject, body, &attachments)
        .await
        .map_err(|e| map_send_error(e, &account.email))?;

    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Email sent to {}", to_emails.join(", ")),
    );
    Ok(())
}

/// Send a new email, building the OAuth provider from the account's stored
/// credentials. The `AppHandle` is used for mid-send token-refresh UI events;
/// pass `None` (via `send_new_email_with_provider`) in tests.
pub async fn send_new_email(
    db: &Arc<Database>,
    account_id: &str,
    to_emails: Vec<String>,
    cc_emails: Vec<String>,
    subject: &str,
    body: &EmailBody,
    attachments: Vec<crate::sync::provider::EmailAttachment>,
    app: AppHandle,
) -> Result<()> {
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    let attachment_count = attachments.len();
    let log_msg = if attachment_count > 0 {
        format!(
            "Sending email to {} ({} attachment{})...",
            to_emails.join(", "),
            attachment_count,
            if attachment_count == 1 { "" } else { "s" }
        )
    } else {
        format!("Sending email to {}...", to_emails.join(", "))
    };
    emit_account_log("info", "sync", &account.email, &log_msg);

    let provider = build_provider_for_account(&account, Some(app))
        .await
        .map_err(|e| map_send_error(e, &account.email))?;
    provider
        .send_new_email(&account.email, &to_emails, &cc_emails, subject, body, &attachments)
        .await
        .map_err(|e| map_send_error(e, &account.email))?;

    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Email sent to {}", to_emails.join(", ")),
    );
    Ok(())
}
