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
//!   message must work offline, and a failed push is logged, never fatal. It is
//!   not lost either: the row keeps a pending marker
//!   (`emails.read_push_pending_since`) until the provider has the change, and
//!   every sync retries what is pending ([`retry_pending_read_pushes`]).
//! - **delete** is provider-first, mirroring [`super::folders::move_email`]. A
//!   delete that only happened locally would silently diverge from the account
//!   with no retry, so the row stays visible if the provider refuses.

use std::sync::Arc;

use crate::db::emails::mailbox_state::{PendingReadPush, PendingStarPush};
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, Email};
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
        // The email (or its account) is gone, e.g. the account was just
        // removed: there is nothing to mark, locally or at the provider.
        Err(e @ AppError::NotFound(_)) => return Err(e),
        Err(e) => {
            logger::log(
                "error",
                "sync",
                format!("Could not reach the mail provider — the read state will be sent on the next sync: {e}"),
            );
            return mark_read(db, email_id, PushVia::NextSync).await;
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
pub(super) async fn write_provider(
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
/// writes — the change then stays local. The local write happens either way.
pub async fn mark_as_read_with_provider(
    db: &Arc<Database>,
    email_id: &str,
    provider: Option<&dyn EmailProvider>,
) -> Result<()> {
    let via = provider.map_or(PushVia::Never, PushVia::Now);
    mark_read(db, email_id, via).await
}

/// How a read-state or star change reaches the provider.
pub(super) enum PushVia<'a> {
    /// The provider has no mailbox writes: the change is local-only.
    Never,
    /// The provider has them but cannot be reached right now (offline, expired
    /// credentials): the change is queued for the next sync.
    NextSync,
    Now(&'a dyn EmailProvider),
}

async fn mark_read(db: &Arc<Database>, email_id: &str, via: PushVia<'_>) -> Result<()> {
    set_flag(db, email_id, PushedFlag::Read, true, via).await
}

/// A per-message flag whose change is local-first and pushed to the provider
/// with a pending marker: read state (V029) and the star (V030).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PushedFlag {
    Read,
    Star,
}

impl PushedFlag {
    fn current(self, email: &Email) -> bool {
        match self {
            Self::Read => email.is_read,
            Self::Star => email.is_starred,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Read => "read state",
            Self::Star => "star",
        }
    }

    fn write_local(self, db: &Database, email_id: &str, on: bool) -> Result<()> {
        match self {
            Self::Read => db.set_read_local(email_id, on),
            Self::Star => db.set_starred_local(email_id, on),
        }
    }

    fn write_pending(self, db: &Database, email_id: &str, on: bool, now: i64) -> Result<()> {
        match self {
            Self::Read => db.set_read_pending_push(email_id, on, now),
            Self::Star => db.set_starred_pending_push(email_id, on, now),
        }
    }

    fn clear_pending(self, db: &Database, email_id: &str) -> Result<()> {
        match self {
            Self::Read => db.clear_read_push_pending(email_id),
            Self::Star => db.clear_star_push_pending(email_id),
        }
    }

    async fn push(self, provider: &dyn EmailProvider, email_id: &str, on: bool) -> Result<()> {
        match self {
            Self::Read => provider.set_read_state(email_id, on).await,
            Self::Star => provider.set_starred(email_id, on).await,
        }
    }
}

/// Set one email's read state or star locally and, through `via`, at the
/// provider. The local write always happens; a failed push leaves the row
/// pending for the sync to retry.
pub(super) async fn set_flag(
    db: &Arc<Database>,
    email_id: &str,
    flag: PushedFlag,
    on: bool,
    via: PushVia<'_>,
) -> Result<()> {
    let email = load_email(db, email_id)?;
    if flag.current(&email) == on {
        // Opening a thread re-marks every message in it; skipping the no-op
        // keeps that from firing one provider write per message per open. A
        // push still owed for this row is retried by the sync, not from here.
        return Ok(());
    }
    // Locally-composed Sent rows have no counterpart at the provider yet.
    let via = if email.id.starts_with(LOCAL_SENT_ID_PREFIX) {
        PushVia::Never
    } else {
        via
    };
    let now = crate::services::clock::now_secs();
    match via {
        PushVia::Never => flag.write_local(db, email_id, on)?,
        PushVia::NextSync => flag.write_pending(db, email_id, on, now)?,
        PushVia::Now(provider) => {
            // Marked pending *before* the push, in the same statement as the
            // flag, so a crash or a concurrent sync never sees a local change
            // the provider does not have yet without its marker.
            flag.write_pending(db, email_id, on, now)?;
            match flag.push(provider, &email.id, on).await {
                // "No such message" settles it too: there is nothing to retry.
                Ok(()) | Err(AppError::NotFound(_)) => flag.clear_pending(db, email_id)?,
                Err(e) => {
                    // Best-effort by design: reading or starring mail must not
                    // depend on the network. The row stays pending and the
                    // next sync retries the push.
                    logger::log(
                        "error",
                        "sync",
                        format!(
                            "Could not send the {} of a message to the provider (will retry on the next sync): {e}",
                            flag.noun()
                        ),
                    );
                }
            }
        }
    }
    Ok(())
}

/// A read-state change older than this is given up on: a push that has failed
/// on every sync for a week (a read-only mailbox, a revoked permission) is not
/// going to land, and retrying it forever would cost a request per sync.
const READ_PUSH_MAX_AGE_SECS: i64 = 7 * 86_400;

/// Most pending read-state changes one sync pushes. A backlog larger than
/// this (a long offline session) drains over the following syncs.
const MAX_READ_PUSHES_PER_SYNC: usize = 100;

/// A sync stops retrying after this many failed pushes: when the provider is
/// down every remaining push would fail the same way, each after its own
/// retries and timeouts.
const MAX_READ_PUSH_FAILURES_PER_SYNC: usize = 3;

/// What to do with the read-state (or star) changes still owed to the provider.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ReadPushPlan {
    /// `(email id, state)` to push, oldest change first.
    pub push: Vec<(String, bool)>,
    /// Email ids whose change is too old to keep retrying.
    pub give_up: Vec<String>,
}

