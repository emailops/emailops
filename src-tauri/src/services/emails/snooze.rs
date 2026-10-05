//! Snooze: hide a conversation from the Inbox until a chosen time, then bring
//! it back to the top, unread.
//!
//! Local state (V031 `thread_snoozes`) — no provider exposes a portable
//! snooze. The wake-up ticker in `sync_scheduler` calls [`wake_due_snoozes`]
//! every 30 s and once at start-up, so a snooze that fell due while the app was
//! closed wakes on the next launch. New mail in a snoozed thread releases it in
//! the ingest itself (`db::emails::snoozes`).

use std::sync::Arc;

use serde::Serialize;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::ThreadSnooze;
use crate::services::app_handle::AppHandle;
use crate::services::logger;

use super::thread_actions::{apply_thread_action, ThreadAction, ThreadActionFailure, ThreadActionReport, ThreadRef};

/// Event the wake-up emits with the conversations that came back, so the
/// frontend refetches the list (and a notifier can announce them).
pub const SNOOZES_WOKEN_EVENT: &str = "snoozes-woken";

/// Payload of [`SNOOZES_WOKEN_EVENT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnoozesWoken {
    pub threads: Vec<ThreadRef>,
}

/// Pure: the snoozed conversations whose time has come at `now`. Woken
/// records are already out of the plan.
pub fn due_snoozes(now: i64, records: &[ThreadSnooze]) -> Vec<ThreadRef> {
    records
        .iter()
        .filter(|r| r.woke_at.is_none() && r.snoozed_until <= now)
        .map(|r| ThreadRef {
            account_id: r.account_id.clone(),
            thread_id: r.thread_id.clone(),
        })
        .collect()
}

fn pairs(threads: &[ThreadRef]) -> Vec<(&str, &str)> {
    threads
        .iter()
        .map(|t| (t.account_id.as_str(), t.thread_id.as_str()))
        .collect()
}

/// Snooze each conversation until `until` (unix seconds, must be after
/// `now`). A conversation that no longer exists is reported, the others are
/// snoozed.
pub fn snooze_threads(db: &Database, threads: &[ThreadRef], until: i64, now: i64) -> Result<ThreadActionReport> {
    if until <= now {
        return Err(AppError::InvalidInput("a snooze must end in the future".to_string()));
    }
    let mut report = ThreadActionReport::default();
    let mut snoozable = Vec::new();
    for thread in threads {
        if db.get_thread(&thread.account_id, &thread.thread_id)?.is_empty() {
            let e = AppError::NotFound(format!("Conversation {} not found", thread.thread_id));
            report.failed.push(ThreadActionFailure::new(thread, &e));
        } else {
            snoozable.push(thread.clone());
        }
    }
    if !snoozable.is_empty() {
        db.snooze_threads(&pairs(&snoozable), until, now)?;
        logger::log(
            "success",
            "system",
            format!("Snoozed {} conversation(s)", snoozable.len()),
        );
    }
    Ok(report)
}

/// Bring conversations back now, without marking them unread — the inverse
/// of snoozing them.
pub fn unsnooze_threads(db: &Database, threads: &[ThreadRef]) -> Result<()> {
    let n = db.unsnooze_threads(&pairs(threads))?;
    if n > 0 {
        logger::log("success", "system", format!("Unsnoozed {n} conversation(s)"));
    }
    Ok(())
}

/// Snooze records for one account, or every enabled account (`None`).
pub fn list_thread_snoozes(db: &Database, account_id: Option<&str>) -> Result<Vec<ThreadSnooze>> {
    let scope = match account_id {
        Some(id) => crate::db::AccountScope::Account(id),
        None => crate::db::AccountScope::AllEnabled,
    };
    db.list_thread_snoozes(scope)
}

