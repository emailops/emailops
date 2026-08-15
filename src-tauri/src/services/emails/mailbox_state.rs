//! Mailbox-state writes (read/unread, delete) and their provider write-back.
//!
//! Until now these were local-DB-only: archiving or reading a message in
//! EmailOps left the user's Gmail account untouched, so the same message came
//! back unread on their phone. Providers that expose mailbox writes
//! ([`provider_supports_mailbox_writes`]) now get the change pushed.
//!
//! The two actions deliberately order their writes differently:
//!
//! - **read state** is local-first and the push is best-effort. Opening a
//!   message must work offline, and a failed push is logged, never fatal.
//! - **delete** is provider-first, mirroring [`super::folders::move_email`]. A
//!   delete that only happened locally would silently diverge from the account
//!   with no retry, so the row stays visible if the provider refuses.

use std::sync::Arc;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Email;
use crate::services::app_handle::AppHandle;
use crate::services::logger;
use crate::sync::provider::{provider_supports_mailbox_writes, EmailProvider};

use super::optimistic::LOCAL_SENT_ID_PREFIX;

/// Command entry point for "mark as read". Resolving the provider is
/// best-effort: an account that needs re-auth, or a machine that is offline,
/// must still be able to read mail.
pub async fn mark_as_read(db: &Arc<Database>, email_id: &str, app: Option<AppHandle>) -> Result<()> {
    let provider = match write_provider(db, email_id, app).await {
        Ok(provider) => provider,
        Err(e) => {
            logger::log(
                "error",
                "sync",
                format!("Read state will stay local — could not reach the mail provider: {e}"),
            );
            None
        }
    };
    mark_as_read_with_provider(db, email_id, provider.as_deref()).await
}

/// Command entry point for "delete". Unlike read state, a provider that
/// cannot be reached aborts the delete: dropping the row locally would leave
/// the message in the account with nothing left to retry the removal.
pub async fn delete_email(db: &Arc<Database>, email_id: &str, app: Option<AppHandle>) -> Result<()> {
    let provider = write_provider(db, email_id, app).await?;
    delete_email_with_provider(db, email_id, provider.as_deref()).await
}

/// The provider that should receive this email's mailbox-state changes.
/// `Ok(None)` means the account's provider has no server-side mailbox writes,
/// so the change stays local. `Err` means it does, but the provider could not
/// be built (offline, expired credentials).
async fn write_provider(
    db: &Arc<Database>,
    email_id: &str,
    app: Option<AppHandle>,
) -> Result<Option<Box<dyn EmailProvider>>> {
    let email = load_email(db, email_id)?;
    let account = db
        .get_account(&email.account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", email.account_id)))?;
    if !provider_supports_mailbox_writes(&account.provider) {
        return Ok(None);
    }
    super::build_provider(&account, app).await.map(Some)
}

/// Mark one email read locally and, when the account's provider supports it,
/// at the provider too. `provider` is `None` for providers without mailbox
/// writes, and when the provider could not be built (offline, missing
/// credentials) — the local write happens either way.
pub async fn mark_as_read_with_provider(
    db: &Arc<Database>,
    email_id: &str,
    provider: Option<&dyn EmailProvider>,
) -> Result<()> {
    let email = load_email(db, email_id)?;
    if email.is_read {
        // Opening a thread re-marks every message in it; skipping the no-op
        // keeps that from firing one provider write per message per open.
        return Ok(());
    }
    db.mark_as_read(email_id)?;

    if let Some(provider) = pushable(provider, &email) {
        if let Err(e) = provider.set_read_state(&email.id, true).await {
            // Best-effort by design: the local row is authoritative and the
            // next sync will re-read the provider's state anyway.
            logger::log(
                "error",
                "sync",
                format!("Could not mark message as read at the provider: {e}"),
            );
        }
    }
    Ok(())
}

/// Delete one email: move it to the provider's Trash first, then soft-delete
/// the local row. A provider failure aborts the whole operation so the user
/// keeps seeing a message that still exists in their account.
pub async fn delete_email_with_provider(
    db: &Arc<Database>,
    email_id: &str,
    provider: Option<&dyn EmailProvider>,
) -> Result<()> {
    let email = load_email(db, email_id)?;

    if let Some(provider) = pushable(provider, &email) {
        provider.trash_message(&email.id).await?;
    }
    db.delete_email(email_id)
}

fn load_email(db: &Arc<Database>, email_id: &str) -> Result<Email> {
    db.get_email(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("Email {email_id} not found")))
}

