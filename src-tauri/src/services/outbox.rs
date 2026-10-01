//! The local outbox behind undo send and scheduled send (V032 `outbox`).
//!
//! A message sent with an undo window, or scheduled for later, is queued here
//! with its `send_at`; nothing reaches the provider before then, so undo and
//! cancel need no provider support. The dispatcher (`dispatch_due_outbox`,
//! driven by `sync_scheduler::outbox_dispatch_loop`) sends due rows through the
//! same delivery path as an immediate send (`emails::send_outgoing`).
//!
//! Double-send safety: a row is flipped `scheduled → sending` in one guarded
//! UPDATE before the provider is called, so two dispatch passes cannot both
//! send it, and undo loses cleanly against a pass that already took it. A row
//! still `sending` at start-up was interrupted mid-send; it becomes `failed`
//! (`interrupted`) and is never resent automatically — the user checks Sent and
//! retries if it did not go out. A provider failure is never retried
//! automatically either (a 5xx does not say whether the message left).
//!
//! The app must be running for a message to go out. A message whose time
//! passed while the app was closed is sent on the next launch.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde::Serialize;

use crate::db::outbox::{NewOutboxRow, ScheduledOutboxRow};
use crate::db::{AccountScope, Database};
use crate::models::error::{AppError, Result};
use crate::models::outbox::{OutboxEntry, OutboxOrigin, OutboxSchedule, OutgoingMessage, UNDO_SEND_DELAYS};
use crate::models::Account;
use crate::services::logger;
use crate::sync::provider::EmailProvider;

/// Event emitted whenever the dispatcher sent or failed rows (and at start-up
/// for interrupted ones), so the frontend refreshes the Scheduled view and the
/// Sent list and tells the user about failures.
pub const OUTBOX_UPDATED_EVENT: &str = "outbox-updated";

/// Finished rows (sent, cancelled) are deleted this long after they finished.
const FINISHED_RETENTION_SECS: i64 = 7 * 86_400;

/// Furthest ahead a message can be scheduled (one year).
const MAX_SCHEDULE_AHEAD_SECS: i64 = 366 * 86_400;

/// A row the dispatcher sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxSent {
    pub id: String,
    pub account_id: String,
    /// The conversation a reply landed in, so an open thread can refresh.
    pub thread_id: Option<String>,
}

/// A row that could not be sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxFailed {
    pub id: String,
    pub account_id: String,
    /// The app stopped mid-send: the message may or may not have gone out.
    pub interrupted: bool,
    pub message: String,
}

/// Payload of [`OUTBOX_UPDATED_EVENT`], and what a dispatch pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxUpdated {
    pub sent: Vec<OutboxSent>,
    pub failed: Vec<OutboxFailed>,
}

impl OutboxUpdated {
    fn is_empty(&self) -> bool {
        self.sent.is_empty() && self.failed.is_empty()
    }
}

/// Builds the provider a due row is sent through. Production builds the
/// account's real client (refreshing OAuth tokens); tests hand out fakes.
#[async_trait]
pub trait OutboxProviders: Send + Sync {
    async fn provider_for(&self, account: &Account) -> Result<Box<dyn EmailProvider>>;
}

// ── Pure planners ────────────────────────────────────────────────────────────

/// Pure: the scheduled rows due at `now`, soonest first.
pub fn due_outbox(now: i64, rows: &[ScheduledOutboxRow]) -> Vec<String> {
    let mut due: Vec<&ScheduledOutboxRow> = rows.iter().filter(|r| r.send_at <= now).collect();
    due.sort_by_key(|r| r.send_at);
    due.into_iter().map(|r| r.id.clone()).collect()
}

