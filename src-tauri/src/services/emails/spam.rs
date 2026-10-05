//! Filing one message in the provider's Spam/Junk folder, and taking it back
//! out — the moves behind "Report junk" and "Block sender".
//!
//! Provider-first, like archive: Graph and IMAP re-key a moved message, so the
//! row is only re-filed once the provider has it, under the id it has now.
//! An account whose provider has no mailbox writes, or an IMAP server with no
//! Junk folder, leaves the message where it is ([`SpamFiling::LocalOnly`]);
//! the caller has already recorded the user's judgement locally.

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Email;
use crate::sync::provider::{EmailProvider, MoveTarget};

use super::folders::refile_moved_row;
use super::optimistic::LOCAL_SENT_ID_PREFIX;

/// Where a message ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpamFiling {
    /// In the provider's Spam/Junk folder, and filed as `spam` locally.
    Filed,
    /// Left where it was: the provider cannot be told.
    LocalOnly,
}

/// File `email` as spam at the provider (`None` = an account without mailbox
/// writes) and re-file the row where the provider put it.
pub async fn file_in_spam(db: &Database, email: &Email, provider: Option<&dyn EmailProvider>) -> Result<SpamFiling> {
    if email.mailbox == "spam" {
        return Ok(SpamFiling::Filed);
    }
    let Some(provider) = provider.filter(|_| !email.id.starts_with(LOCAL_SENT_ID_PREFIX)) else {
        return Ok(SpamFiling::LocalOnly);
    };
    let (new_id, mailbox) = match provider.move_to_spam(&email.id, email.message_id.as_deref()).await {
        Ok(location) => (location.id, location.mailbox),
        // The provider lost it under this id: nothing is left in its inbox.
        Err(AppError::NotFound(_)) => (email.id.clone(), "spam".to_string()),
        Err(AppError::NoSpamFolder) => return Ok(SpamFiling::LocalOnly),
        Err(e) => return Err(e),
    };
    refile_moved_row(db, &email.id, &new_id, &mailbox)?;
    Ok(SpamFiling::Filed)
}

/// Bring a message the account keeps in Spam back to the inbox — the inverse
/// of [`file_in_spam`]. Returns the id the message has now.
pub async fn restore_from_spam(db: &Database, email: &Email, provider: Option<&dyn EmailProvider>) -> Result<String> {
    if email.mailbox != "spam" {
        return Ok(email.id.clone());
    }
    let new_id = match provider {
        None => email.id.clone(),
        Some(provider) => match provider
            .move_message(&email.id, email.message_id.as_deref(), &MoveTarget::Inbox)
            .await
        {
            Ok(Some(moved)) => moved.id,
            Ok(None) | Err(AppError::NotFound(_)) => email.id.clone(),
            Err(e) => return Err(e),
        },
    };
    refile_moved_row(db, &email.id, &new_id, "inbox")?;
    Ok(new_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::provider::{EmailCategory, FakeEmailProvider, FakeFolderOp, FakeMailboxOp};

    fn message(id: &str, mailbox: &str) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acc-1".to_string(),
            thread_id: "t-1".to_string(),
            message_id: Some(format!("<{id}@example.com>")),
            references: None,
            subject: "s".to_string(),
            sender: "News".to_string(),
            sender_email: "news@example.com".to_string(),
            recipients: vec![],
            cc: vec![],
            body: "b".to_string(),
            snippet: "b".to_string(),
            timestamp: 1,
            is_read: true,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: mailbox.to_string(),
            is_sent: false,
            is_starred: false,
            headers: None,
        }
    }

    fn setup(row: &Email) -> (Database, FakeEmailProvider) {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(std::slice::from_ref(row)).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.add_message(row.clone(), EmailCategory::Primary, vec![]);
        (db, provider)
    }

    #[tokio::test]
    async fn filing_moves_it_at_the_provider_then_locally() {
        let email = message("m1", "inbox");
        let (db, provider) = setup(&email);

        let filed = file_in_spam(&db, &email, Some(&provider)).await.unwrap();

        assert_eq!(filed, SpamFiling::Filed);
        assert_eq!(
            provider.mailbox_ops(),
            vec![FakeMailboxOp::Spam {
                message_id: "m1".to_string()
            }]
        );
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "spam");
    }

    #[tokio::test]
    async fn without_mailbox_writes_the_message_stays_put() {
        let email = message("m1", "inbox");
        let (db, _provider) = setup(&email);

        assert_eq!(file_in_spam(&db, &email, None).await.unwrap(), SpamFiling::LocalOnly);
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "inbox");
    }

    #[tokio::test]
    async fn a_provider_failure_leaves_the_row_alone_and_is_reported() {
        let email = message("m1", "inbox");
        let (db, provider) = setup(&email);
        provider.fail_mailbox_writes("offline");

        assert!(file_in_spam(&db, &email, Some(&provider)).await.is_err());
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "inbox");
    }

    #[tokio::test]
    async fn restoring_moves_it_back_to_the_inbox() {
        let email = message("m1", "spam");
        let (db, provider) = setup(&email);

        restore_from_spam(&db, &email, Some(&provider)).await.unwrap();

        assert_eq!(
            provider.folder_ops(),
            vec![FakeFolderOp::Move {
                message_id: "m1".to_string(),
                mailbox_value: "inbox".to_string()
            }]
        );
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "inbox");
    }
}
