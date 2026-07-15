use std::sync::Arc;

use tauri::AppHandle;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Account;
use crate::sync::provider::{EmailAttachment, EmailBody, EmailProvider, SentMessageMeta};

use super::events::emit_account_log;
use super::optimistic::{build_optimistic_sent_email, OptimisticSendInput};
use super::provider::{build_provider_for_account, map_send_error};

/// Language for the "Sent with EmailOps" footer: the user's UI-language
/// preference, falling back to English (the app default) when unset.
fn footer_language(db: &Arc<Database>) -> Result<crate::services::i18n::Language> {
    Ok(crate::services::i18n::resolve_ui_language(db)?.unwrap_or_default())
}

/// Insert the optimistic local Sent copy right after the provider accepted
/// the send, so the message shows in the thread and Sent views immediately
/// instead of after the next sync. Best-effort by design: the mail IS sent
/// at this point, so failures are logged, never returned.
#[allow(clippy::too_many_arguments)]
fn insert_optimistic_sent(
    db: &Arc<Database>,
    account: &Account,
    to: &[String],
    cc: &[String],
    subject: &str,
    body: &EmailBody,
    reply_to_thread_id: Option<&str>,
    meta: &SentMessageMeta,
    attachments: &[EmailAttachment],
) {
    let synthetic_uuid = uuid::Uuid::new_v4().to_string();
    let planned = build_optimistic_sent_email(&OptimisticSendInput {
        account,
        to,
        cc,
        subject,
        body,
        reply_to_thread_id,
        meta,
        synthetic_uuid: &synthetic_uuid,
        now_secs: crate::services::clock::now_secs(),
    });

    if let Err(e) = db.insert_sent_email_local(&planned.email, planned.pending_sync) {
        emit_account_log(
            "error",
            "sync",
            &account.email,
            &format!("Email was sent but could not be stored locally: {e}"),
        );
        return;
    }

    // Attachment metas from the payloads already in hand. They cascade-delete
    // with a synthetic row on reconciliation.
    if !attachments.is_empty() {
        let metas: Vec<_> = attachments
            .iter()
            .map(|att| {
                (
                    planned.email.id.clone(),
                    account.id.clone(),
                    String::new(), // no provider attachment id yet
                    att.filename.clone(),
                    att.mime_type.clone(),
                    estimated_base64_decoded_len(&att.data),
                    Some(att.data.clone()),
                )
            })
            .collect();
        if let Err(e) = db.insert_email_attachment_metas_batch(&metas) {
            emit_account_log(
                "error",
                "sync",
                &account.email,
                &format!("Sent email stored locally but its attachment metadata was not: {e}"),
            );
        }
    }
}

/// Decoded size of a base64 payload without decoding it (metadata only).
fn estimated_base64_decoded_len(data: &str) -> i64 {
    let clean_len = data.chars().filter(|c| !c.is_ascii_whitespace() && *c != '=').count();
    ((clean_len * 3) / 4) as i64
}

/// Background upgrade of a provider-keyed optimistic row (Gmail): fetch the
/// authoritative message and overwrite the local row with the real parse
/// (server timestamp, labels, provider attachment ids). Fire-and-forget —
/// the optimistic row is already good enough if this fails.
fn spawn_authoritative_refresh(
    db: Arc<Database>,
    provider: Box<dyn EmailProvider>,
    provider_message_id: String,
    account_email: String,
) {
    tauri::async_runtime::spawn(async move {
        match provider.get_message(&provider_message_id).await {
            Ok((mut email, _category, attachment_infos)) => {
                // get_message returns the provider's view; account_id is local.
                let account_id = match db.get_email(&provider_message_id) {
                    Ok(Some(existing)) => existing.account_id,
                    _ => return, // optimistic row vanished; nothing to upgrade
                };
                email.account_id = account_id.clone();
                if let Err(e) = db.insert_emails_batch(&[email]) {
                    emit_account_log(
                        "debug",
                        "sync",
                        &account_email,
                        &format!("Could not upgrade sent copy {provider_message_id}: {e}"),
                    );
                    return;
                }
                let metas: Vec<_> = attachment_infos
                    .iter()
                    .map(|info| {
                        (
                            provider_message_id.clone(),
                            account_id.clone(),
                            info.attachment_id.clone(),
                            info.filename.clone(),
                            info.mime_type.clone(),
                            info.size,
                            info.inline_data.clone(),
                        )
                    })
                    .collect();
                if !metas.is_empty() {
                    if let Err(e) = db.insert_email_attachment_metas_batch(&metas) {
                        emit_account_log(
                            "debug",
                            "sync",
                            &account_email,
                            &format!("Sent copy upgraded but attachment metadata was not: {e}"),
                        );
                    }
                }
            }
            Err(e) => {
                emit_account_log(
                    "debug",
                    "sync",
                    &account_email,
                    &format!("Could not fetch authoritative sent copy {provider_message_id}: {e}"),
                );
            }
        }
    });
}

