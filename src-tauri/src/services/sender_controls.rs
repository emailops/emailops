//! Block sender, and the sender facts the reading pane shows (blocked?
//! unsubscribe option? already unsubscribed?).
//!
//! A block is per account (like Gmail's). Its effect is applied by the app,
//! not the provider: Gmail's own block creates a filter, which needs the
//! `gmail.settings.basic` scope the app does not ask for, and IMAP has no
//! filters at all. So every message that arrives in the inbox from a blocked
//! address is marked junk locally and filed in the provider's Spam/Junk folder
//! (`file_blocked_arrivals`, run by the sync right after a batch is stored) —
//! the same move "Report junk" makes, so other clients agree. See DECISIONS
//! 2026-10-01 "Block sender files arrivals in the provider's spam folder".

use std::collections::HashSet;
use std::sync::Arc;

use serde::Serialize;
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, Email};
use crate::services::emails::{file_in_spam, restore_from_spam, ProviderAccess, SpamFiling};
use crate::services::junk;
use crate::services::logger;
use crate::services::unsubscribe::{parse_unsubscribe, UnsubscribeOption};
use crate::sync::provider::EmailProvider;

/// Existing messages moved by one block (or unblock) at most. A sender with
/// more keeps the rest where they are; the log says so.
pub const MAX_EXISTING_MOVES: usize = 500;

/// Mailboxes a block sweeps the sender's existing mail out of.
const BLOCK_SWEEPS: &[&str] = &["inbox", "archive"];

/// What the reading pane needs to know about a message's sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct SenderStatus {
    /// Normalized (lowercase) sender address.
    pub address: String,
    pub blocked: bool,
    /// How to leave the list this message came from, if it says.
    pub unsubscribe: Option<UnsubscribeOption>,
    /// When the user asked to leave this sender's list, if ever.
    pub unsubscribed_at: Option<i64>,
}

/// The outcome of moving a sender's existing mail on block or unblock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct SenderMoveReport {
    /// Messages moved (to Spam on block, back to the inbox on unblock).
    pub moved: u32,
    /// Messages marked junk locally but left in place: the provider cannot
    /// file them (no mailbox writes, or an IMAP server with no Junk folder).
    pub local_only: u32,
    /// Messages the provider refused or could not be reached for.
    pub failed: u32,
}

/// Pure: the address a block is stored under — trimmed, `<…>` stripped,
/// lowercase — or `InvalidInput` when it is not one plain address.
pub fn normalize_address(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(trimmed)
        .trim()
        .to_lowercase();
    let valid = !inner.is_empty()
        && inner.len() <= 254
        && !inner
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, ',' | ';' | '<' | '>' | '"'))
        && matches!(inner.split_once('@'), Some((local, domain)) if !local.is_empty() && !domain.is_empty() && !domain.contains('@'));
    if valid {
        Ok(inner)
    } else {
        Err(AppError::InvalidInput(format!("Not an email address: {raw}")))
    }
}

/// Pure: the messages of a freshly stored batch that a block catches —
/// received inbox mail whose sender is on the account's block list.
pub fn plan_blocked_arrivals<'a>(emails: &'a [Email], blocked: &HashSet<String>) -> Vec<&'a Email> {
    if blocked.is_empty() {
        return Vec::new();
    }
    emails
        .iter()
        .filter(|e| e.mailbox == "inbox" && !e.is_sent)
        .filter(|e| blocked.contains(&e.sender_email.trim().to_lowercase()))
        .collect()
}

/// The sender facts of one message, for the reading pane.
pub fn sender_status(db: &Database, account_id: &str, email_id: &str) -> Result<SenderStatus> {
    let email = owned_email(db, account_id, email_id)?;
    let address = email.sender_email.trim().to_lowercase();
    let headers = db
        .get_email_headers_batch(std::slice::from_ref(&email.id))?
        .remove(&email.id);
    let unsubscribe = headers
        .as_ref()
        .and_then(|h| parse_unsubscribe(h.list_unsubscribe.as_deref(), h.list_unsubscribe_post.as_deref()))
        .map(|m| m.option());
    Ok(SenderStatus {
        blocked: db.is_sender_blocked(account_id, &address)?,
        unsubscribed_at: db.sender_unsubscribed_at(account_id, &address)?,
        unsubscribe,
        address,
    })
}