/// Pure: when a queued message goes out, and why it waits.
pub fn plan_send_at(now: i64, schedule: OutboxSchedule) -> Result<(i64, OutboxOrigin)> {
    match schedule {
        OutboxSchedule::Undo { delay_secs } => {
            if !UNDO_SEND_DELAYS.contains(&delay_secs) {
                return Err(AppError::InvalidInput(format!(
                    "undo-send delay must be one of {UNDO_SEND_DELAYS:?} seconds"
                )));
            }
            Ok((now + delay_secs, OutboxOrigin::Undo))
        }
        OutboxSchedule::At { send_at } => {
            if send_at <= now {
                return Err(AppError::InvalidInput(
                    "a scheduled send must be in the future".to_string(),
                ));
            }
            if send_at > now + MAX_SCHEDULE_AHEAD_SECS {
                return Err(AppError::InvalidInput(
                    "a send can be scheduled at most a year ahead".to_string(),
                ));
            }
            Ok((send_at, OutboxOrigin::Scheduled))
        }
    }
}

// ── Queue, cancel, send now ──────────────────────────────────────────────────

/// Check and complete a message before it is stored: the body is sanitized
/// and its inline images normalized exactly as an immediate send would; a new
/// message needs recipients and a single-line subject; a reply needs its
/// parent, whose subject it takes when it has none of its own.
fn prepare_message(db: &Database, mut message: OutgoingMessage) -> Result<OutgoingMessage> {
    let body = crate::services::emails::outgoing_body(
        std::mem::take(&mut message.body),
        message.body_html.take(),
        std::mem::take(&mut message.inline_images),
    )?;
    message.body = body.text;
    message.body_html = body.html;
    message.inline_images = body.inline_images;

    if message.to.is_empty() {
        return Err(AppError::InvalidInput(
            "At least one recipient (To) is required".to_string(),
        ));
    }
    if db.get_account(&message.account_id)?.is_none() {
        return Err(AppError::NotFound(format!("Account {} not found", message.account_id)));
    }
    if let Some(parent_id) = message.reply_to_email_id.as_deref() {
        let parent = db
            .get_email(parent_id)?
            .ok_or_else(|| AppError::NotFound(format!("Email {parent_id} not found")))?;
        if message.subject.trim().is_empty() {
            message.subject = crate::sync::mime_builder::reply_subject(&parent.subject);
        }
    }
    crate::services::emails::validate_new_email(&message.to, &message.subject)?;
    Ok(message)
}

/// Queue a composed message: undo send (`Undo`) or scheduled send (`At`).
///
/// When the composer had saved it as a draft (`draft_id`), the draft's file
/// attachments are read into the message now — a later send must not depend
/// on files that may be gone — and the draft is deleted (locally and, through
/// `provider` when given, from the provider's Drafts folder) once the message
/// is safely queued, as an immediate send does. Undo / edit reopen the
/// composer from the queued copy, which saves a fresh draft.
pub async fn queue_outgoing(
    db: &Arc<Database>,
    message: OutgoingMessage,
    schedule: OutboxSchedule,
    draft_id: Option<&str>,
    provider: Option<&dyn EmailProvider>,
    now: i64,
) -> Result<OutboxEntry> {
    let (send_at, origin) = plan_send_at(now, schedule)?;
    let mut message = prepare_message(db, message)?;

    let draft = match draft_id {
        Some(id) => db.get_draft(id)?,
        None => None,
    };
    if let Some(draft) = &draft {
        message
            .attachments
            .extend(crate::services::emails::load_draft_attachments(&draft.attachments)?);
    }

    let id = uuid::Uuid::new_v4().to_string();
    db.insert_outbox(&NewOutboxRow {
        id: &id,
        message: &message,
        origin,
        send_at,
        now,
    })?;
    logger::log(
        "info",
        "sync",
        match origin {
            OutboxOrigin::Undo => "Message queued: it goes out when the undo window closes".to_string(),
            OutboxOrigin::Scheduled => "Message scheduled for later".to_string(),
        },
    );

    if let Some(draft) = draft {
        delete_queued_draft(db, &draft, provider).await;
    }

    wake_dispatcher_at(send_at, now);
    db.get_outbox_entry(&id)?
        .ok_or_else(|| AppError::NotFound(format!("Outbox message {id} vanished after queueing")))
}

/// The draft a queued message came from leaves Drafts. Best-effort: the
/// message is already queued, so a failure here is logged, not returned.
async fn delete_queued_draft(db: &Arc<Database>, draft: &crate::models::Draft, provider: Option<&dyn EmailProvider>) {
    let outcome = match db.get_account(&draft.account_id) {
        Ok(Some(account)) => crate::services::emails::delete_draft(db, &account, &draft.id, provider).await,
        Ok(None) => Ok(()),
        Err(e) => Err(e),
    };
    if let Err(e) = outcome {
        logger::log(
            "error",
            "drafts",
            format!("Message queued, but its draft could not be removed: {e}"),
        );
    }
}

