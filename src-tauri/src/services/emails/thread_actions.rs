//! Thread-level mailbox actions — mark read/unread, star/unstar, archive,
//! move back to the inbox and delete — with their provider write-back.
//!
//! Every action takes a list of threads, so one call serves the reading pane
//! (one thread) and bulk selection (many). Each thread is planned on its own
//! ([`plan_thread_action`], pure) and executed against the account's provider;
//! one thread failing never stops the others, and the report names the ones
//! that failed so the UI can roll back exactly those.
//!
//! The two families order their writes differently, following the actions
//! they extend (`mailbox_state`):
//!
//! - **read state and star** are local-first: the row changes immediately and
//!   is marked pending until the provider has it; the sync retries what is
//!   pending. An account whose provider cannot be reached still changes
//!   locally (the push waits for the next sync).
//! - **archive, move to inbox and delete** are provider-first, like folder
//!   moves: on IMAP and Graph they re-key the message, so a local-only change
//!   would leave a row the provider no longer knows under that id. A failure
//!   leaves the thread where it was and is reported.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, Email};
use crate::services::app_handle::AppHandle;
use crate::services::logger;
use crate::sync::provider::{provider_supports_mailbox_writes, EmailProvider, MessageLocation, MoveTarget};

use super::folders::refile_moved_row;
use super::mailbox_state::{delete_email_with_provider, set_flag, PushVia, PushedFlag};
use super::optimistic::LOCAL_SENT_ID_PREFIX;

/// One conversation of one account — thread ids are only unique per account.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct ThreadRef {
    pub account_id: String,
    pub thread_id: String,
}

/// What to do to a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub enum ThreadAction {
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
    Archive,
    MoveToInbox,
    /// Move every message of the conversation to the provider's Trash.
    Delete,
}

/// A thread an action could not be applied to, with the error in the same
/// `{code, params, message}` shape `AppError` has at the command boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct ThreadActionFailure {
    pub account_id: String,
    pub thread_id: String,
    pub code: String,
    pub params: BTreeMap<String, String>,
    pub message: String,
}

/// The outcome of one action over many threads: every thread not listed in
/// `failed` was applied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct ThreadActionReport {
    pub failed: Vec<ThreadActionFailure>,
}

impl ThreadActionFailure {
    pub(crate) fn new(thread: &ThreadRef, error: &AppError) -> Self {
        Self {
            account_id: thread.account_id.clone(),
            thread_id: thread.thread_id.clone(),
            code: error.code().to_string(),
            params: error.params().into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            message: error.to_string(),
        }
    }
}