/// Pure: split pending changes into the ones to push now and the ones that
/// have been failing for longer than [`READ_PUSH_MAX_AGE_SECS`]. A marker
/// dated in the future (the clock moved back) is still pushed.
pub(super) fn plan_read_push_retries(pending: &[PendingReadPush], now: i64) -> ReadPushPlan {
    plan_push_retries(pending.iter().map(|r| (&r.id, r.is_read, r.pending_since)), now)
}

/// [`plan_read_push_retries`] for pending stars — the same rules.
pub(super) fn plan_star_push_retries(pending: &[PendingStarPush], now: i64) -> ReadPushPlan {
    plan_push_retries(pending.iter().map(|r| (&r.id, r.is_starred, r.pending_since)), now)
}

fn plan_push_retries<'a>(pending: impl Iterator<Item = (&'a String, bool, i64)>, now: i64) -> ReadPushPlan {
    let mut plan = ReadPushPlan::default();
    for (id, state, since) in pending {
        if now - since > READ_PUSH_MAX_AGE_SECS {
            plan.give_up.push(id.clone());
        } else {
            plan.push.push((id.clone(), state));
        }
    }
    plan
}

/// Push the read-state changes a previous attempt could not deliver (offline,
/// a 5xx, expired credentials). Runs at the start of every sync, before the
/// server-to-local refresh, which leaves pending rows alone in any case.
///
/// Non-fatal: a push that fails again stays pending for the next sync.
pub(super) async fn retry_pending_read_pushes(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
    now: i64,
) {
    if !provider_supports_mailbox_writes(&account.provider) {
        return;
    }
    let plan = match db.pending_read_pushes(&account.id, MAX_READ_PUSHES_PER_SYNC) {
        Ok(rows) if rows.is_empty() => return,
        Ok(rows) => plan_read_push_retries(&rows, now),
        Err(e) => {
            super::emit_account_log(
                "warn",
                "sync",
                &account.email,
                &format!("Could not read the read-state changes still to send: {e}"),
            );
            return;
        }
    };
    retry_flag_pushes(db, account, provider, PushedFlag::Read, plan).await;
}

/// [`retry_pending_read_pushes`] for stars (V030): the same caps and rules.
pub(super) async fn retry_pending_star_pushes(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
    now: i64,
) {
    if !provider_supports_mailbox_writes(&account.provider) {
        return;
    }
    let plan = match db.pending_star_pushes(&account.id, MAX_READ_PUSHES_PER_SYNC) {
        Ok(rows) if rows.is_empty() => return,
        Ok(rows) => plan_star_push_retries(&rows, now),
        Err(e) => {
            super::emit_account_log(
                "warn",
                "sync",
                &account.email,
                &format!("Could not read the star changes still to send: {e}"),
            );
            return;
        }
    };
    retry_flag_pushes(db, account, provider, PushedFlag::Star, plan).await;
}