/// The provider to push to, or `None` when this message has no counterpart at
/// the provider yet. Locally-composed Sent rows carry a synthetic
/// `local-sent-<uuid>` id until the real copy is ingested, and sending that id
/// to the provider would 404.
fn pushable<'a>(provider: Option<&'a dyn EmailProvider>, email: &Email) -> Option<&'a dyn EmailProvider> {
    if email.id.starts_with(LOCAL_SENT_ID_PREFIX) {
        return None;
    }
    provider
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::db::{AccountScope, Database};
    use crate::models::Email;
    use crate::services::emails::mailbox_state::{
        delete_email_with_provider as delete, mark_as_read_with_provider as mark_read,
    };
    use crate::sync::provider::{FakeEmailProvider, FakeMailboxOp};

    fn test_db(account_id: &str) -> Arc<Database> {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account(account_id);
        Arc::new(db)
    }

    fn email(id: &str, account: &str, is_read: bool) -> Email {
        Email {
            id: id.to_string(),
            account_id: account.to_string(),
            thread_id: format!("t-{id}"),
            message_id: Some(format!("<{id}@example.com>")),
            subject: "s".to_string(),
            sender: "Sender".to_string(),
            sender_email: "sender@example.com".to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: vec![],
            body: "body".to_string(),
            snippet: "body".to_string(),
            timestamp: 1_000,
            is_read,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: "inbox".to_string(),
            is_sent: false,
            headers: None,
        }
    }

    fn inbox_ids(db: &Arc<Database>, account_id: &str) -> Vec<String> {
        db.get_emails(AccountScope::Account(account_id), 50, 0, None, Some("inbox"), None)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect()
    }

    // ── read state ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn mark_read_updates_the_row_and_pushes_to_the_provider() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        mark_read(&db, "m-1", Some(&provider)).await.unwrap();

        assert!(db.get_email("m-1").unwrap().unwrap().is_read);
        assert_eq!(
            provider.mailbox_ops(),
            vec![FakeMailboxOp::SetReadState {
                message_id: "m-1".to_string(),
                read: true
            }]
        );
    }

    #[tokio::test]
    async fn mark_read_without_a_provider_stays_local_only() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();

        mark_read(&db, "m-1", None).await.unwrap();

        assert!(db.get_email("m-1").unwrap().unwrap().is_read);
    }

    #[tokio::test]
    async fn mark_read_survives_a_failing_push() {
        // Offline or a provider hiccup must not stop the user from reading
        // their mail; the local row is authoritative and the error is logged.
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("network unreachable");

        mark_read(&db, "m-1", Some(&provider)).await.unwrap();

        assert!(
            db.get_email("m-1").unwrap().unwrap().is_read,
            "local read state is kept even when the push fails"
        );
    }

    #[tokio::test]
    async fn mark_read_on_an_already_read_row_skips_the_push() {
        // Opening a thread re-marks every message; without this guard each
        // open would fire one Gmail write per message, forever.
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", true)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        mark_read(&db, "m-1", Some(&provider)).await.unwrap();

        assert!(provider.mailbox_ops().is_empty(), "no write for a no-op change");
    }

    // ── delete ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_trashes_at_the_provider_then_soft_deletes_locally() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", true)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        delete(&db, "m-1", Some(&provider)).await.unwrap();

        assert_eq!(
            provider.mailbox_ops(),
            vec![FakeMailboxOp::Trash {
                message_id: "m-1".to_string()
            }]
        );
        assert!(inbox_ids(&db, "acc-1").is_empty(), "row no longer listed");
    }

    #[tokio::test]
    async fn delete_keeps_the_row_when_the_provider_refuses() {
        // A local-only delete would diverge from the account with no retry,
        // so the message must stay visible and the error reach the user.
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", true)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("network unreachable");

        let err = delete(&db, "m-1", Some(&provider)).await.unwrap_err();

        assert!(err.to_string().contains("network unreachable"), "unexpected: {err}");
        assert_eq!(
            inbox_ids(&db, "acc-1"),
            vec!["m-1".to_string()],
            "the message is still there after a failed trash"
        );
    }

    #[tokio::test]
    async fn delete_without_a_provider_soft_deletes_locally() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", true)]).unwrap();

        delete(&db, "m-1", None).await.unwrap();

        assert!(inbox_ids(&db, "acc-1").is_empty());
    }

    #[tokio::test]
    async fn locally_composed_sent_rows_are_never_pushed() {
        // `local-sent-<uuid>` ids are synthetic placeholders for a message the
        // provider has not confirmed yet — sending one to Gmail would 404.
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("local-sent-abc", "acc-1", false)])
            .unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        mark_read(&db, "local-sent-abc", Some(&provider)).await.unwrap();
        delete(&db, "local-sent-abc", Some(&provider)).await.unwrap();

        assert!(
            provider.mailbox_ops().is_empty(),
            "synthetic ids must never reach the provider"
        );
        assert!(inbox_ids(&db, "acc-1").is_empty(), "local delete still happens");
    }

    #[tokio::test]
    async fn missing_emails_are_reported_as_not_found() {
        let db = test_db("acc-1");
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        assert!(mark_read(&db, "nope", Some(&provider)).await.is_err());
        assert!(delete(&db, "nope", Some(&provider)).await.is_err());
        assert!(provider.mailbox_ops().is_empty());
    }
}
