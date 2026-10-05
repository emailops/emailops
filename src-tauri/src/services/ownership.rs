//! Account ownership checks for records the UI names by id.
//!
//! Every account in the database belongs to the one local user, so these are
//! not a barrier between people. They keep a command from acting on a record
//! of a different account than the one the UI says it is working in: a stale
//! id, a mixed-up selection or a compromised page cannot read, edit, delete
//! or send through another account's record by naming its id. A record that
//! exists under another account is reported exactly like one that does not
//! exist, so the error never confirms that the id is valid elsewhere.

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::outbox::OutboxEntry;
use crate::models::{AttachmentRule, Draft, Email};

fn not_found(kind: &str, id: &str) -> AppError {
    AppError::NotFound(format!("{kind} {id} not found"))
}

/// Succeeds when `owner` is `account_id`; `NotFound` when the record is
/// missing (`None`) or belongs to another account.
fn owned_by(owner: Option<String>, account_id: &str, kind: &str, id: &str) -> Result<()> {
    match owner {
        Some(owner) if owner == account_id => Ok(()),
        _ => Err(not_found(kind, id)),
    }
}

pub fn email_in_account(db: &Database, account_id: &str, email_id: &str) -> Result<Email> {
    let email = db.get_email(email_id)?.ok_or_else(|| not_found("Email", email_id))?;
    owned_by(Some(email.account_id.clone()), account_id, "Email", email_id)?;
    Ok(email)
}

pub fn draft_in_account(db: &Database, account_id: &str, draft_id: &str) -> Result<Draft> {
    let draft = db.get_draft(draft_id)?.ok_or_else(|| not_found("Draft", draft_id))?;
    owned_by(Some(draft.account_id.clone()), account_id, "Draft", draft_id)?;
    Ok(draft)
}

pub fn attachment_rule_in_account(db: &Database, account_id: &str, rule_id: &str) -> Result<AttachmentRule> {
    let rule = db
        .get_attachment_rule(rule_id)?
        .ok_or_else(|| not_found("Attachment rule", rule_id))?;
    owned_by(Some(rule.account_id.clone()), account_id, "Attachment rule", rule_id)?;
    Ok(rule)
}

pub fn conversation_in_account(db: &Database, account_id: &str, conversation_id: &str) -> Result<()> {
    owned_by(
        db.get_chat_conversation_account(conversation_id)?,
        account_id,
        "Conversation",
        conversation_id,
    )
}

pub fn memory_fact_in_account(db: &Database, account_id: &str, fact_id: &str) -> Result<()> {
    owned_by(db.get_memory_fact_account(fact_id)?, account_id, "Memory fact", fact_id)
}

pub fn pending_task_in_account(db: &Database, account_id: &str, task_id: &str) -> Result<()> {
    owned_by(db.get_pending_task_account(task_id)?, account_id, "Task", task_id)
}

pub fn outbox_in_account(db: &Database, account_id: &str, outbox_id: &str) -> Result<OutboxEntry> {
    let entry = db
        .get_outbox_entry(outbox_id)?
        .ok_or_else(|| not_found("Outbox message", outbox_id))?;
    owned_by(Some(entry.account_id.clone()), account_id, "Outbox message", outbox_id)?;
    Ok(entry)
}

/// A thread exists in `account_id` when at least one of its emails does.
pub fn thread_in_account(db: &Database, account_id: &str, thread_id: &str) -> Result<()> {
    if db.get_thread(account_id, thread_id)?.is_empty() {
        return Err(not_found("Thread", thread_id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use rusqlite::params;

    const OWNER: &str = "acct-owner";
    const OTHER: &str = "acct-other";

    /// Two accounts and one record of every checked kind, all owned by OWNER.
    fn db_with_records() -> Database {
        let db = Database::new_for_testing().expect("test db");
        {
            let conn = db.connection();
            for (id, email) in [(OWNER, "owner@example.com"), (OTHER, "other@example.com")] {
                conn.execute(
                    "INSERT INTO accounts (id, provider, email, name, created_at) VALUES (?1, 'imap', ?2, ?2, 0)",
                    params![id, email],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email, recipients_json, snippet, timestamp, created_at)
                 VALUES ('email-1', ?1, 'thread-1', 's', 'S', 's@example.com', '[]', '', 0, 0)",
                params![OWNER],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO drafts (id, account_id, to_addresses_json, subject, body, created_at, updated_at)
                 VALUES ('draft-1', ?1, '[]', 's', 'b', 0, 0)",
                params![OWNER],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO attachment_rules (id, account_id, name, created_at, updated_at) VALUES ('rule-1', ?1, 'r', 0, 0)",
                params![OWNER],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO chat_conversations (id, account_id, title, created_at, updated_at) VALUES ('conv-1', ?1, 't', 0, 0)",
                params![OWNER],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO memory_facts (id, account_id, subject_kind, subject_key, fact, created_at, updated_at)
                 VALUES ('fact-1', ?1, 'user', 'me', 'f', 0, 0)",
                params![OWNER],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO pending_tasks (id, account_id, title, created_at, updated_at) VALUES ('task-1', ?1, 't', 0, 0)",
                params![OWNER],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO outbox (id, account_id, kind, origin, payload, send_at, created_at, updated_at)
                 VALUES ('outbox-1', ?1, 'new', 'scheduled', '{}', 1, 0, 0)",
                params![OWNER],
            )
            .unwrap();
        }
        db
    }

    type Check = fn(&Database, &str, &str) -> Result<()>;

    /// Every check: the record is found for its own account, and reads as
    /// "not found" — never as someone else's record — for another account or
    /// an id that does not exist.
    #[test]
    fn a_record_is_visible_only_to_its_own_account() {
        let db = db_with_records();
        let checks: [(&str, &str, Check); 8] = [
            ("email", "email-1", |db, a, id| email_in_account(db, a, id).map(|_| ())),
            ("draft", "draft-1", |db, a, id| draft_in_account(db, a, id).map(|_| ())),
            ("attachment rule", "rule-1", |db, a, id| {
                attachment_rule_in_account(db, a, id).map(|_| ())
            }),
            ("conversation", "conv-1", conversation_in_account),
            ("memory fact", "fact-1", memory_fact_in_account),
            ("pending task", "task-1", pending_task_in_account),
            ("thread", "thread-1", thread_in_account),
            ("outbox message", "outbox-1", |db, a, id| {
                outbox_in_account(db, a, id).map(|_| ())
            }),
        ];
        for (kind, id, check) in checks {
            assert!(check(&db, OWNER, id).is_ok(), "{kind}: owner must see it");
            assert!(
                matches!(check(&db, OTHER, id), Err(AppError::NotFound(_))),
                "{kind}: another account must get NotFound"
            );
            assert!(
                matches!(check(&db, OWNER, "missing"), Err(AppError::NotFound(_))),
                "{kind}: a missing id must get NotFound"
            );
        }
    }

    #[test]
    fn the_owned_record_is_returned() {
        let db = db_with_records();
        assert_eq!(email_in_account(&db, OWNER, "email-1").unwrap().id, "email-1");
        assert_eq!(draft_in_account(&db, OWNER, "draft-1").unwrap().id, "draft-1");
        assert_eq!(attachment_rule_in_account(&db, OWNER, "rule-1").unwrap().id, "rule-1");
        assert_eq!(outbox_in_account(&db, OWNER, "outbox-1").unwrap().id, "outbox-1");
    }
}