/// A message of `account_id`, or `NotFound`.
pub(crate) fn owned_email(db: &Database, account_id: &str, email_id: &str) -> Result<Email> {
    db.get_email(email_id)?
        .filter(|e| e.account_id == account_id)
        .ok_or_else(|| AppError::NotFound(format!("Email {email_id} not found")))
}

/// Block `raw_address` in `account`, then — when `move_existing` — mark the
/// sender's inbox and archived mail junk and file it in Spam. The block is
/// stored first, so a provider failure never loses it.
pub async fn block_sender(
    db: &Arc<Database>,
    account: &Account,
    raw_address: &str,
    move_existing: bool,
    access: ProviderAccess<'_>,
    now: i64,
) -> Result<SenderMoveReport> {
    let address = normalize_address(raw_address)?;
    db.insert_blocked_sender(&account.id, &address, now)?;
    logger::log("success", "account", format!("[{}] Blocked {address}", account.email));
    if !move_existing {
        return Ok(SenderMoveReport::default());
    }
    let ids = db.email_ids_from_sender_in(&account.id, &address, BLOCK_SWEEPS, MAX_EXISTING_MOVES)?;
    let mut report = SenderMoveReport::default();
    for id in ids {
        let outcome = match db.get_email(&id)? {
            Some(email) => junk_and_file(db, &email, &access).await,
            None => continue,
        };
        tally(
            &mut report,
            outcome,
            account,
            "file a message from a blocked sender as spam",
        );
    }
    log_report(account, &address, "Moved to spam", &report);
    Ok(report)
}

/// Unblock, then — when `restore` — bring the sender's mail back out of Spam
/// and forget the junk mark the block gave it.
pub async fn unblock_sender(
    db: &Arc<Database>,
    account: &Account,
    raw_address: &str,
    restore: bool,
    access: ProviderAccess<'_>,
) -> Result<SenderMoveReport> {
    let address = normalize_address(raw_address)?;
    db.delete_blocked_sender(&account.id, &address)?;
    logger::log("success", "account", format!("[{}] Unblocked {address}", account.email));
    if !restore {
        return Ok(SenderMoveReport::default());
    }
    let ids = db.email_ids_from_sender_in(&account.id, &address, &["spam"], MAX_EXISTING_MOVES)?;
    let mut report = SenderMoveReport::default();
    for id in ids {
        let Some(email) = db.get_email(&id)? else {
            continue;
        };
        let outcome = match provider_for_write(&access) {
            Ok(provider) => match restore_from_spam(db, &email, provider).await {
                Ok(new_id) => junk::clear_feedback(db, &account.id, &new_id).map(|()| SpamFiling::Filed),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        tally(&mut report, outcome, account, "bring a message back from spam");
    }
    // Mail a refused block left in place (provider offline, no Junk folder)
    // carries only the local junk mark: forget it, no provider needed.
    let left_in_place = db.email_ids_from_sender_in(&account.id, &address, BLOCK_SWEEPS, MAX_EXISTING_MOVES)?;
    let marked = db.get_junk_verdicts_batch(&left_in_place)?;
    for id in &left_in_place {
        if marked.get(id).and_then(|v| v.user_override.as_deref()) == Some("junk") {
            junk::clear_feedback(db, &account.id, id)?;
        }
    }
    log_report(account, &address, "Moved back to the inbox", &report);
    Ok(report)
}

/// Sync hook: file every message of a freshly stored batch that comes from a
/// blocked sender. Never fails the sync — each failure is logged, and the
/// message stays marked junk locally. Returns how many were caught.
pub async fn file_blocked_arrivals(
    db: &Arc<Database>,
    account: &Account,
    provider: &dyn EmailProvider,
    emails: &[Email],
) -> usize {
    let blocked = match db.blocked_addresses(&account.id) {
        Ok(set) => set,
        Err(e) => {
            logger::log(
                "error",
                "sync",
                format!("[{}] Could not read the blocked senders: {e}", account.email),
            );
            return 0;
        }
    };
    let caught = plan_blocked_arrivals(emails, &blocked);
    let access = if crate::sync::provider::provider_supports_mailbox_writes(&account.provider) {
        ProviderAccess::Ready(provider)
    } else {
        ProviderAccess::LocalOnly
    };
    let mut report = SenderMoveReport::default();
    for email in &caught {
        let outcome = junk_and_file(db, email, &access).await;
        tally(&mut report, outcome, account, "file mail from a blocked sender as spam");
    }
    if !caught.is_empty() {
        logger::log(
            "info",
            "sync",
            format!(
                "[{}] {} message(s) from blocked senders filed as spam",
                account.email,
                caught.len()
            ),
        );
    }
    caught.len()
}

/// Mark one message junk (the user's judgement, recorded first so it survives
/// a provider failure), then file it in Spam.
async fn junk_and_file(db: &Arc<Database>, email: &Email, access: &ProviderAccess<'_>) -> Result<SpamFiling> {
    junk::set_feedback(db, &email.account_id, &email.id, true).await?;
    file_in_spam(db, email, provider_for_write(access)?).await
}

fn provider_for_write<'a>(access: &ProviderAccess<'a>) -> Result<Option<&'a dyn EmailProvider>> {
    match access {
        ProviderAccess::LocalOnly => Ok(None),
        ProviderAccess::Unreachable(e) => Err(AppError::SyncError(format!(
            "the mail provider could not be reached: {e}"
        ))),
        ProviderAccess::Ready(provider) => Ok(Some(*provider)),
    }
}