/// Take a message back out of the outbox (undo, edit, delete) and return it so
/// the composer can reopen with it. Refused with `OutboxNotPending` once the
/// dispatcher has started sending it.
pub fn cancel_outbox_message(db: &Database, id: &str, now: i64) -> Result<OutgoingMessage> {
    match db.cancel_outbox(id, now)? {
        Some(payload) => {
            logger::log("info", "sync", "Queued message cancelled");
            Ok(serde_json::from_str(&payload)?)
        }
        None => match db.get_outbox_entry(id)? {
            Some(_) => Err(AppError::OutboxNotPending),
            None => Err(AppError::NotFound(format!("Outbox message {id} not found"))),
        },
    }
}

/// Send a waiting or failed message now (also how a failed one is retried).
pub fn send_outbox_message_now(db: &Database, id: &str, now: i64) -> Result<()> {
    if db.reschedule_outbox_now(id, now)? {
        wake_dispatcher_at(now, now);
        return Ok(());
    }
    match db.get_outbox_entry(id)? {
        Some(_) => Err(AppError::OutboxNotPending),
        None => Err(AppError::NotFound(format!("Outbox message {id} not found"))),
    }
}

/// Waiting and failed messages of one account, or of every enabled account.
pub fn list_outbox(db: &Database, account_id: Option<&str>) -> Result<Vec<OutboxEntry>> {
    let scope = match account_id {
        Some(id) => AccountScope::Account(id),
        None => AccountScope::AllEnabled,
    };
    db.list_outbox(scope)
}

// ── Dispatcher ───────────────────────────────────────────────────────────────

/// At start-up, before the first dispatch pass: rows left `sending` by a
/// crash or quit become `failed` (`interrupted`) and are announced. They are
/// never sent again automatically.
pub fn recover_interrupted_outbox(db: &Database, now: i64) -> Result<Vec<String>> {
    let ids = db.fail_interrupted_outbox(now)?;
    if ids.is_empty() {
        return Ok(ids);
    }
    let mut update = OutboxUpdated::default();
    for id in &ids {
        if let Some(entry) = db.get_outbox_entry(id)? {
            update.failed.push(OutboxFailed {
                id: id.clone(),
                account_id: entry.account_id,
                interrupted: true,
                message: "EmailOps stopped while this message was being sent; it may or may not have gone out"
                    .to_string(),
            });
        }
    }
    logger::log(
        "error",
        "sync",
        format!(
            "{} message(s) were being sent when EmailOps stopped — check Sent before retrying them",
            ids.len()
        ),
    );
    crate::services::events::emit(OUTBOX_UPDATED_EVENT, &update);
    Ok(ids)
}

/// One dispatch pass: send every row due at `now`, oldest first. Each row is
/// claimed (`sending`) before its provider is called; success marks it `sent`
/// (the send path stored the optimistic Sent copy), any failure marks it
/// `failed` with the error. Emits [`OUTBOX_UPDATED_EVENT`] when anything
/// changed and returns what happened.
pub async fn dispatch_due_outbox(
    db: &Arc<Database>,
    now: i64,
    providers: &dyn OutboxProviders,
) -> Result<OutboxUpdated> {
    let due = due_outbox(now, &db.scheduled_outbox()?);
    let mut update = OutboxUpdated::default();
    for id in due {
        let Some(payload) = db.claim_outbox(&id, now)? else {
            continue; // cancelled, rescheduled or taken by another pass meanwhile
        };
        match send_claimed(db, &payload, providers).await {
            Ok((message, thread_id)) => {
                if let Err(e) = db.mark_outbox_sent(&id, now) {
                    // The message left; the row stays `sending` and is
                    // reported as interrupted at the next start, never resent.
                    logger::log("error", "sync", format!("Message sent but not marked as sent: {e}"));
                }
                logger::log("success", "sync", "Queued message sent");
                update.sent.push(OutboxSent {
                    id,
                    account_id: message.account_id,
                    thread_id,
                });
            }
            Err((account_id, e)) => {
                let text = e.to_string();
                db.mark_outbox_failed(&id, &text, now)?;
                logger::log("error", "sync", format!("A queued message could not be sent: {text}"));
                update.failed.push(OutboxFailed {
                    id,
                    account_id,
                    interrupted: false,
                    message: text,
                });
            }
        }
    }
    match db.prune_finished_outbox(now - FINISHED_RETENTION_SECS) {
        Ok(_) => {}
        Err(e) => logger::log("debug", "sync", format!("Could not prune finished outbox rows: {e}")),
    }
    if !update.is_empty() {
        crate::services::events::emit(OUTBOX_UPDATED_EVENT, &update);
    }
    Ok(update)
}