/// Where an action's provider writes go for one account.
pub enum ProviderAccess<'a> {
    /// The account's provider has no mailbox writes: changes stay local.
    LocalOnly,
    /// It has them, but could not be reached (offline, expired credentials).
    Unreachable(&'a AppError),
    Ready(&'a dyn EmailProvider),
}

/// Pure: which messages of a thread (as `get_thread` returns it: live rows,
/// oldest first) `action` changes.
///
/// - **Mark read** reads every unread message.
/// - **Mark unread** marks the latest received message unread — what Gmail
///   does — and nothing when the thread already has an unread message.
/// - **Star** stars the latest message, unless one is starred already; a
///   thread is starred when any of its messages is.
/// - **Unstar** unstars every starred message.
/// - **Archive** takes every inbox message out of the inbox; Sent, Spam,
///   Trash and folder copies stay where they are.
/// - **Move to inbox** brings back every archived message, and on IMAP every
///   message filed in a folder (the archive folder is one).
/// - **Delete** trashes every message, as deleting a conversation always has.
pub fn plan_thread_action(action: ThreadAction, messages: &[Email]) -> Vec<String> {
    let ids = |pred: &dyn Fn(&Email) -> bool| -> Vec<String> {
        messages.iter().filter(|m| pred(m)).map(|m| m.id.clone()).collect()
    };
    match action {
        ThreadAction::MarkRead => ids(&|m| !m.is_read),
        ThreadAction::MarkUnread => {
            if messages.iter().any(|m| !m.is_read) {
                return Vec::new();
            }
            latest(messages, |m| !m.is_sent && !in_spam_or_trash(m))
                .map(|m| vec![m.id.clone()])
                .unwrap_or_default()
        }
        ThreadAction::Star => {
            if messages.iter().any(|m| m.is_starred) {
                return Vec::new();
            }
            latest(messages, |m| !in_spam_or_trash(m))
                .map(|m| vec![m.id.clone()])
                .unwrap_or_default()
        }
        ThreadAction::Unstar => ids(&|m| m.is_starred),
        ThreadAction::Archive => ids(&|m| m.mailbox == "inbox"),
        ThreadAction::MoveToInbox => ids(&|m| m.mailbox == "archive" || m.mailbox.starts_with("folder:")),
        ThreadAction::Delete => ids(&|_| true),
    }
}

fn in_spam_or_trash(m: &Email) -> bool {
    m.mailbox == "spam" || m.mailbox == "trash"
}

/// The newest message matching `preferred`, else the newest of all.
fn latest(messages: &[Email], preferred: impl Fn(&Email) -> bool) -> Option<&Email> {
    let order = |a: &&Email, b: &&Email| (a.timestamp, &a.id).cmp(&(b.timestamp, &b.id));
    messages
        .iter()
        .filter(|m| preferred(m))
        .max_by(order)
        .or_else(|| messages.iter().max_by(order))
}

/// Command entry point: apply `action` to every thread, resolving each
/// account's provider once. Never fails as a whole — the report lists the
/// threads that could not be changed.
pub async fn apply_thread_action(
    db: &Arc<Database>,
    threads: &[ThreadRef],
    action: ThreadAction,
    app: Option<AppHandle>,
) -> ThreadActionReport {
    let mut report = ThreadActionReport::default();
    for (account_id, group) in group_by_account(threads) {
        let account = match db.get_account(account_id) {
            Ok(Some(account)) => account,
            Ok(None) => {
                let e = AppError::NotFound(format!("Account {account_id} not found"));
                report
                    .failed
                    .extend(group.iter().map(|t| ThreadActionFailure::new(t, &e)));
                continue;
            }
            Err(e) => {
                report
                    .failed
                    .extend(group.iter().map(|t| ThreadActionFailure::new(t, &e)));
                continue;
            }
        };
        if !provider_supports_mailbox_writes(&account.provider) {
            apply_to_account(db, &account, &group, action, ProviderAccess::LocalOnly, &mut report).await;
            continue;
        }
        match super::build_provider(&account, app.clone()).await {
            Ok(provider) => {
                apply_to_account(
                    db,
                    &account,
                    &group,
                    action,
                    ProviderAccess::Ready(provider.as_ref()),
                    &mut report,
                )
                .await;
            }
            Err(e) => {
                apply_to_account(
                    db,
                    &account,
                    &group,
                    action,
                    ProviderAccess::Unreachable(&e),
                    &mut report,
                )
                .await;
            }
        }
    }
    report
}

/// Threads grouped per account, in the order the accounts first appear.
fn group_by_account(threads: &[ThreadRef]) -> Vec<(&str, Vec<&ThreadRef>)> {
    let mut groups: Vec<(&str, Vec<&ThreadRef>)> = Vec::new();
    for thread in threads {
        match groups.iter_mut().find(|(id, _)| *id == thread.account_id) {
            Some((_, group)) => group.push(thread),
            None => groups.push((thread.account_id.as_str(), vec![thread])),
        }
    }
    groups
}

/// Apply `action` to one account's threads through `access`, logging one
/// summary line and recording each failure.
pub async fn apply_to_account(
    db: &Arc<Database>,
    account: &Account,
    threads: &[&ThreadRef],
    action: ThreadAction,
    access: ProviderAccess<'_>,
    report: &mut ThreadActionReport,
) {
    let mut done = 0usize;
    let mut failed = 0usize;
    for thread in threads {
        match apply_to_thread(db, &thread.account_id, &thread.thread_id, action, &access).await {
            Ok(()) => done += 1,
            Err(e) => {
                failed += 1;
                logger::log(
                    "error",
                    "sync",
                    format!("[{}] Could not {} a conversation: {e}", account.email, verb(action)),
                );
                report.failed.push(ThreadActionFailure::new(thread, &e));
            }
        }
    }
    if done > 0 {
        logger::log(
            "success",
            "sync",
            format!("[{}] {} {done} conversation(s)", account.email, past_tense(action)),
        );
    }
    if failed > 0 && matches!(access, ProviderAccess::Unreachable(_)) {
        logger::log(
            "error",
            "sync",
            format!(
                "[{}] The mail provider could not be reached: {failed} conversation(s) left unchanged",
                account.email
            ),
        );
    }
}

fn verb(action: ThreadAction) -> &'static str {
    match action {
        ThreadAction::MarkRead => "mark as read",
        ThreadAction::MarkUnread => "mark as unread",
        ThreadAction::Star => "star",
        ThreadAction::Unstar => "unstar",
        ThreadAction::Archive => "archive",
        ThreadAction::MoveToInbox => "move to the inbox",
        ThreadAction::Delete => "delete",
    }
}