fn tally(report: &mut SenderMoveReport, outcome: Result<SpamFiling>, account: &Account, what: &str) {
    match outcome {
        Ok(SpamFiling::Filed) => report.moved += 1,
        Ok(SpamFiling::LocalOnly) => report.local_only += 1,
        Err(e) => {
            report.failed += 1;
            logger::log("error", "sync", format!("[{}] Could not {what}: {e}", account.email));
        }
    }
}

fn log_report(account: &Account, address: &str, verb: &str, report: &SenderMoveReport) {
    if report.moved > 0 {
        logger::log(
            "success",
            "account",
            format!("[{}] {verb}: {} message(s) from {address}", account.email, report.moved),
        );
    }
    if report.local_only > 0 {
        logger::log(
            "info",
            "account",
            format!(
                "[{}] {} message(s) from {address} marked junk here only: the server has no Junk folder to file them in",
                account.email, report.local_only
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::provider::{EmailCategory, FakeEmailProvider, FakeFolderOp, FakeMailboxOp};

    fn account() -> Account {
        Account {
            id: "acc-1".to_string(),
            provider: "gmail".to_string(),
            email: "me@example.com".to_string(),
            name: "Me".to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        }
    }

    fn message(id: &str, sender: &str, mailbox: &str) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acc-1".to_string(),
            thread_id: format!("t-{id}"),
            message_id: Some(format!("<{id}@example.com>")),
            references: None,
            subject: "Weekly deals".to_string(),
            sender: "Deals".to_string(),
            sender_email: sender.to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: vec![],
            body: "b".to_string(),
            snippet: "b".to_string(),
            timestamp: 1,
            is_read: false,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: mailbox.to_string(),
            is_sent: false,
            is_starred: false,
            headers: None,
        }
    }

    fn setup(rows: &[Email]) -> (Arc<Database>, FakeEmailProvider) {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(rows).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        for row in rows {
            provider.add_message(row.clone(), EmailCategory::Primary, vec![]);
        }
        (Arc::new(db), provider)
    }

    fn override_of(db: &Database, id: &str) -> Option<String> {
        db.connection()
            .query_row("SELECT user_override FROM email_junk WHERE email_id = ?1", [id], |r| {
                r.get(0)
            })
            .ok()
            .flatten()
    }

    // ── planners ──────────────────────────────────────────────────────────

    #[test]
    fn addresses_are_normalized_or_refused() {
        assert_eq!(normalize_address("  News@Example.COM ").unwrap(), "news@example.com");
        assert_eq!(normalize_address("<deals@shop.example>").unwrap(), "deals@shop.example");
        for bad in [
            "",
            "   ",
            "not-an-address",
            "a@b@c",
            "a b@example.com",
            "a@example.com, b@example.com",
        ] {
            assert!(normalize_address(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_received_inbox_mail_from_a_blocked_address_is_caught() {
        let blocked: HashSet<String> = ["deals@shop.example".to_string()].into();
        let mut sent = message("sent", "deals@shop.example", "inbox");
        sent.is_sent = true;
        let batch = [
            message("hit", "Deals@Shop.example", "inbox"),
            message("other", "friend@example.com", "inbox"),
            message("already-spam", "deals@shop.example", "spam"),
            sent,
        ];
        let caught: Vec<&str> = plan_blocked_arrivals(&batch, &blocked)
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(caught, vec!["hit"]);
        assert!(plan_blocked_arrivals(&batch, &HashSet::new()).is_empty());
    }

    // ── executors ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn blocking_records_the_block_and_files_existing_mail_as_spam() {
        let (db, provider) = setup(&[
            message("m1", "deals@shop.example", "inbox"),
            message("m2", "deals@shop.example", "archive"),
            message("keep", "friend@example.com", "inbox"),
        ]);

        let report = block_sender(
            &db,
            &account(),
            "Deals@Shop.example",
            true,
            ProviderAccess::Ready(&provider),
            100,
        )
        .await
        .unwrap();

        assert_eq!(
            report,
            SenderMoveReport {
                moved: 2,
                local_only: 0,
                failed: 0
            }
        );
        assert!(db.is_sender_blocked("acc-1", "deals@shop.example").unwrap());
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "spam");
        assert_eq!(db.get_email("m2").unwrap().unwrap().mailbox, "spam");
        assert_eq!(db.get_email("keep").unwrap().unwrap().mailbox, "inbox");
        assert_eq!(override_of(&db, "m1").as_deref(), Some("junk"));
        assert_eq!(provider.mailbox_ops().len(), 2);
    }

    #[tokio::test]
    async fn blocking_without_moving_existing_mail_touches_nothing_else() {
        let (db, provider) = setup(&[message("m1", "deals@shop.example", "inbox")]);

        let report = block_sender(
            &db,
            &account(),
            "deals@shop.example",
            false,
            ProviderAccess::Ready(&provider),
            100,
        )
        .await
        .unwrap();

        assert_eq!(report, SenderMoveReport::default());
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "inbox");
        assert!(provider.mailbox_ops().is_empty());
    }

    #[tokio::test]
    async fn an_unreachable_provider_keeps_the_block_and_counts_the_failures() {
        let (db, _provider) = setup(&[message("m1", "deals@shop.example", "inbox")]);
        let offline = AppError::SyncError("offline".into());

        let report = block_sender(
            &db,
            &account(),
            "deals@shop.example",
            true,
            ProviderAccess::Unreachable(&offline),
            100,
        )
        .await
        .unwrap();

        assert_eq!(report.failed, 1);
        assert!(db.is_sender_blocked("acc-1", "deals@shop.example").unwrap());
        assert_eq!(override_of(&db, "m1").as_deref(), Some("junk"), "the judgement is kept");
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "inbox");
    }

    #[tokio::test]
    async fn a_new_arrival_from_a_blocked_sender_is_marked_junk_and_filed_as_spam() {
        let arrival = message("new", "deals@shop.example", "inbox");
        let (db, provider) = setup(&[arrival.clone(), message("ok", "friend@example.com", "inbox")]);
        db.insert_blocked_sender("acc-1", "deals@shop.example", 1).unwrap();

        let caught = file_blocked_arrivals(
            &db,
            &account(),
            &provider,
            &[arrival, message("ok", "friend@example.com", "inbox")],
        )
        .await;

        assert_eq!(caught, 1);
        assert_eq!(
            provider.mailbox_ops(),
            vec![FakeMailboxOp::Spam {
                message_id: "new".to_string()
            }]
        );
        assert_eq!(db.get_email("new").unwrap().unwrap().mailbox, "spam");
        assert_eq!(override_of(&db, "new").as_deref(), Some("junk"));
        assert_eq!(db.get_email("ok").unwrap().unwrap().mailbox, "inbox");
    }

    #[tokio::test]
    async fn a_provider_refusal_on_arrival_leaves_the_message_marked_junk_in_place() {
        let arrival = message("new", "deals@shop.example", "inbox");
        let (db, provider) = setup(std::slice::from_ref(&arrival));
        db.insert_blocked_sender("acc-1", "deals@shop.example", 1).unwrap();
        provider.fail_mailbox_writes("server said no");

        let caught = file_blocked_arrivals(&db, &account(), &provider, &[arrival]).await;

        assert_eq!(caught, 1);
        assert_eq!(db.get_email("new").unwrap().unwrap().mailbox, "inbox");
        assert_eq!(override_of(&db, "new").as_deref(), Some("junk"));
    }

    // A block the provider refused (offline, or no Junk folder) leaves the
    // sender's mail in the inbox, marked junk here only. Unblocking with
    // "bring their mail back" must forget that mark too, or the messages stay
    // junk with no block left to explain it.
    #[tokio::test]
    async fn unblocking_forgets_the_junk_mark_on_mail_a_failed_block_left_in_place() {
        let (db, _provider) = setup(&[message("m1", "deals@shop.example", "inbox")]);
        let offline = AppError::SyncError("offline".into());
        block_sender(
            &db,
            &account(),
            "deals@shop.example",
            true,
            ProviderAccess::Unreachable(&offline),
            100,
        )
        .await
        .unwrap();
        assert_eq!(override_of(&db, "m1").as_deref(), Some("junk"));

        let report = unblock_sender(
            &db,
            &account(),
            "deals@shop.example",
            true,
            ProviderAccess::Unreachable(&offline),
        )
        .await
        .unwrap();

        assert_eq!(report.failed, 0, "nothing needed the provider");
        assert_eq!(override_of(&db, "m1"), None, "the block's junk mark is forgotten");
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "inbox");
    }

    #[tokio::test]
    async fn unblocking_without_restore_keeps_the_junk_mark_in_place() {
        let (db, _provider) = setup(&[message("m1", "deals@shop.example", "inbox")]);
        let offline = AppError::SyncError("offline".into());
        block_sender(
            &db,
            &account(),
            "deals@shop.example",
            true,
            ProviderAccess::Unreachable(&offline),
            100,
        )
        .await
        .unwrap();

        unblock_sender(
            &db,
            &account(),
            "deals@shop.example",
            false,
            ProviderAccess::Unreachable(&offline),
        )
        .await
        .unwrap();

        assert_eq!(override_of(&db, "m1").as_deref(), Some("junk"));
    }

    #[tokio::test]
    async fn unblocking_removes_the_block_and_brings_their_mail_back_from_spam() {
        let (db, provider) = setup(&[
            message("m1", "deals@shop.example", "inbox"),
            message("other-spam", "spam@else.example", "spam"),
        ]);
        block_sender(
            &db,
            &account(),
            "deals@shop.example",
            true,
            ProviderAccess::Ready(&provider),
            100,
        )
        .await
        .unwrap();

        let report = unblock_sender(
            &db,
            &account(),
            "deals@shop.example",
            true,
            ProviderAccess::Ready(&provider),
        )
        .await
        .unwrap();

        assert_eq!(report.moved, 1);
        assert!(!db.is_sender_blocked("acc-1", "deals@shop.example").unwrap());
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "inbox");
        assert_eq!(override_of(&db, "m1"), None, "the block's junk mark is forgotten");
        assert_eq!(db.get_email("other-spam").unwrap().unwrap().mailbox, "spam");
        assert!(provider.folder_ops().contains(&FakeFolderOp::Move {
            message_id: "m1".to_string(),
            mailbox_value: "inbox".to_string()
        }));
    }

    #[tokio::test]
    async fn unblocking_without_restore_leaves_spam_alone() {
        let (db, provider) = setup(&[message("m1", "deals@shop.example", "spam")]);
        db.insert_blocked_sender("acc-1", "deals@shop.example", 1).unwrap();

        unblock_sender(
            &db,
            &account(),
            "deals@shop.example",
            false,
            ProviderAccess::Ready(&provider),
        )
        .await
        .unwrap();

        assert!(!db.is_sender_blocked("acc-1", "deals@shop.example").unwrap());
        assert_eq!(db.get_email("m1").unwrap().unwrap().mailbox, "spam");
    }

    #[test]
    fn sender_status_reports_block_and_unsubscribe_state_without_raw_headers() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        let mut email = message("m1", "Deals@Shop.example", "inbox");
        email.headers = Some(crate::models::headers::RawHeaders {
            list_unsubscribe: Some("<https://shop.example/u/token-123>".into()),
            list_unsubscribe_post: Some("List-Unsubscribe=One-Click".into()),
            ..Default::default()
        });
        db.insert_emails_batch(&[email]).unwrap();
        db.insert_blocked_sender("acc-1", "deals@shop.example", 1).unwrap();
        db.upsert_sender_unsubscribe("acc-1", "deals@shop.example", "one_click", 7)
            .unwrap();

        let status = sender_status(&db, "acc-1", "m1").unwrap();

        assert_eq!(
            status,
            SenderStatus {
                address: "deals@shop.example".into(),
                blocked: true,
                unsubscribe: Some(UnsubscribeOption {
                    kind: crate::services::unsubscribe::UnsubscribeKind::OneClick,
                    target: "shop.example".into(),
                    url: None,
                }),
                unsubscribed_at: Some(7),
            }
        );
        assert!(
            sender_status(&db, "acc-other", "m1").is_err(),
            "another account's message"
        );
    }
}