async fn retry_flag_pushes(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
    flag: PushedFlag,
    plan: ReadPushPlan,
) {
    let mut settled: Vec<&str> = plan.give_up.iter().map(String::as_str).collect();
    if !plan.give_up.is_empty() {
        super::emit_account_log(
            "warn",
            "sync",
            &account.email,
            &format!(
                "Gave up sending the {} of {} message(s) after {} days of failed attempts",
                flag.noun(),
                plan.give_up.len(),
                READ_PUSH_MAX_AGE_SECS / 86_400
            ),
        );
    }

    let mut pushed: u32 = 0;
    let mut failures: usize = 0;
    for (id, on) in &plan.push {
        match flag.push(provider, id, *on).await {
            Ok(()) => {
                pushed += 1;
                settled.push(id);
            }
            // The provider no longer has the message: nothing left to push.
            Err(AppError::NotFound(_)) => settled.push(id),
            Err(e) => {
                super::emit_account_log(
                    "warn",
                    "sync",
                    &account.email,
                    &format!(
                        "Could not send a {} change (will retry on the next sync): {e}",
                        flag.noun()
                    ),
                );
                failures += 1;
                if failures >= MAX_READ_PUSH_FAILURES_PER_SYNC {
                    break;
                }
            }
        }
    }

    for id in settled {
        if let Err(e) = flag.clear_pending(db, id) {
            super::emit_account_log(
                "warn",
                "sync",
                &account.email,
                &format!("Could not record that a {} change was sent: {e}", flag.noun()),
            );
        }
    }
    if pushed > 0 {
        super::emit_account_log(
            "debug",
            "sync",
            &account.email,
            &format!(
                "Sent {pushed} {} change(s) that could not be delivered earlier",
                flag.noun()
            ),
        );
    }
}

