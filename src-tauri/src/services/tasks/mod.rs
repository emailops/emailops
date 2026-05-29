//! Task subsystem service layer.
//!
//! Owns user-visible pending tasks, thread awaiting-reply state, and the task
//! extraction configuration/pipeline. Memory facts live under `services::memory`.

use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{CreateTaskRequest, PendingTask};

pub mod config;
pub mod extractor;

/// List the user's open (or filtered) pending tasks. Backs the chat
/// `list_pending_tasks` tool and any command that needs to render the
/// pending list. Keeps the SQL in `db::memory` (where the table lives).
pub fn list_pending(
    db: &Arc<Database>,
    account_id: &str,
    status: Option<&str>,
    due_before: Option<i64>,
    limit: i32,
) -> Result<Vec<PendingTask>> {
    db.list_pending_tasks(account_id, status, due_before, limit)
}

pub fn create_task(db: &Arc<Database>, req: CreateTaskRequest) -> Result<PendingTask> {
    let title = req.title.trim();
    if title.is_empty() {
        return Err(AppError::InvalidInput("task title cannot be empty".into()));
    }
    let now = chrono::Utc::now().timestamp();
    let company = match req.company.as_ref().map(|s| s.trim().to_string()) {
        Some(c) if !c.is_empty() => Some(c),
        _ => derive_company_from_email(db, req.source_email_id.as_deref(), &req.account_id),
    };
    let task = PendingTask {
        id: Uuid::new_v4().to_string(),
        account_id: req.account_id,
        title: title.to_string(),
        detail: req.detail,
        source: req.source.unwrap_or_else(|| "user".to_string()),
        source_email_id: req.source_email_id,
        source_thread_id: req.source_thread_id,
        assignee: "me".to_string(),
        status: "open".to_string(),
        priority: req.priority.unwrap_or_else(|| "normal".to_string()),
        due_at: req.due_at,
        completed_at: None,
        company,
        created_at: now,
        updated_at: now,
    };
    db.insert_pending_task(&task)?;
    Ok(task)
}

fn derive_company_from_email(db: &Arc<Database>, email_id: Option<&str>, account_id: &str) -> Option<String> {
    let id = email_id?;
    let email = db.get_email(id).ok().flatten()?;
    let owner_email = db
        .get_account(account_id)
        .ok()
        .flatten()
        .map(|a| a.email)
        .unwrap_or_default();
    extractor::derive_company_tag(&email.recipients, &email.cc, &owner_email)
}

pub fn update_task_status(db: &Arc<Database>, task_id: &str, status: &str) -> Result<()> {
    let normalized = match status {
        "open" | "done" | "snoozed" | "dismissed" => status,
        other => {
            return Err(AppError::InvalidInput(format!(
                "invalid task status '{other}' (expected open|done|snoozed|dismissed)"
            )))
        }
    };
    let now = chrono::Utc::now().timestamp();
    let completed_at = if normalized == "done" { Some(now) } else { None };
    db.update_pending_task_status(task_id, normalized, completed_at, now)
}

pub fn on_email_read(db: &Arc<Database>, email_id: &str) {
    let Ok(Some(email)) = db.get_email(email_id) else {
        return;
    };
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = db.log_interaction_event(&email.account_id, "read", Some(email_id), Some(&email.thread_id), None) {
        crate::services::logger::log("error", "tasks", format!("log_interaction_event(read) failed: {e}"));
    }
    if let Err(e) = db.touch_thread_state(&email.account_id, &email.thread_id, None, None, now) {
        crate::services::logger::log("error", "tasks", format!("touch_thread_state(read) failed: {e}"));
    }
}

pub fn on_reply_sent(
    db: &Arc<Database>,
    account_id: &str,
    thread_id: &str,
    in_reply_to_email_id: Option<&str>,
    to_addresses: &[String],
) {
    let now = chrono::Utc::now().timestamp();
    let payload = json!({
        "to": to_addresses,
    })
    .to_string();
    if let Err(e) = db.log_interaction_event(
        account_id,
        "reply",
        in_reply_to_email_id,
        Some(thread_id),
        Some(&payload),
    ) {
        crate::services::logger::log("error", "tasks", format!("log_interaction_event(reply) failed: {e}"));
    }
    if let Err(e) = db.touch_thread_state(account_id, thread_id, Some("them"), Some(now), now) {
        crate::services::logger::log("error", "tasks", format!("touch_thread_state(reply) failed: {e}"));
    }
}