fn past_tense(action: ThreadAction) -> &'static str {
    match action {
        ThreadAction::MarkRead => "Marked as read",
        ThreadAction::MarkUnread => "Marked as unread",
        ThreadAction::Star => "Starred",
        ThreadAction::Unstar => "Unstarred",
        ThreadAction::Archive => "Archived",
        ThreadAction::MoveToInbox => "Moved to the inbox",
        ThreadAction::Delete => "Deleted",
    }
}

/// Apply `action` to one thread. Stops at the first message that fails; the
/// messages before it keep their change (each one is complete on its own).
pub async fn apply_to_thread(
    db: &Arc<Database>,
    account_id: &str,
    thread_id: &str,
    action: ThreadAction,
    access: &ProviderAccess<'_>,
) -> Result<()> {
    let messages = db.get_thread(account_id, thread_id)?;
    if messages.is_empty() {
        return Err(AppError::NotFound(format!("Conversation {thread_id} not found")));
    }
    let targets = plan_thread_action(action, &messages);
    for id in &targets {
        let id = id.as_str();
        let Some(email) = messages.iter().find(|m| m.id == id) else {
            continue;
        };
        match action {
            ThreadAction::MarkRead | ThreadAction::MarkUnread => {
                set_flag(
                    db,
                    id,
                    PushedFlag::Read,
                    action == ThreadAction::MarkRead,
                    push_via(access),
                )
                .await?
            }
            ThreadAction::Star | ThreadAction::Unstar => {
                set_flag(db, id, PushedFlag::Star, action == ThreadAction::Star, push_via(access)).await?
            }
            ThreadAction::Archive => archive_message(db, email, writable(access)?).await?,
            ThreadAction::MoveToInbox => move_message_to_inbox(db, email, writable(access)?).await?,
            ThreadAction::Delete => delete_email_with_provider(db, id, writable(access)?).await?,
        }
    }
    // A conversation the user archives or deletes is done with: its snooze
    // ends too (as in Gmail), so it neither lingers in the Snoozed view nor
    // wakes later.
    if matches!(action, ThreadAction::Archive | ThreadAction::Delete) {
        db.unsnooze_threads(&[(account_id, thread_id)])?;
    }
    // The same interaction signals the single-message read and delete
    // commands record (thread state, follow-up tasks).
    match action {
        ThreadAction::MarkRead => {
            for id in &targets {
                crate::services::tasks::on_email_read(db, id);
            }
        }
        ThreadAction::Archive if !targets.is_empty() => {
            if let Some(latest) = db.get_thread(account_id, thread_id)?.last() {
                crate::services::tasks::on_archived(db, &latest.id);
            }
        }
        // A soft delete keeps the rows, so `on_archived` still resolves them
        // (as the single-message `delete_email` command relies on).
        ThreadAction::Delete => {
            if let Some(latest) = targets.last() {
                crate::services::tasks::on_archived(db, latest);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Local-first writes: an unreachable provider queues the push.
fn push_via<'a>(access: &ProviderAccess<'a>) -> PushVia<'a> {
    match access {
        ProviderAccess::LocalOnly => PushVia::Never,
        ProviderAccess::Unreachable(_) => PushVia::NextSync,
        ProviderAccess::Ready(provider) => PushVia::Now(*provider),
    }
}

/// Provider-first writes: an unreachable provider is the action's error.
fn writable<'a>(access: &ProviderAccess<'a>) -> Result<Option<&'a dyn EmailProvider>> {
    match access {
        ProviderAccess::LocalOnly => Ok(None),
        ProviderAccess::Unreachable(e) => Err(AppError::SyncError(format!(
            "the mail provider could not be reached: {e}"
        ))),
        ProviderAccess::Ready(provider) => Ok(Some(*provider)),
    }
}