/// Send a reply using an already-built `EmailProvider`. No `AppHandle`
/// required — suitable for integration tests via `FakeEmailProvider`.
pub async fn send_reply_with_provider(
    db: &Arc<Database>,
    email_id: &str,
    body: &EmailBody,
    from_account_id: Option<&str>,
    to_emails: Option<Vec<String>>,
    cc_emails: Option<Vec<String>>,
    attachments: Vec<crate::sync::provider::EmailAttachment>,
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
    let body = body.clone().with_language(footer_language(db)?);

    emit_account_log(
        "info",
        "sync",
        &account.email,
        &format!("Sending reply to {}...", to.join(", ")),
    );

    let meta = provider
        .send_reply(
            &account.email,
            &to,
            &cc,
            &email.thread_id,
            email.message_id.as_deref(),
            &email.subject,
            &body,
            &attachments,
        )
        .await
        .map_err(|e| map_send_error(e, &account.email))?;

    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Reply sent to {}", to.join(", ")),
    );

    insert_optimistic_sent(
        db,
        &account,
        &to,
        &cc,
        &email.subject,
        &body,
        Some(&email.thread_id),
        &meta,
        &attachments,
    );

    crate::services::tasks::on_reply_sent(db, &email.account_id, &email.thread_id, Some(email_id), &to);

    Ok(())
}

/// Send a reply for `email_id`, building the OAuth provider from the account's
/// stored credentials. The `AppHandle` is used for mid-send token-refresh UI
/// events; pass `None` (via `send_reply_with_provider`) in tests.
///
/// Returns the id of the account the reply was sent from, so the caller can
/// trigger a follow-up sync that pulls the Sent copy into the Sent view.
pub async fn send_reply(
    db: &Arc<Database>,
    email_id: &str,
    body: &EmailBody,
    from_account_id: Option<&str>,
    to_emails: Option<Vec<String>>,
    cc_emails: Option<Vec<String>>,
    attachments: Vec<crate::sync::provider::EmailAttachment>,
    app: AppHandle,
) -> Result<String> {
    let email = db
        .get_email(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email {} not found", email_id)))?;

    let account_id = from_account_id.unwrap_or(&email.account_id);
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    let to = to_emails.unwrap_or_else(|| vec![email.sender_email.clone()]);
    let cc = cc_emails.unwrap_or_default();
    let body = body.clone().with_language(footer_language(db)?);

    emit_account_log(
        "info",
        "sync",
        &account.email,
        &format!("Sending reply to {}...", to.join(", ")),
    );

    let provider = build_provider_for_account(&account, Some(app))
        .await
        .map_err(|e| map_send_error(e, &account.email))?;
    let meta = provider
        .send_reply(
            &account.email,
            &to,
            &cc,
            &email.thread_id,
            email.message_id.as_deref(),
            &email.subject,
            &body,
            &attachments,
        )
        .await
        .map_err(|e| map_send_error(e, &account.email))?;

    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Reply sent to {}", to.join(", ")),
    );

    insert_optimistic_sent(
        db,
        &account,
        &to,
        &cc,
        &email.subject,
        &body,
        Some(&email.thread_id),
        &meta,
        &attachments,
    );
    if let Some(provider_message_id) = meta.provider_message_id.clone() {
        spawn_authoritative_refresh(Arc::clone(db), provider, provider_message_id, account.email.clone());
    }

    crate::services::tasks::on_reply_sent(db, &email.account_id, &email.thread_id, Some(email_id), &to);

    Ok(account_id.to_string())
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

    let body = body.clone().with_language(footer_language(db)?);
    let meta = provider
        .send_new_email(&account.email, &to_emails, &cc_emails, subject, &body, &attachments)
        .await
        .map_err(|e| map_send_error(e, &account.email))?;

    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Email sent to {}", to_emails.join(", ")),
    );

    insert_optimistic_sent(
        db,
        &account,
        &to_emails,
        &cc_emails,
        subject,
        &body,
        None,
        &meta,
        &attachments,
    );
    Ok(())
}

/// Send a new email, building the OAuth provider from the account's stored
/// credentials. The `AppHandle` is used for mid-send token-refresh UI events;
/// pass `None` (via `send_new_email_with_provider`) in tests.
///
/// Returns the id of the sending account so the caller can trigger a follow-up
/// sync that pulls the Sent copy into the Sent view.
pub async fn send_new_email(
    db: &Arc<Database>,
    account_id: &str,
    to_emails: Vec<String>,
    cc_emails: Vec<String>,
    subject: &str,
    body: &EmailBody,
    attachments: Vec<crate::sync::provider::EmailAttachment>,
    app: AppHandle,
) -> Result<String> {
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
    let body = body.clone().with_language(footer_language(db)?);
    let meta = provider
        .send_new_email(&account.email, &to_emails, &cc_emails, subject, &body, &attachments)
        .await
        .map_err(|e| map_send_error(e, &account.email))?;

    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Email sent to {}", to_emails.join(", ")),
    );

    insert_optimistic_sent(
        db,
        &account,
        &to_emails,
        &cc_emails,
        subject,
        &body,
        None,
        &meta,
        &attachments,
    );
    if let Some(provider_message_id) = meta.provider_message_id.clone() {
        spawn_authoritative_refresh(Arc::clone(db), provider, provider_message_id, account.email.clone());
    }
    Ok(account_id.to_string())
}