pub fn on_archived(db: &Arc<Database>, email_id: &str) {
    let Ok(Some(email)) = db.get_email(email_id) else {
        return;
    };
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = db.log_interaction_event(
        &email.account_id,
        "archive",
        Some(email_id),
        Some(&email.thread_id),
        None,
    ) {
        crate::services::logger::log("error", "tasks", format!("log_interaction_event(archive) failed: {e}"));
    }
    if let Err(e) = db.touch_thread_state(&email.account_id, &email.thread_id, Some("resolved"), None, now) {
        crate::services::logger::log("error", "tasks", format!("touch_thread_state(archive) failed: {e}"));
    }
}

pub fn on_tag_applied(db: &Arc<Database>, email_id: &str, tag_type: &str, tag_value: &str) {
    let Ok(Some(email)) = db.get_email(email_id) else {
        return;
    };
    let now = chrono::Utc::now().timestamp();
    let payload = json!({
        "tagType": tag_type,
        "tagValue": tag_value,
    })
    .to_string();
    if let Err(e) = db.log_interaction_event(
        &email.account_id,
        "tag",
        Some(email_id),
        Some(&email.thread_id),
        Some(&payload),
    ) {
        crate::services::logger::log("error", "tasks", format!("log_interaction_event(tag) failed: {e}"));
    }
    if let Err(e) = db.touch_thread_state(&email.account_id, &email.thread_id, None, None, now) {
        crate::services::logger::log("error", "tasks", format!("touch_thread_state(tag) failed: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn insert_email(db: &Database, id: &str, account: &str, thread: &str, sender: &str) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
            rusqlite::params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails (
                    id, account_id, thread_id, message_id, subject, sender, sender_email,
                    sender_domain, recipients_json, cc_json, snippet, timestamp,
                    is_read, is_deleted, category, raw_json, created_at
                 ) VALUES (?1, ?2, ?3, NULL, 'sub', ?4, ?4, 'ex.com', '[]', '[]',
                           'snip', 1000, 0, 0, 'primary', NULL, 1000)",
            rusqlite::params![id, account, thread, sender],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO email_bodies (email_id, body) VALUES (?1, '')",
            rusqlite::params![id],
        )
        .unwrap();
    }

    #[test]
    fn create_task_generates_fields() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        let task = create_task(
            &db,
            CreateTaskRequest {
                account_id: "a1".into(),
                title: "send invoice".into(),
                detail: None,
                priority: Some("high".into()),
                due_at: Some(2000),
                source_email_id: None,
                source_thread_id: None,
                source: None,
                company: None,
            },
        )
        .unwrap();
        assert!(!task.id.is_empty());
        assert_eq!(task.title, "send invoice");
        assert_eq!(task.priority, "high");
        assert_eq!(task.status, "open");
        assert_eq!(task.source, "user");
    }

    #[test]
    fn create_task_rejects_empty_title() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        let err = create_task(
            &db,
            CreateTaskRequest {
                account_id: "a1".into(),
                title: "   ".into(),
                detail: None,
                priority: None,
                due_at: None,
                source_email_id: None,
                source_thread_id: None,
                source: None,
                company: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[test]
    fn update_task_status_sets_completed_at() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        let task = create_task(
            &db,
            CreateTaskRequest {
                account_id: "a1".into(),
                title: "x".into(),
                detail: None,
                priority: None,
                due_at: None,
                source_email_id: None,
                source_thread_id: None,
                source: None,
                company: None,
            },
        )
        .unwrap();
        update_task_status(&db, &task.id, "done").unwrap();
        let rows = db.list_pending_tasks("a1", Some("done"), None, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].completed_at.is_some());
    }

    #[test]
    fn on_reply_sent_flips_awaiting_and_logs() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        on_reply_sent(&db, "a1", "t1", Some("e1"), &["x@ex.com".into()]);
        let state = db.get_thread_state("a1", "t1").unwrap().unwrap();
        assert_eq!(state.awaiting, "them");
        assert!(state.last_outbound_at.is_some());
    }

    #[test]
    fn on_email_read_logs_and_preserves_awaiting() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        insert_email(&db, "e1", "a1", "t1", "x@ex.com");
        db.touch_thread_state("a1", "t1", Some("user"), None, 500).unwrap();
        on_email_read(&db, "e1");
        let state = db.get_thread_state("a1", "t1").unwrap().unwrap();
        assert_eq!(state.awaiting, "user");
        let events = db.recent_interaction_events("a1", 10).unwrap();
        assert_eq!(events[0].kind, "read");
    }
}