/// Locally-composed Sent rows carry a synthetic id the provider never saw.
fn pushable<'a>(provider: Option<&'a dyn EmailProvider>, email: &Email) -> Option<&'a dyn EmailProvider> {
    provider.filter(|_| !email.id.starts_with(LOCAL_SENT_ID_PREFIX))
}

/// Take one message out of the inbox at the provider, then file the row where
/// the provider put it (re-keyed on IMAP and Graph). "No such message" means
/// the provider lost it under this id: the row is archived locally and the
/// state refresh follows it from there.
async fn archive_message(db: &Database, email: &Email, provider: Option<&dyn EmailProvider>) -> Result<()> {
    let local = || MessageLocation {
        id: email.id.clone(),
        mailbox: "archive".to_string(),
    };
    let location = match pushable(provider, email) {
        None => local(),
        Some(provider) => match provider.archive_message(&email.id, email.message_id.as_deref()).await {
            Ok(location) => location,
            Err(AppError::NotFound(_)) => local(),
            Err(e) => return Err(e),
        },
    };
    refile_moved_row(db, &email.id, &location.id, &location.mailbox)
}

/// Bring one archived (or foldered) message back to the inbox — the inverse
/// of [`archive_message`].
async fn move_message_to_inbox(db: &Database, email: &Email, provider: Option<&dyn EmailProvider>) -> Result<()> {
    let new_id = match pushable(provider, email) {
        None => email.id.clone(),
        Some(provider) => {
            match provider
                .move_message(&email.id, email.message_id.as_deref(), &MoveTarget::Inbox)
                .await
            {
                Ok(Some(moved)) => moved.id,
                Ok(None) | Err(AppError::NotFound(_)) => email.id.clone(),
                Err(e) => return Err(e),
            }
        }
    };
    refile_moved_row(db, &email.id, &new_id, "inbox")
}