/// Wake every snooze due at `now`: the conversation returns to the inbox,
/// sorted by the wake time, with its latest message marked unread — pushed to
/// the provider like any mark-unread, so the next state refresh does not read
/// it back as read. Emits [`SNOOZES_WOKEN_EVENT`] and one log line. Returns the
/// woken conversations.
pub async fn wake_due_snoozes(db: &Arc<Database>, now: i64, app: Option<AppHandle>) -> Result<Vec<ThreadRef>> {
    let due = due_snoozes(now, &db.pending_snoozes()?);
    let mut woken = Vec::new();
    for thread in due {
        // One at a time: a record re-snoozed meanwhile is skipped.
        if db.mark_snoozes_woken(&[(&thread.account_id, &thread.thread_id)], now)? > 0 {
            woken.push(thread);
        }
    }
    let pruned = db.prune_woken_snoozes()?;
    if pruned > 0 {
        logger::log(
            "debug",
            "system",
            format!("Dropped {pruned} woken snooze record(s) whose conversation left the inbox"),
        );
    }
    if woken.is_empty() {
        return Ok(woken);
    }
    let report = apply_thread_action(db, &woken, ThreadAction::MarkUnread, app).await;
    for failure in &report.failed {
        logger::log(
            "error",
            "system",
            format!(
                "A conversation back from snooze could not be marked unread: {}",
                failure.message
            ),
        );
    }
    logger::log(
        "info",
        "system",
        format!("{} snoozed conversation(s) returned to the inbox", woken.len()),
    );
    crate::services::events::emit(SNOOZES_WOKEN_EVENT, SnoozesWoken { threads: woken.clone() });
    Ok(woken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Email;

    const NOW: i64 = 1_800_000_000;

    fn record(thread: &str, until: i64, woke_at: Option<i64>) -> ThreadSnooze {
        ThreadSnooze {
            account_id: "acc-1".into(),
            thread_id: thread.into(),
            snoozed_until: until,
            created_at: NOW - 86_400,
            woke_at,
        }
    }

    fn thread(id: &str) -> ThreadRef {
        ThreadRef {
            account_id: "acc-1".into(),
            thread_id: id.into(),
        }
    }

    #[test]
    fn due_snoozes_are_the_snoozed_ones_whose_time_has_come() {
        let records = [
            record("past", NOW - 60, None),
            record("exactly-now", NOW, None),
            record("future", NOW + 1, None),
            record("already-woken", NOW - 60, Some(NOW - 30)),
        ];
        assert_eq!(due_snoozes(NOW, &records), vec![thread("past"), thread("exactly-now")]);
        assert!(due_snoozes(NOW, &[]).is_empty());
    }

    fn message(id: &str, ts: i64, is_read: bool) -> Email {
        Email {
            id: id.into(),
            account_id: "acc-1".into(),
            thread_id: "t-1".into(),
            message_id: None,
            references: None,
            subject: "Plans".into(),
            sender: "Ana".into(),
            sender_email: "ana@example.com".into(),
            recipients: vec![],
            cc: vec![],
            body: String::new(),
            snippet: String::new(),
            timestamp: ts,
            is_read,
            triage_status: None,
            category: "primary".into(),
            mailbox: "inbox".into(),
            is_sent: false,
            is_starred: false,
            headers: None,
        }
    }

    /// An account whose provider takes no mailbox writes, so the mark-unread
    /// stays local and the test needs no network.
    fn local_db(rows: &[Email]) -> Arc<Database> {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.connection()
            .execute("UPDATE accounts SET provider = 'local-test' WHERE id = 'acc-1'", [])
            .unwrap();
        db.insert_emails_batch(rows).unwrap();
        Arc::new(db)
    }

    #[test]
    fn snoozing_needs_a_future_time_and_an_existing_thread() {
        let db = local_db(&[message("a", NOW - 100, true)]);
        assert!(matches!(
            snooze_threads(&db, &[thread("t-1")], NOW, NOW),
            Err(AppError::InvalidInput(_))
        ));
        let report = snooze_threads(&db, &[thread("t-1"), thread("missing")], NOW + 60, NOW).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].thread_id, "missing");
        assert_eq!(report.failed[0].code, "not_found");
        let listed: Vec<String> = list_thread_snoozes(&db, Some("acc-1"))
            .unwrap()
            .into_iter()
            .map(|s| s.thread_id)
            .collect();
        assert_eq!(listed, vec!["t-1".to_string()]);
        unsnooze_threads(&db, &[thread("t-1")]).unwrap();
        assert!(list_thread_snoozes(&db, None).unwrap().is_empty());
    }

    /// Sync test driving its own runtime: the event-sink lock must not be
    /// held across an `.await` of the test body itself.
    #[test]
    fn a_due_snooze_wakes_unread_at_the_top_of_the_inbox_and_announces_itself() {
        let _g = crate::services::events::seam_test_lock();
        let sink = crate::services::events::install_for_testing();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut newer = message("n", NOW - 60, true);
        newer.thread_id = "t-newer".into();
        let db = local_db(&[message("a", NOW - 30 * 86_400, true), newer]);
        snooze_threads(&db, &[thread("t-1")], NOW - 1, NOW - 3600).unwrap();

        let woken = rt.block_on(wake_due_snoozes(&db, NOW, None)).unwrap();

        assert_eq!(woken, vec![thread("t-1")]);
        assert!(
            !db.get_email("a").unwrap().unwrap().is_read,
            "the latest message is unread again"
        );
        let inbox: Vec<String> = db
            .get_emails(crate::db::AccountScope::Account("acc-1"), 50, 0, None, None, None)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(inbox, vec!["a".to_string(), "n".to_string()]);
        assert_eq!(sink.count(SNOOZES_WOKEN_EVENT), 1);
        // A second tick has nothing left to wake.
        assert!(rt.block_on(wake_due_snoozes(&db, NOW + 30, None)).unwrap().is_empty());
        assert_eq!(sink.count(SNOOZES_WOKEN_EVENT), 1);
        crate::services::events::install(Arc::new(crate::services::events::NoopEventSink));
    }

    #[tokio::test]
    async fn a_snooze_not_yet_due_stays_hidden() {
        let db = local_db(&[message("a", NOW - 100, true)]);
        snooze_threads(&db, &[thread("t-1")], NOW + 600, NOW).unwrap();
        assert!(wake_due_snoozes(&db, NOW, None).await.unwrap().is_empty());
        assert!(db
            .get_emails(crate::db::AccountScope::Account("acc-1"), 50, 0, None, None, None)
            .unwrap()
            .is_empty());
    }
}