/// Send one claimed row. The error carries the row's account (empty when the
/// payload could not even be read).
async fn send_claimed(
    db: &Arc<Database>,
    payload: &str,
    providers: &dyn OutboxProviders,
) -> std::result::Result<(OutgoingMessage, Option<String>), (String, AppError)> {
    let message: OutgoingMessage = serde_json::from_str(payload).map_err(|e| (String::new(), e.into()))?;
    let account_id = message.account_id.clone();
    let fail = |e: AppError| (account_id.clone(), e);
    let account = db
        .get_account(&message.account_id)
        .map_err(fail)?
        .ok_or_else(|| fail(AppError::NotFound(format!("Account {} not found", message.account_id))))?;
    let provider = providers.provider_for(&account).await.map_err(fail)?;
    let thread_id = crate::services::emails::send_outgoing(db, &message, provider)
        .await
        .map_err(fail)?;
    Ok((message, thread_id))
}

// ── Waking the dispatcher ────────────────────────────────────────────────────

/// Wakes the dispatcher loop between its periodic ticks.
pub fn dispatcher_waker() -> &'static tokio::sync::Notify {
    static WAKER: OnceLock<tokio::sync::Notify> = OnceLock::new();
    WAKER.get_or_init(tokio::sync::Notify::new)
}