/// Widen each row's star to its thread's: in the thread-deduped lists a row
/// stands for its conversation, which is starred when any message is.
pub fn widen_stars_to_threads(db: &Database, emails: &mut [Email]) -> Result<()> {
    let keys: Vec<(&str, &str)> = emails
        .iter()
        .filter(|e| !e.is_starred)
        .map(|e| (e.account_id.as_str(), e.thread_id.as_str()))
        .collect();
    if keys.is_empty() {
        return Ok(());
    }
    let starred = db.starred_threads(&keys)?;
    for email in emails.iter_mut() {
        if starred.contains(&(email.account_id.clone(), email.thread_id.clone())) {
            email.is_starred = true;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::provider::{EmailCategory, FakeEmailProvider, FakeFolderOp, FakeMailboxOp};

    fn message(id: &str, ts: i64, mailbox: &str) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acc-1".to_string(),
            thread_id: "t-1".to_string(),
            message_id: Some(format!("<{id}@example.com>")),
            references: None,
            subject: "s".to_string(),
            sender: "Ana".to_string(),
            sender_email: "ana@example.com".to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: vec![],
            body: "body".to_string(),
            snippet: "body".to_string(),
            timestamp: ts,
            is_read: true,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: mailbox.to_string(),
            is_sent: mailbox == "sent",
            is_starred: false,
            headers: None,
        }
    }

    fn with(mut email: Email, change: impl FnOnce(&mut Email)) -> Email {
        change(&mut email);
        email
    }

    fn ids(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // ── planner ───────────────────────────────────────────────────────────

    #[test]
    fn mark_read_reads_every_unread_message() {
        let thread = [
            with(message("a", 1, "inbox"), |m| m.is_read = false),
            message("b", 2, "inbox"),
            with(message("c", 3, "inbox"), |m| m.is_read = false),
        ];
        assert_eq!(plan_thread_action(ThreadAction::MarkRead, &thread), ids(&["a", "c"]));
    }

    #[test]
    fn mark_unread_marks_the_latest_received_message_only_when_all_are_read() {
        let thread = [
            message("a", 1, "inbox"),
            message("b", 2, "inbox"),
            message("reply", 3, "sent"),
        ];
        assert_eq!(
            plan_thread_action(ThreadAction::MarkUnread, &thread),
            ids(&["b"]),
            "the user's own reply is not the one to mark"
        );

        let already_unread = [
            with(message("a", 1, "inbox"), |m| m.is_read = false),
            message("b", 2, "inbox"),
        ];
        assert!(plan_thread_action(ThreadAction::MarkUnread, &already_unread).is_empty());

        let only_sent = [message("s", 1, "sent")];
        assert_eq!(plan_thread_action(ThreadAction::MarkUnread, &only_sent), ids(&["s"]));
    }

    #[test]
    fn star_stars_the_latest_message_unless_the_thread_is_starred() {
        let thread = [
            message("a", 1, "inbox"),
            message("b", 2, "sent"),
            message("junk", 3, "spam"),
        ];
        assert_eq!(plan_thread_action(ThreadAction::Star, &thread), ids(&["b"]));

        let starred = [
            with(message("a", 1, "inbox"), |m| m.is_starred = true),
            message("b", 2, "inbox"),
        ];
        assert!(plan_thread_action(ThreadAction::Star, &starred).is_empty());
    }

    #[test]
    fn same_second_messages_pick_the_latest_by_id() {
        let thread = [message("a", 5, "inbox"), message("b", 5, "inbox")];
        assert_eq!(plan_thread_action(ThreadAction::Star, &thread), ids(&["b"]));
    }

    #[test]
    fn unstar_unstars_every_starred_message() {
        let thread = [
            with(message("a", 1, "inbox"), |m| m.is_starred = true),
            message("b", 2, "inbox"),
            with(message("c", 3, "archive"), |m| m.is_starred = true),
        ];
        assert_eq!(plan_thread_action(ThreadAction::Unstar, &thread), ids(&["a", "c"]));
    }

    #[test]
    fn archive_takes_only_inbox_messages_and_move_to_inbox_brings_back_archived_and_foldered_ones() {
        let thread = [
            message("in", 1, "inbox"),
            message("sent", 2, "sent"),
            message("arch", 3, "archive"),
            message("filed", 4, "folder:Archive"),
            message("bin", 5, "trash"),
            message("junk", 6, "spam"),
        ];
        assert_eq!(plan_thread_action(ThreadAction::Archive, &thread), ids(&["in"]));
        assert_eq!(
            plan_thread_action(ThreadAction::MoveToInbox, &thread),
            ids(&["arch", "filed"])
        );
    }

    #[test]
    fn delete_takes_every_message_of_the_thread() {
        let thread = [
            message("in", 1, "inbox"),
            message("sent", 2, "sent"),
            message("arch", 3, "archive"),
        ];
        assert_eq!(
            plan_thread_action(ThreadAction::Delete, &thread),
            ids(&["in", "sent", "arch"])
        );
    }

    // ── executor ──────────────────────────────────────────────────────────

    fn test_db(rows: &[Email]) -> Arc<Database> {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(rows).unwrap();
        Arc::new(db)
    }

    fn row(db: &Database, id: &str) -> Option<Email> {
        db.get_email(id).unwrap()
    }

    async fn run(db: &Arc<Database>, action: ThreadAction, access: ProviderAccess<'_>) -> Result<()> {
        apply_to_thread(db, "acc-1", "t-1", action, &access).await
    }

    #[tokio::test]
    async fn mark_unread_writes_locally_and_pushes_to_the_provider() {
        let db = test_db(&[message("a", 1, "inbox"), message("b", 2, "inbox")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        run(&db, ThreadAction::MarkUnread, ProviderAccess::Ready(&provider))
            .await
            .unwrap();

        assert!(!row(&db, "b").unwrap().is_read);
        assert!(row(&db, "a").unwrap().is_read);
        assert_eq!(
            provider.mailbox_ops(),
            vec![FakeMailboxOp::SetReadState {
                message_id: "b".to_string(),
                read: false
            }]
        );
        assert!(db.pending_read_pushes("acc-1", 10).unwrap().is_empty());
    }

    #[tokio::test]
    async fn mark_unread_with_an_unreachable_provider_is_queued_for_the_sync() {
        let db = test_db(&[message("a", 1, "inbox")]);
        let offline = AppError::SyncError("offline".to_string());

        run(&db, ThreadAction::MarkUnread, ProviderAccess::Unreachable(&offline))
            .await
            .unwrap();

        assert!(!row(&db, "a").unwrap().is_read);
        let pending = db.pending_read_pushes("acc-1", 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(!pending[0].is_read);
    }

    #[tokio::test]
    async fn star_and_unstar_push_the_star() {
        let db = test_db(&[message("a", 1, "inbox"), message("b", 2, "inbox")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        run(&db, ThreadAction::Star, ProviderAccess::Ready(&provider))
            .await
            .unwrap();
        assert!(row(&db, "b").unwrap().is_starred);
        run(&db, ThreadAction::Unstar, ProviderAccess::Ready(&provider))
            .await
            .unwrap();

        assert!(!row(&db, "b").unwrap().is_starred);
        assert_eq!(
            provider.mailbox_ops(),
            vec![
                FakeMailboxOp::SetStarred {
                    message_id: "b".to_string(),
                    starred: true
                },
                FakeMailboxOp::SetStarred {
                    message_id: "b".to_string(),
                    starred: false
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_star_push_keeps_the_local_star_and_stays_pending() {
        let db = test_db(&[message("a", 1, "inbox")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("503");

        run(&db, ThreadAction::Star, ProviderAccess::Ready(&provider))
            .await
            .unwrap();

        assert!(row(&db, "a").unwrap().is_starred);
        assert_eq!(db.pending_star_pushes("acc-1", 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn archive_files_the_inbox_messages_under_archive_and_leaves_sent_alone() {
        let db = test_db(&[message("a", 1, "inbox"), message("reply", 2, "sent")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        run(&db, ThreadAction::Archive, ProviderAccess::Ready(&provider))
            .await
            .unwrap();

        assert_eq!(row(&db, "a").unwrap().mailbox, "archive");
        assert_eq!(row(&db, "reply").unwrap().mailbox, "sent");
        assert_eq!(
            provider.mailbox_ops(),
            vec![FakeMailboxOp::Archive {
                message_id: "a".to_string()
            }]
        );
        let inbox = db
            .get_emails(
                crate::db::AccountScope::Account("acc-1"),
                50,
                0,
                None,
                Some("inbox"),
                None,
            )
            .unwrap();
        assert!(inbox.is_empty(), "the thread left the inbox");
    }

    #[tokio::test]
    async fn archive_re_keys_the_row_when_the_provider_moves_the_message() {
        let db = test_db(&[message("acc-1::7", 1, "inbox")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.set_archive_location(MessageLocation {
            id: "acc-1::FOLDER::QXJjaGl2ZQ::3".to_string(),
            mailbox: "folder:Archive".to_string(),
        });

        run(&db, ThreadAction::Archive, ProviderAccess::Ready(&provider))
            .await
            .unwrap();

        assert!(row(&db, "acc-1::7").is_none());
        assert_eq!(
            row(&db, "acc-1::FOLDER::QXJjaGl2ZQ::3").unwrap().mailbox,
            "folder:Archive"
        );
    }

    #[tokio::test]
    async fn a_refused_archive_leaves_the_thread_in_the_inbox() {
        let db = test_db(&[message("a", 1, "inbox")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("503");

        let err = run(&db, ThreadAction::Archive, ProviderAccess::Ready(&provider))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("503"), "{err}");
        assert_eq!(row(&db, "a").unwrap().mailbox, "inbox");
    }

    #[tokio::test]
    async fn archive_with_an_unreachable_provider_is_refused_not_queued() {
        let db = test_db(&[message("a", 1, "inbox")]);
        let offline = AppError::SyncError("offline".to_string());

        assert!(run(&db, ThreadAction::Archive, ProviderAccess::Unreachable(&offline))
            .await
            .is_err());
        assert_eq!(row(&db, "a").unwrap().mailbox, "inbox");
    }

    #[tokio::test]
    async fn archive_of_a_message_the_provider_lost_is_archived_locally() {
        let db = test_db(&[message("a", 1, "inbox")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes_as_not_found();

        run(&db, ThreadAction::Archive, ProviderAccess::Ready(&provider))
            .await
            .unwrap();

        assert_eq!(row(&db, "a").unwrap().mailbox, "archive");
    }

    #[tokio::test]
    async fn local_only_accounts_archive_and_move_back_without_provider_calls() {
        let db = test_db(&[message("a", 1, "inbox")]);

        run(&db, ThreadAction::Archive, ProviderAccess::LocalOnly)
            .await
            .unwrap();
        assert_eq!(row(&db, "a").unwrap().mailbox, "archive");
        run(&db, ThreadAction::MoveToInbox, ProviderAccess::LocalOnly)
            .await
            .unwrap();
        assert_eq!(row(&db, "a").unwrap().mailbox, "inbox");
    }

    #[tokio::test]
    async fn archiving_or_deleting_a_snoozed_thread_ends_its_snooze() {
        for action in [ThreadAction::Archive, ThreadAction::Delete] {
            let db = test_db(&[message("a", 1, "inbox")]);
            db.snooze_threads(&[("acc-1", "t-1")], 9_999_999_999, 1).unwrap();
            run(&db, action, ProviderAccess::LocalOnly).await.unwrap();
            assert!(
                db.pending_snoozes().unwrap().is_empty(),
                "{action:?} must end the snooze (Gmail does the same)"
            );
        }
        // Other actions leave it snoozed.
        let db = test_db(&[message("a", 1, "inbox")]);
        db.snooze_threads(&[("acc-1", "t-1")], 9_999_999_999, 1).unwrap();
        run(&db, ThreadAction::Star, ProviderAccess::LocalOnly).await.unwrap();
        assert_eq!(db.pending_snoozes().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn move_to_inbox_brings_an_archived_thread_back_through_the_provider() {
        let archived = message("a", 1, "archive");
        let db = test_db(std::slice::from_ref(&archived));
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.add_message(archived, EmailCategory::Primary, vec![]);

        run(&db, ThreadAction::MoveToInbox, ProviderAccess::Ready(&provider))
            .await
            .unwrap();

        assert_eq!(row(&db, "a").unwrap().mailbox, "inbox");
        assert_eq!(
            provider.folder_ops(),
            vec![FakeFolderOp::Move {
                message_id: "a".to_string(),
                mailbox_value: "inbox".to_string()
            }]
        );
    }

    #[tokio::test]
    async fn a_missing_thread_is_not_found() {
        let db = test_db(&[]);
        let err = run(&db, ThreadAction::Star, ProviderAccess::LocalOnly)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
    }

    #[tokio::test]
    async fn the_report_lists_only_the_threads_that_failed() {
        let db = Database::new_for_testing().unwrap();
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acc-1', 'exchange-ews', 'me@example.com', 'Me', 0)",
                [],
            )
            .unwrap();
        db.insert_emails_batch(&[message("a", 1, "inbox")]).unwrap();
        let db = Arc::new(db);
        let threads = vec![
            ThreadRef {
                account_id: "acc-1".to_string(),
                thread_id: "t-1".to_string(),
            },
            ThreadRef {
                account_id: "acc-1".to_string(),
                thread_id: "t-missing".to_string(),
            },
            ThreadRef {
                account_id: "acc-gone".to_string(),
                thread_id: "t-1".to_string(),
            },
        ];

        let report = apply_thread_action(&db, &threads, ThreadAction::Archive, None).await;

        let failed: Vec<(&str, &str, &str)> = report
            .failed
            .iter()
            .map(|f| (f.account_id.as_str(), f.thread_id.as_str(), f.code.as_str()))
            .collect();
        assert_eq!(
            failed,
            vec![("acc-1", "t-missing", "not_found"), ("acc-gone", "t-1", "not_found")]
        );
        assert_eq!(
            row(&db, "a").unwrap().mailbox,
            "archive",
            "the account keeps its mail local"
        );
    }

    #[test]
    fn a_failure_carries_the_error_in_its_wire_shape() {
        let failure = ThreadActionFailure::new(
            &ThreadRef {
                account_id: "acc-1".to_string(),
                thread_id: "t-1".to_string(),
            },
            &AppError::NoArchiveFolder,
        );
        assert_eq!(failure.code, "no_archive_folder");
        assert!(failure.params.is_empty());
        assert!(failure.message.contains("Archive"));
    }

    #[test]
    fn a_thread_row_is_starred_when_any_of_its_messages_is() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        let older_starred = with(message("old", 1, "inbox"), |m| m.is_starred = true);
        let other_thread = with(message("x", 1, "inbox"), |m| m.thread_id = "t-2".to_string());
        db.insert_emails_batch(&[older_starred, message("new", 2, "inbox"), other_thread.clone()])
            .unwrap();
        let mut rows = vec![message("new", 2, "inbox"), other_thread];

        widen_stars_to_threads(&db, &mut rows).unwrap();

        assert!(rows[0].is_starred, "the thread has a starred message");
        assert!(!rows[1].is_starred);
    }

    // ── delete ────────────────────────────────────────────────────────────

    fn live_thread(db: &Database) -> Vec<String> {
        db.get_thread("acc-1", "t-1")
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect()
    }

    #[tokio::test]
    async fn delete_trashes_every_message_at_the_provider_then_locally() {
        let db = test_db(&[message("a", 1, "inbox"), message("reply", 2, "sent")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        run(&db, ThreadAction::Delete, ProviderAccess::Ready(&provider))
            .await
            .unwrap();

        assert!(live_thread(&db).is_empty(), "the thread is gone");
        let trashed: Vec<String> = provider
            .mailbox_ops()
            .into_iter()
            .filter_map(|op| match op {
                FakeMailboxOp::Trash { message_id, .. } => Some(message_id),
                _ => None,
            })
            .collect();
        assert_eq!(trashed, ids(&["a", "reply"]));
    }

    #[tokio::test]
    async fn a_refused_delete_keeps_the_thread() {
        let db = test_db(&[message("a", 1, "inbox")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("503");

        assert!(run(&db, ThreadAction::Delete, ProviderAccess::Ready(&provider))
            .await
            .is_err());
        assert_eq!(live_thread(&db), ids(&["a"]));
    }

    #[tokio::test]
    async fn delete_with_an_unreachable_provider_is_refused_not_queued() {
        let db = test_db(&[message("a", 1, "inbox")]);
        let offline = AppError::SyncError("offline".to_string());

        assert!(run(&db, ThreadAction::Delete, ProviderAccess::Unreachable(&offline))
            .await
            .is_err());
        assert_eq!(live_thread(&db), ids(&["a"]));
    }

    #[tokio::test]
    async fn local_only_accounts_delete_without_provider_calls() {
        let db = test_db(&[message("a", 1, "inbox")]);

        run(&db, ThreadAction::Delete, ProviderAccess::LocalOnly).await.unwrap();

        assert!(live_thread(&db).is_empty());
    }

    #[tokio::test]
    async fn bulk_delete_reports_only_the_threads_that_failed() {
        let db = Database::new_for_testing().unwrap();
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acc-1', 'exchange-ews', 'me@example.com', 'Me', 0)",
                [],
            )
            .unwrap();
        let mut other = message("b", 2, "inbox");
        other.thread_id = "t-2".to_string();
        db.insert_emails_batch(&[message("a", 1, "inbox"), other]).unwrap();
        let db = Arc::new(db);
        let thread = |id: &str| ThreadRef {
            account_id: "acc-1".to_string(),
            thread_id: id.to_string(),
        };

        let report = apply_thread_action(
            &db,
            &[thread("t-1"), thread("t-2"), thread("t-missing")],
            ThreadAction::Delete,
            None,
        )
        .await;

        let failed: Vec<&str> = report.failed.iter().map(|f| f.thread_id.as_str()).collect();
        assert_eq!(failed, vec!["t-missing"]);
        assert!(live_thread(&db).is_empty());
        assert!(db.get_thread("acc-1", "t-2").unwrap().is_empty());
    }
}