/// Delete one email: move it to the provider's Trash first, then soft-delete
/// the local row. A provider failure aborts the whole operation so the user
/// keeps seeing a message that still exists in their account — except "no
/// such message", which means the account already lost it.
pub async fn delete_email_with_provider(
    db: &Arc<Database>,
    email_id: &str,
    provider: Option<&dyn EmailProvider>,
) -> Result<()> {
    let email = load_email(db, email_id)?;

    if let Some(provider) = pushable(provider, &email) {
        match provider.trash_message(&email.id, email.message_id.as_deref()).await {
            Ok(()) => {}
            // Already deleted, or moved and re-keyed, in another client: there
            // is nothing left under this id to trash, and keeping the row
            // would leave a message the user can never get rid of.
            Err(AppError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    db.delete_email(email_id)
}

pub(super) fn load_email(db: &Arc<Database>, email_id: &str) -> Result<Email> {
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

    use super::{
        plan_read_push_retries, retry_pending_read_pushes, retry_pending_star_pushes, PendingReadPush, PushVia,
        ReadPushPlan,
    };

    fn pending_ids(db: &Arc<Database>, account_id: &str) -> Vec<String> {
        db.pending_read_pushes(account_id, 100)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect()
    }

    fn account(id: &str, provider: &str) -> crate::models::Account {
        crate::models::Account {
            id: id.to_string(),
            provider: provider.to_string(),
            email: format!("{id}@example.com"),
            name: "Test".to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        }
    }

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
            references: None,
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
            is_starred: false,
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

    // Regression: marking read an email whose account was just removed logged
    // "Read state will stay local — could not reach the mail provider", which
    // blamed the network for a row that no longer exists.
    // Sync `#[test]` on its own runtime so the seam lock is never held across
    // an await point (`clippy::await_holding_lock`).
    #[test]
    fn mark_read_of_a_missing_email_is_not_found_without_a_provider_error() {
        let _seam = crate::services::events::seam_test_lock();
        let logs = crate::services::logger::install_for_testing();
        let db = test_db("acc-1");

        let result = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build test runtime")
            .block_on(super::mark_as_read(&db, "gone", None));

        assert!(
            matches!(result, Err(crate::models::error::AppError::NotFound(_))),
            "got {result:?}"
        );
        // The logger is process-global, so other tests' lines can land here too.
        let provider_errors: Vec<_> = logs
            .events()
            .into_iter()
            .filter(|e| e.message.contains("Email gone not found"))
            .collect();
        assert!(
            provider_errors.is_empty(),
            "missing email must not log a provider error, got: {provider_errors:?}"
        );
    }

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
    async fn a_failed_push_leaves_the_row_pending_for_the_next_sync() {
        // Regression: the failure was only logged, so the message stayed
        // unread in every other client forever.
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("network unreachable");

        mark_read(&db, "m-1", Some(&provider)).await.unwrap();

        assert_eq!(pending_ids(&db, "acc-1"), vec!["m-1".to_string()]);
    }

    #[tokio::test]
    async fn a_delivered_push_leaves_nothing_pending() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        mark_read(&db, "m-1", Some(&provider)).await.unwrap();

        assert!(pending_ids(&db, "acc-1").is_empty());
    }

    #[tokio::test]
    async fn a_push_to_a_message_the_provider_lost_is_not_kept_pending() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes_as_not_found();

        mark_read(&db, "m-1", Some(&provider)).await.unwrap();

        assert!(db.get_email("m-1").unwrap().unwrap().is_read);
        assert!(pending_ids(&db, "acc-1").is_empty());
    }

    #[tokio::test]
    async fn an_unreachable_provider_queues_the_push() {
        // Offline, or credentials that need refreshing: the provider could not
        // even be built, but it does take mailbox writes.
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();

        super::mark_read(&db, "m-1", PushVia::NextSync).await.unwrap();

        assert!(db.get_email("m-1").unwrap().unwrap().is_read);
        assert_eq!(pending_ids(&db, "acc-1"), vec!["m-1".to_string()]);
    }

    #[tokio::test]
    async fn a_provider_without_mailbox_writes_queues_nothing() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();

        mark_read(&db, "m-1", None).await.unwrap();

        assert!(pending_ids(&db, "acc-1").is_empty());
    }

    // ── retry of pending pushes ───────────────────────────────────────────

    fn pending(id: &str, since: i64) -> PendingReadPush {
        PendingReadPush {
            id: id.to_string(),
            is_read: true,
            pending_since: since,
        }
    }

    #[test]
    fn retry_plan_pushes_recent_changes_and_gives_up_on_week_old_ones() {
        const DAY: i64 = 86_400;
        let now = 100 * DAY;
        let cases = [
            ("just queued", now, true),
            ("six days old", now - 6 * DAY, true),
            ("exactly a week old", now - 7 * DAY, true),
            ("over a week old", now - 7 * DAY - 1, false),
            ("queued in the future (clock moved back)", now + DAY, true),
        ];
        for (label, since, pushed) in cases {
            let plan = plan_read_push_retries(&[pending("m-1", since)], now);
            let expected = if pushed {
                ReadPushPlan {
                    push: vec![("m-1".to_string(), true)],
                    give_up: vec![],
                }
            } else {
                ReadPushPlan {
                    push: vec![],
                    give_up: vec!["m-1".to_string()],
                }
            };
            assert_eq!(plan, expected, "{label}");
        }
    }

    #[test]
    fn retry_plan_keeps_the_order_and_the_state_of_each_change() {
        let mut unread = pending("m-2", 20);
        unread.is_read = false;
        let plan = plan_read_push_retries(&[pending("m-1", 10), unread], 30);
        assert_eq!(plan.push, vec![("m-1".to_string(), true), ("m-2".to_string(), false)]);
    }

    #[test]
    fn retry_plan_of_nothing_is_empty() {
        assert_eq!(plan_read_push_retries(&[], 30), ReadPushPlan::default());
    }

    #[tokio::test]
    async fn the_next_sync_delivers_a_push_that_failed() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("network unreachable");
        mark_read(&db, "m-1", Some(&provider)).await.unwrap();
        provider.restore_mailbox_writes();

        retry_pending_read_pushes(&db, &account("acc-1", "imap"), &provider, 2_000).await;

        assert_eq!(
            provider.mailbox_ops(),
            vec![FakeMailboxOp::SetReadState {
                message_id: "m-1".to_string(),
                read: true
            }]
        );
        assert!(pending_ids(&db, "acc-1").is_empty(), "delivered, so no longer pending");
    }

    #[tokio::test]
    async fn star_push_retries_follow_the_read_state_rules() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", true), email("m-old", "acc-1", true)])
            .unwrap();
        db.set_starred_pending_push("m-1", true, 1_000).unwrap();
        db.set_starred_pending_push("m-old", true, 1_000 - 8 * 86_400).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        retry_pending_star_pushes(&db, &account("acc-1", "gmail"), &provider, 2_000).await;

        assert_eq!(
            provider.mailbox_ops(),
            vec![FakeMailboxOp::SetStarred {
                message_id: "m-1".to_string(),
                starred: true
            }],
            "the week-old change is given up without a call"
        );
        assert!(db.pending_star_pushes("acc-1", 10).unwrap().is_empty());
        assert!(
            db.get_email("m-old").unwrap().unwrap().is_starred,
            "the local star is kept"
        );
    }

    #[tokio::test]
    async fn a_star_push_that_fails_stays_pending() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", true)]).unwrap();
        db.set_starred_pending_push("m-1", true, 1_000).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("503");

        retry_pending_star_pushes(&db, &account("acc-1", "outlook"), &provider, 2_000).await;

        assert_eq!(db.pending_star_pushes("acc-1", 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_push_that_fails_again_stays_pending() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        db.mark_as_read_pending_push("m-1", 1_000).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("503");

        retry_pending_read_pushes(&db, &account("acc-1", "outlook"), &provider, 2_000).await;

        assert_eq!(pending_ids(&db, "acc-1"), vec!["m-1".to_string()]);
    }

    #[tokio::test]
    async fn a_retry_the_provider_answers_with_no_such_message_is_dropped() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        db.mark_as_read_pending_push("m-1", 1_000).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes_as_not_found();

        retry_pending_read_pushes(&db, &account("acc-1", "outlook"), &provider, 2_000).await;

        assert!(pending_ids(&db, "acc-1").is_empty());
    }

    #[tokio::test]
    async fn a_week_old_pending_push_is_given_up_without_a_provider_call() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        db.mark_as_read_pending_push("m-1", 1_000).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        retry_pending_read_pushes(&db, &account("acc-1", "imap"), &provider, 1_000 + 8 * 86_400).await;

        assert!(provider.mailbox_ops().is_empty());
        assert!(pending_ids(&db, "acc-1").is_empty());
        assert!(db.get_email("m-1").unwrap().unwrap().is_read, "the local state is kept");
    }

    #[tokio::test]
    async fn a_down_provider_is_not_asked_once_per_pending_row() {
        let db = test_db("acc-1");
        let rows: Vec<_> = (0..10).map(|i| email(&format!("m-{i}"), "acc-1", false)).collect();
        db.insert_emails_batch(&rows).unwrap();
        for row in &rows {
            db.mark_as_read_pending_push(&row.id, 1_000).unwrap();
        }
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("connection refused");
        let calls = provider.call_log();

        retry_pending_read_pushes(&db, &account("acc-1", "imap"), &provider, 2_000).await;

        let attempts = calls
            .read()
            .unwrap()
            .iter()
            .filter(|c| c.as_str() == "set_read_state")
            .count();
        assert_eq!(attempts, super::MAX_READ_PUSH_FAILURES_PER_SYNC);
        assert_eq!(pending_ids(&db, "acc-1").len(), 10, "every change is still owed");
    }

    #[tokio::test]
    async fn an_unknown_provider_has_nothing_retried() {
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", false)]).unwrap();
        db.mark_as_read_pending_push("m-1", 1_000).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        retry_pending_read_pushes(&db, &account("acc-1", "exchange-ews"), &provider, 2_000).await;

        assert!(provider.mailbox_ops().is_empty());
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
                message_id: "m-1".to_string(),
                message_id_header: Some("<m-1@example.com>".to_string()),
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
    async fn delete_goes_through_when_the_provider_no_longer_has_the_message() {
        // Deleted or moved in another client: a refusal here would leave a row
        // the user can never delete.
        let db = test_db("acc-1");
        db.insert_emails_batch(&[email("m-1", "acc-1", true)]).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes_as_not_found();

        delete(&db, "m-1", Some(&provider)).await.unwrap();

        assert!(inbox_ids(&db, "acc-1").is_empty());
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