/// Wake the dispatcher at `send_at`, so an undo window that closes sends the
/// message right away instead of at the next periodic tick.
pub fn wake_dispatcher_at(send_at: i64, now: i64) {
    let delay = u64::try_from(send_at - now).unwrap_or(0);
    if delay == 0 {
        dispatcher_waker().notify_one();
        return;
    }
    // Only meaningful inside a runtime (the app); a caller without one (a sync
    // unit test) relies on the periodic tick instead.
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    crate::runtime::spawn::spawn(async move {
        // One extra second so the row is due by the dispatcher's own clock.
        tokio::time::sleep(std::time::Duration::from_secs(delay + 1)).await;
        dispatcher_waker().notify_one();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::outbox::{OutboxFailureKind, OutboxStatus};
    use crate::models::Email;
    use crate::sync::provider::{EmailAttachment, FakeEmailProvider};

    const NOW: i64 = 1_800_000_000;

    // ── Planners ──

    #[test]
    fn due_rows_are_those_whose_time_has_come_oldest_first() {
        let rows = [
            ScheduledOutboxRow {
                id: "future".into(),
                send_at: NOW + 1,
            },
            ScheduledOutboxRow {
                id: "now".into(),
                send_at: NOW,
            },
            ScheduledOutboxRow {
                id: "overdue".into(),
                send_at: NOW - 3600,
            },
        ];
        assert_eq!(due_outbox(NOW, &rows), vec!["overdue".to_string(), "now".to_string()]);
        assert!(due_outbox(NOW, &[]).is_empty());
    }

    #[test]
    fn undo_send_waits_one_of_the_offered_delays() {
        assert_eq!(
            plan_send_at(NOW, OutboxSchedule::Undo { delay_secs: 10 }).unwrap(),
            (NOW + 10, OutboxOrigin::Undo)
        );
        for bad in [0, 3, 60, -5] {
            assert!(
                matches!(
                    plan_send_at(NOW, OutboxSchedule::Undo { delay_secs: bad }),
                    Err(AppError::InvalidInput(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_scheduled_send_must_be_in_the_future_and_within_a_year() {
        assert_eq!(
            plan_send_at(NOW, OutboxSchedule::At { send_at: NOW + 60 }).unwrap(),
            (NOW + 60, OutboxOrigin::Scheduled)
        );
        for bad in [NOW, NOW - 1, NOW + MAX_SCHEDULE_AHEAD_SECS + 1] {
            assert!(plan_send_at(NOW, OutboxSchedule::At { send_at: bad }).is_err(), "{bad}");
        }
    }

    // ── Fixtures ──

    struct FakeProviders(Arc<FakeEmailProvider>);

    struct Shared(Arc<FakeEmailProvider>);

    /// Hands the dispatcher a provider that records into the test's fake.
    #[async_trait]
    impl EmailProvider for Shared {
        async fn get_profile(&self) -> Result<(String, String)> {
            self.0.get_profile().await
        }

        async fn list_messages(
            &self,
            max_results: u32,
            page_token: Option<&str>,
            after_timestamp: Option<i64>,
            before_timestamp: Option<i64>,
            label_filter: Option<&str>,
        ) -> Result<(Vec<crate::sync::provider::MessageRef>, Option<String>)> {
            self.0
                .list_messages(max_results, page_token, after_timestamp, before_timestamp, label_filter)
                .await
        }

        async fn get_message(
            &self,
            message_id: &str,
        ) -> Result<(
            Email,
            crate::sync::provider::EmailCategory,
            Vec<crate::sync::provider::AttachmentInfo>,
        )> {
            self.0.get_message(message_id).await
        }

        async fn send_reply(
            &self,
            from_email: &str,
            from_name: Option<&str>,
            to: &[String],
            cc: &[String],
            target: &crate::sync::provider::ReplyTarget<'_>,
            subject: &str,
            body: &crate::sync::provider::EmailBody,
            attachments: &[EmailAttachment],
        ) -> Result<crate::sync::provider::SentMessageMeta> {
            self.0
                .send_reply(from_email, from_name, to, cc, target, subject, body, attachments)
                .await
        }

        async fn send_new_email(
            &self,
            from_email: &str,
            from_name: Option<&str>,
            to: &[String],
            cc: &[String],
            subject: &str,
            body: &crate::sync::provider::EmailBody,
            attachments: &[EmailAttachment],
        ) -> Result<crate::sync::provider::SentMessageMeta> {
            self.0
                .send_new_email(from_email, from_name, to, cc, subject, body, attachments)
                .await
        }

        async fn fetch_attachment_bytes(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
            self.0.fetch_attachment_bytes(message_id, attachment_id).await
        }
    }

    #[async_trait]
    impl OutboxProviders for FakeProviders {
        async fn provider_for(&self, _account: &Account) -> Result<Box<dyn EmailProvider>> {
            Ok(Box::new(Shared(Arc::clone(&self.0))))
        }
    }

    struct Unreachable;

    #[async_trait]
    impl OutboxProviders for Unreachable {
        async fn provider_for(&self, _account: &Account) -> Result<Box<dyn EmailProvider>> {
            Err(AppError::SyncError("offline".into()))
        }
    }

    fn parent() -> Email {
        Email {
            id: "parent-1".into(),
            account_id: "acc-1".into(),
            thread_id: "thread-1".into(),
            message_id: Some("<parent@example.com>".into()),
            references: None,
            subject: "Quarterly plan".into(),
            sender: "Ana".into(),
            sender_email: "ana@example.com".into(),
            recipients: vec!["me@example.com".into()],
            cc: vec![],
            body: "Thoughts?".into(),
            snippet: String::new(),
            timestamp: NOW - 3600,
            is_read: true,
            triage_status: None,
            category: "primary".into(),
            mailbox: "inbox".into(),
            is_sent: false,
            is_starred: false,
            headers: None,
        }
    }

    fn setup() -> (Arc<Database>, Arc<FakeEmailProvider>) {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(&[parent()]).unwrap();
        (Arc::new(db), Arc::new(FakeEmailProvider::new("acc-1", "Test")))
    }

    fn new_message() -> OutgoingMessage {
        OutgoingMessage {
            account_id: "acc-1".into(),
            reply_to_email_id: None,
            to: vec!["ben@example.com".into()],
            cc: vec![],
            subject: "Lunch".into(),
            body: "Friday?".into(),
            body_html: Some("<p>Friday?</p><script>x()</script>".into()),
            inline_images: vec![],
            attachments: vec![EmailAttachment {
                filename: "menu.txt".into(),
                mime_type: "text/plain".into(),
                data: "bWVudQ==".into(),
                content_id: None,
                is_inline: false,
            }],
        }
    }

    fn reply_message() -> OutgoingMessage {
        OutgoingMessage {
            reply_to_email_id: Some("parent-1".into()),
            to: vec!["ana@example.com".into()],
            subject: String::new(),
            body_html: None,
            attachments: vec![],
            ..new_message()
        }
    }

    async fn queue(db: &Arc<Database>, message: OutgoingMessage, schedule: OutboxSchedule) -> OutboxEntry {
        queue_outgoing(db, message, schedule, None, None, NOW).await.unwrap()
    }

    // ── Queueing ──

    #[tokio::test]
    async fn queueing_stores_a_sanitized_message_and_sends_nothing_yet() {
        let (db, fake) = setup();
        let entry = queue(&db, new_message(), OutboxSchedule::Undo { delay_secs: 10 }).await;
        assert_eq!(entry.send_at, NOW + 10);
        assert_eq!(entry.origin, OutboxOrigin::Undo);
        assert_eq!(entry.status, OutboxStatus::Scheduled);
        let restored = cancel_outbox_message(&db, &entry.id, NOW).unwrap();
        let html = restored.body_html.unwrap();
        assert!(!html.contains("script"), "{html}");
        assert_eq!(restored.attachments, new_message().attachments);
        assert!(fake.sent().is_empty());
    }

    #[tokio::test]
    async fn queueing_refuses_what_an_immediate_send_would_refuse() {
        let (db, _fake) = setup();
        let no_recipient = OutgoingMessage {
            to: vec![],
            ..new_message()
        };
        let header_injection = OutgoingMessage {
            subject: "Hi\r\nBcc: x@example.com".into(),
            ..new_message()
        };
        let orphan_reply = OutgoingMessage {
            reply_to_email_id: Some("gone".into()),
            ..reply_message()
        };
        for (msg, schedule) in [
            (no_recipient, OutboxSchedule::Undo { delay_secs: 5 }),
            (header_injection, OutboxSchedule::Undo { delay_secs: 5 }),
            (new_message(), OutboxSchedule::At { send_at: NOW - 1 }),
        ] {
            assert!(matches!(
                queue_outgoing(&db, msg, schedule, None, None, NOW).await,
                Err(AppError::InvalidInput(_))
            ));
        }
        assert!(matches!(
            queue_outgoing(
                &db,
                orphan_reply,
                OutboxSchedule::Undo { delay_secs: 5 },
                None,
                None,
                NOW
            )
            .await,
            Err(AppError::NotFound(_))
        ));
        assert!(list_outbox(&db, None).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_reply_without_its_own_subject_takes_the_parents() {
        let (db, _fake) = setup();
        let entry = queue(&db, reply_message(), OutboxSchedule::At { send_at: NOW + 60 }).await;
        assert_eq!(entry.subject, "Re: Quarterly plan");
        assert_eq!(entry.reply_to_email_id.as_deref(), Some("parent-1"));
    }

    #[tokio::test]
    async fn a_queued_draft_brings_its_files_and_leaves_drafts() {
        let (db, _fake) = setup();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agenda.txt");
        std::fs::write(&path, b"agenda").unwrap();
        let draft = db
            .save_user_draft(&crate::models::SaveDraftRequest {
                id: None,
                email_id: None,
                account_id: "acc-1".into(),
                to_addresses: vec!["ben@example.com".into()],
                cc_addresses: vec![],
                subject: "Lunch".into(),
                body: "Friday?".into(),
                body_html: None,
                provider_draft_id: None,
                attachments: None,
            })
            .unwrap();
        db.replace_draft_attachments(
            &draft.id,
            &[crate::models::DraftAttachment {
                id: String::new(),
                draft_id: draft.id.clone(),
                file_path: path.to_string_lossy().into_owned(),
                filename: "agenda.txt".into(),
                mime_type: "text/plain".into(),
            }],
        )
        .unwrap();

        let entry = queue_outgoing(
            &db,
            new_message(),
            OutboxSchedule::At { send_at: NOW + 60 },
            Some(&draft.id),
            None,
            NOW,
        )
        .await
        .unwrap();
        assert!(db.get_draft(&draft.id).unwrap().is_none(), "the draft leaves Drafts");
        assert_eq!(entry.attachment_count, 2);
        // The bytes travel with the queued message: the file can go away.
        drop(dir);
        let restored = cancel_outbox_message(&db, &entry.id, NOW).unwrap();
        assert_eq!(restored.attachments[1].filename, "agenda.txt");
        assert_eq!(restored.attachments[1].data, "YWdlbmRh");
    }

    // ── Dispatch ──

    #[tokio::test]
    async fn a_due_message_goes_out_once_through_the_send_path() {
        let (db, fake) = setup();
        let entry = queue(&db, new_message(), OutboxSchedule::Undo { delay_secs: 10 }).await;
        let providers = FakeProviders(Arc::clone(&fake));

        let early = dispatch_due_outbox(&db, NOW + 9, &providers).await.unwrap();
        assert!(early.is_empty(), "not before the undo window closes");
        assert!(fake.sent().is_empty());

        let pass = dispatch_due_outbox(&db, NOW + 10, &providers).await.unwrap();
        assert_eq!(pass.sent.len(), 1);
        assert_eq!(db.outbox_status(&entry.id).unwrap(), OutboxStatus::Sent);
        let sent = fake.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].to_emails, vec!["ben@example.com".to_string()]);
        assert_eq!(sent[0].attachments.len(), 1);
        // The optimistic Sent copy of the immediate send path is stored.
        let sent_rows: i64 = db
            .reader()
            .query_row(
                "SELECT COUNT(*) FROM emails WHERE account_id = 'acc-1' AND is_sent = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sent_rows, 1);

        let again = dispatch_due_outbox(&db, NOW + 60, &providers).await.unwrap();
        assert!(again.is_empty());
        assert_eq!(fake.sent().len(), 1, "never twice");
    }

    #[tokio::test]
    async fn a_queued_reply_keeps_its_threading() {
        let (db, fake) = setup();
        queue(&db, reply_message(), OutboxSchedule::Undo { delay_secs: 5 }).await;
        let pass = dispatch_due_outbox(&db, NOW + 5, &FakeProviders(Arc::clone(&fake)))
            .await
            .unwrap();
        assert_eq!(pass.sent[0].thread_id.as_deref(), Some("thread-1"));
        let sent = fake.sent();
        assert_eq!(sent[0].thread_id.as_deref(), Some("thread-1"));
        assert_eq!(sent[0].original_message_id.as_deref(), Some("<parent@example.com>"));
        assert_eq!(sent[0].subject, "Re: Quarterly plan");
    }

    #[tokio::test]
    async fn an_overdue_message_from_a_closed_app_is_sent_at_the_next_pass() {
        let (db, fake) = setup();
        queue(&db, new_message(), OutboxSchedule::At { send_at: NOW + 60 }).await;
        let pass = dispatch_due_outbox(&db, NOW + 86_400, &FakeProviders(Arc::clone(&fake)))
            .await
            .unwrap();
        assert_eq!(pass.sent.len(), 1);
    }

    #[tokio::test]
    async fn a_refused_send_fails_visibly_keeps_the_message_and_is_not_retried() {
        let (db, fake) = setup();
        let entry = queue(&db, new_message(), OutboxSchedule::Undo { delay_secs: 5 }).await;
        fake.fail_sends(Some("503 service unavailable"));
        let providers = FakeProviders(Arc::clone(&fake));
        let pass = dispatch_due_outbox(&db, NOW + 5, &providers).await.unwrap();
        assert_eq!(pass.failed.len(), 1);
        assert!(pass.failed[0].message.contains("503"));
        let failed = db.get_outbox_entry(&entry.id).unwrap().unwrap();
        assert_eq!(failed.status, OutboxStatus::Failed);
        assert_eq!(failed.failure_kind, Some(OutboxFailureKind::Error));

        fake.fail_sends(None);
        assert!(dispatch_due_outbox(&db, NOW + 600, &providers)
            .await
            .unwrap()
            .is_empty());
        assert!(fake.sent().is_empty(), "a failure is never retried on its own");

        // Retry is the user's call.
        send_outbox_message_now(&db, &entry.id, NOW + 700).unwrap();
        let retry = dispatch_due_outbox(&db, NOW + 700, &providers).await.unwrap();
        assert_eq!(retry.sent.len(), 1);
        assert_eq!(db.get_outbox_entry(&entry.id).unwrap().unwrap().attempts, 2);
    }

    #[tokio::test]
    async fn an_unreachable_provider_fails_the_row() {
        let (db, _fake) = setup();
        let entry = queue(&db, new_message(), OutboxSchedule::Undo { delay_secs: 5 }).await;
        let pass = dispatch_due_outbox(&db, NOW + 5, &Unreachable).await.unwrap();
        assert_eq!(pass.failed[0].account_id, "acc-1");
        assert_eq!(db.outbox_status(&entry.id).unwrap(), OutboxStatus::Failed);
    }

    #[tokio::test]
    async fn undo_wins_before_the_pass_and_loses_after_it_claimed_the_row() {
        let (db, fake) = setup();
        let providers = FakeProviders(Arc::clone(&fake));
        let undone = queue(&db, new_message(), OutboxSchedule::Undo { delay_secs: 10 }).await;
        assert_eq!(
            cancel_outbox_message(&db, &undone.id, NOW + 9).unwrap().subject,
            "Lunch"
        );
        assert!(dispatch_due_outbox(&db, NOW + 10, &providers).await.unwrap().is_empty());
        assert!(fake.sent().is_empty(), "an undone message never leaves");

        let late = queue(&db, new_message(), OutboxSchedule::Undo { delay_secs: 10 }).await;
        db.claim_outbox(&late.id, NOW + 10).unwrap();
        assert!(matches!(
            cancel_outbox_message(&db, &late.id, NOW + 10),
            Err(AppError::OutboxNotPending)
        ));
        assert!(matches!(
            cancel_outbox_message(&db, "missing", NOW),
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn a_message_interrupted_mid_send_is_reported_and_never_resent() {
        let (db, fake) = setup();
        let entry = queue(&db, new_message(), OutboxSchedule::Undo { delay_secs: 5 }).await;
        // The app stopped after claiming the row, before the send finished.
        db.claim_outbox(&entry.id, NOW + 5).unwrap();

        assert_eq!(
            recover_interrupted_outbox(&db, NOW + 100).unwrap(),
            vec![entry.id.clone()]
        );
        let pass = dispatch_due_outbox(&db, NOW + 100, &FakeProviders(Arc::clone(&fake)))
            .await
            .unwrap();
        assert!(pass.is_empty());
        assert!(fake.sent().is_empty());
        let row = db.get_outbox_entry(&entry.id).unwrap().unwrap();
        assert_eq!(row.failure_kind, Some(OutboxFailureKind::Interrupted));
        assert_eq!(
            list_outbox(&db, Some("acc-1")).unwrap().len(),
            1,
            "visible for retry/edit/delete"
        );
    }

    #[test]
    fn a_dispatch_pass_announces_what_it_sent() {
        let _g = crate::services::events::seam_test_lock();
        let sink = crate::services::events::install_for_testing();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (db, fake) = setup();
        rt.block_on(async {
            queue(&db, new_message(), OutboxSchedule::Undo { delay_secs: 5 }).await;
            dispatch_due_outbox(&db, NOW + 5, &FakeProviders(Arc::clone(&fake)))
                .await
                .unwrap();
        });
        // Other tests may emit on the shared sink meanwhile; ours is among them.
        let events = sink.payloads_for(OUTBOX_UPDATED_EVENT);
        assert!(
            events.iter().any(|e| e["sent"][0]["accountId"] == "acc-1"),
            "{events:?}"
        );
    }
}
