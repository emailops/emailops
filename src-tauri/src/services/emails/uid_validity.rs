//! IMAP UIDVALIDITY: noticing that a server renumbered a mailbox, and moving
//! the stored rows onto the new numbers.
//!
//! An IMAP message id here is its mailbox plus its UID, and a UID is only
//! stable while the mailbox's UIDVALIDITY is (RFC 3501 §2.3.1.1). A server
//! that rebuilds a mailbox issues a new UIDVALIDITY and renumbers. Left
//! unnoticed, new mail whose UID equals a stored id is dropped as "already
//! synced", and every stored id addresses some other message — for a re-fetch,
//! a move, a flag change.
//!
//! So each sync starts by comparing the server's UIDVALIDITY per mailbox with
//! the one recorded when its mail was stored (V027 `folder_uid_validity`). On
//! a change, before anything is listed:
//!
//! 1. the mailbox is listed as `(new id, Message-ID, arrival time)`;
//! 2. stored rows are matched to it ([`plan_uid_rekey`]) and re-keyed in
//!    place, so read state, tags, bodies and embeddings survive;
//! 3. rows nothing matches are removed — their id names nothing any more;
//! 4. the mailbox's sync windows are reopened, so whatever is still on the
//!    server and not stored is downloaded again.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::db::emails::folder_ops::StoredUidRow;
use crate::db::Database;
use crate::models::error::Result;
use crate::models::Account;
use crate::sync::provider::{EmailProvider, MessageIdentity};

use super::emit_account_log;

/// How one mailbox's stored rows map onto its renumbered messages.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct UidRekeyPlan {
    /// `(stored id, id the message has now)`, for rows whose id changed.
    pub rekey: Vec<(String, String)>,
    /// Stored ids no message of the mailbox answers to.
    pub orphans: Vec<String>,
}

fn usable(message_id: Option<&str>) -> Option<&str> {
    message_id.map(str::trim).filter(|id| !id.is_empty())
}

/// Pure: match the rows stored for a mailbox to the messages it holds now.
///
/// - **By Message-ID.** Copies sharing one Message-ID are paired in order
///   (stored oldest first, server in listing order): they are the same message
///   as far as any header can tell.
/// - **By arrival time**, for the few messages without a Message-ID: only when
///   exactly one such stored row and one such server message share it.
/// - A stored row left unmatched is an orphan: its message was deleted or
///   moved away while the mailbox was being rebuilt, or cannot be told apart.
///
/// A server message left unmatched is not this function's business — it is
/// mail the normal passes download.
pub(super) fn plan_uid_rekey(stored: &[StoredUidRow], server: &[MessageIdentity]) -> UidRekeyPlan {
    let mut by_message_id: HashMap<&str, VecDeque<usize>> = HashMap::new();
    for (index, message) in server.iter().enumerate() {
        if let Some(message_id) = usable(message.message_id.as_deref()) {
            by_message_id.entry(message_id).or_default().push_back(index);
        }
    }
    let mut matched: Vec<Option<usize>> = stored
        .iter()
        .map(|row| {
            usable(row.message_id.as_deref())
                .and_then(|message_id| by_message_id.get_mut(message_id))
                .and_then(VecDeque::pop_front)
        })
        .collect();

    // Arrival time, for what has no Message-ID on either side.
    let mut server_by_time: HashMap<i64, Vec<usize>> = HashMap::new();
    for (index, message) in server.iter().enumerate() {
        if usable(message.message_id.as_deref()).is_none() {
            if let Some(timestamp) = message.timestamp {
                server_by_time.entry(timestamp).or_default().push(index);
            }
        }
    }
    let mut stored_by_time: HashMap<i64, Vec<usize>> = HashMap::new();
    for (index, row) in stored.iter().enumerate() {
        if usable(row.message_id.as_deref()).is_none() {
            stored_by_time.entry(row.timestamp).or_default().push(index);
        }
    }
    for (timestamp, rows) in &stored_by_time {
        if let ([row], Some([message])) = (rows.as_slice(), server_by_time.get(timestamp).map(Vec::as_slice)) {
            matched[*row] = Some(*message);
        }
    }

    let mut plan = UidRekeyPlan::default();
    for (row, matched) in stored.iter().zip(matched) {
        match matched {
            Some(index) if server[index].id != row.id => plan.rekey.push((row.id.clone(), server[index].id.clone())),
            Some(_) => {}
            None => plan.orphans.push(row.id.clone()),
        }
    }
    plan
}

/// Compare every mailbox's UIDVALIDITY with the recorded one and repair the
/// stored ids of any mailbox the server renumbered. Runs first in a sync.
///
/// Fatal on failure, unlike the other upkeep passes: a sync that went on to
/// list a renumbered mailbox with the old ids still in place would drop new
/// mail and re-fetch the wrong messages. The recorded UIDVALIDITY only
/// advances once the repair is done, so a failed attempt is simply retried by
/// the next sync.
pub(super) async fn reconcile_uid_validity(
    db: &Arc<Database>,
    account: &Account,
    email_provider: &dyn EmailProvider,
) -> Result<()> {
    for folder in email_provider.folder_uid_validities().await? {
        match db.get_folder_uid_validity(&account.id, &folder.mailbox)? {
            Some(known) if known == folder.uid_validity => continue,
            // First sight of this mailbox: nothing to compare against yet.
            None => {
                db.set_folder_uid_validity(&account.id, &folder.mailbox, folder.uid_validity)?;
                continue;
            }
            Some(_) => {}
        }

        let server = email_provider.list_mailbox_identities(&folder.mailbox).await?;
        let stored = db.emails_in_uid_namespace(&account.id, &folder.id_prefix)?;
        let plan = plan_uid_rekey(&stored, &server);
        db.apply_uid_rekey(&account.id, &folder.id_prefix, &plan.rekey, &plan.orphans)?;
        super::sync::reopen_mailbox_sync(db, &account.id, &folder.mailbox)?;
        // Last: everything above is safe to repeat, so a crash before this
        // line just redoes it on the next sync.
        db.set_folder_uid_validity(&account.id, &folder.mailbox, folder.uid_validity)?;

        emit_account_log(
            "warn",
            "sync",
            &account.email,
            &format!(
                "The server renumbered the {} mailbox: {} stored message(s) kept, {} moved to their new ids, {} no longer there and removed",
                folder.mailbox,
                stored.len() - plan.orphans.len(),
                plan.rekey.len(),
                plan.orphans.len()
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Email;
    use crate::sync::provider::{EmailCategory, FakeEmailProvider};

    fn stored(id: &str, message_id: Option<&str>, timestamp: i64) -> StoredUidRow {
        StoredUidRow {
            id: id.to_string(),
            message_id: message_id.map(str::to_string),
            timestamp,
        }
    }

    fn on_server(id: &str, message_id: Option<&str>, timestamp: Option<i64>) -> MessageIdentity {
        MessageIdentity {
            id: id.to_string(),
            message_id: message_id.map(str::to_string),
            timestamp,
        }
    }

    fn pair(old: &str, new: &str) -> (String, String) {
        (old.to_string(), new.to_string())
    }

    // ── planner ───────────────────────────────────────────────────────────

    #[test]
    fn rows_follow_their_message_id_to_the_new_uid() {
        let plan = plan_uid_rekey(
            &[
                stored("a::5", Some("<x@example.com>"), 10),
                stored("a::6", Some("<y@example.com>"), 20),
            ],
            &[
                on_server("a::2", Some("<y@example.com>"), Some(20)),
                on_server("a::1", Some("<x@example.com>"), Some(10)),
            ],
        );
        assert_eq!(
            plan,
            UidRekeyPlan {
                rekey: vec![pair("a::5", "a::1"), pair("a::6", "a::2")],
                orphans: vec![],
            }
        );
    }

    #[test]
    fn a_row_whose_uid_did_not_change_needs_no_rekey_and_is_not_an_orphan() {
        // The server only issued a new UIDVALIDITY.
        let plan = plan_uid_rekey(
            &[stored("a::5", Some("<x@example.com>"), 10)],
            &[on_server("a::5", Some("<x@example.com>"), Some(10))],
        );
        assert_eq!(plan, UidRekeyPlan::default());
    }

    #[test]
    fn two_rows_may_trade_ids() {
        let plan = plan_uid_rekey(
            &[
                stored("a::5", Some("<x@example.com>"), 10),
                stored("a::7", Some("<y@example.com>"), 20),
            ],
            &[
                on_server("a::5", Some("<y@example.com>"), Some(20)),
                on_server("a::7", Some("<x@example.com>"), Some(10)),
            ],
        );
        assert_eq!(plan.rekey, vec![pair("a::5", "a::7"), pair("a::7", "a::5")]);
    }

    #[test]
    fn a_row_whose_message_left_the_mailbox_is_an_orphan() {
        let plan = plan_uid_rekey(
            &[stored("a::5", Some("<gone@example.com>"), 10)],
            &[on_server("a::1", Some("<other@example.com>"), Some(10))],
        );
        assert_eq!(
            plan,
            UidRekeyPlan {
                rekey: vec![],
                orphans: vec!["a::5".to_string()],
            }
        );
    }

    #[test]
    fn copies_sharing_a_message_id_are_paired_in_order_and_the_surplus_orphaned() {
        let plan = plan_uid_rekey(
            &[
                stored("a::5", Some("<dup@example.com>"), 10),
                stored("a::6", Some("<dup@example.com>"), 11),
                stored("a::7", Some("<dup@example.com>"), 12),
            ],
            &[
                on_server("a::1", Some("<dup@example.com>"), Some(10)),
                on_server("a::2", Some("<dup@example.com>"), Some(11)),
            ],
        );
        assert_eq!(plan.rekey, vec![pair("a::5", "a::1"), pair("a::6", "a::2")]);
        assert_eq!(plan.orphans, vec!["a::7".to_string()]);
    }

    #[test]
    fn a_server_message_is_never_given_to_two_rows() {
        let plan = plan_uid_rekey(
            &[
                stored("a::5", Some("<x@example.com>"), 10),
                stored("a::6", Some("<x@example.com>"), 10),
            ],
            &[on_server("a::1", Some("<x@example.com>"), Some(10))],
        );
        assert_eq!(plan.rekey, vec![pair("a::5", "a::1")]);
        assert_eq!(plan.orphans, vec!["a::6".to_string()]);
    }

    #[test]
    fn a_message_without_a_message_id_is_matched_by_its_arrival_time_when_unambiguous() {
        let plan = plan_uid_rekey(
            &[stored("a::5", None, 10), stored("a::6", Some("  "), 20)],
            &[on_server("a::1", None, Some(10)), on_server("a::2", Some(""), Some(20))],
        );
        assert_eq!(plan.rekey, vec![pair("a::5", "a::1"), pair("a::6", "a::2")]);
        assert!(plan.orphans.is_empty());
    }

    #[test]
    fn an_ambiguous_arrival_time_matches_nothing() {
        let cases: [(&str, Vec<StoredUidRow>, Vec<MessageIdentity>); 4] = [
            (
                "two server messages at that time",
                vec![stored("a::5", None, 10)],
                vec![on_server("a::1", None, Some(10)), on_server("a::2", None, Some(10))],
            ),
            (
                "two stored rows at that time",
                vec![stored("a::5", None, 10), stored("a::6", None, 10)],
                vec![on_server("a::1", None, Some(10))],
            ),
            (
                "the server message has no arrival time",
                vec![stored("a::5", None, 10)],
                vec![on_server("a::1", None, None)],
            ),
            (
                "the server message at that time has a Message-ID of its own",
                vec![stored("a::5", None, 10)],
                vec![on_server("a::1", Some("<x@example.com>"), Some(10))],
            ),
        ];
        for (label, stored_rows, server) in cases {
            let plan = plan_uid_rekey(&stored_rows, &server);
            assert!(plan.rekey.is_empty(), "{label}");
            assert_eq!(plan.orphans.len(), stored_rows.len(), "{label}");
        }
    }

    #[test]
    fn a_row_with_a_message_id_is_never_matched_by_time_alone() {
        // Its message is gone; a different message arriving in the same
        // second must not inherit its tags and read state.
        let plan = plan_uid_rekey(
            &[stored("a::5", Some("<gone@example.com>"), 10)],
            &[on_server("a::1", None, Some(10))],
        );
        assert_eq!(plan.orphans, vec!["a::5".to_string()]);
    }

    #[test]
    fn message_ids_are_compared_trimmed() {
        let plan = plan_uid_rekey(
            &[stored("a::5", Some(" <x@example.com> "), 10)],
            &[on_server("a::1", Some("<x@example.com>"), Some(10))],
        );
        assert_eq!(plan.rekey, vec![pair("a::5", "a::1")]);
    }

    #[test]
    fn an_emptied_mailbox_orphans_everything_and_an_empty_store_plans_nothing() {
        assert_eq!(
            plan_uid_rekey(&[stored("a::5", Some("<x@example.com>"), 10)], &[]).orphans,
            vec!["a::5".to_string()]
        );
        assert_eq!(
            plan_uid_rekey(&[], &[on_server("a::1", Some("<x@example.com>"), Some(10))]),
            UidRekeyPlan::default()
        );
    }

    // ── executor ──────────────────────────────────────────────────────────

    const PREFIX: &str = "acc-1::";

    fn account() -> Account {
        Account {
            id: "acc-1".to_string(),
            provider: "imap".to_string(),
            email: "me@example.com".to_string(),
            name: "Me".to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        }
    }

    fn email(id: &str, message_id: &str, subject: &str) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acc-1".to_string(),
            thread_id: format!("t-{message_id}"),
            message_id: Some(message_id.to_string()),
            references: None,
            subject: subject.to_string(),
            sender: "Sender".to_string(),
            sender_email: "sender@example.com".to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: vec![],
            body: "body".to_string(),
            snippet: "body".to_string(),
            timestamp: 1_000,
            is_read: true,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: "inbox".to_string(),
            is_sent: false,
            is_starred: false,
            headers: None,
        }
    }

    fn db_with(emails: &[Email]) -> Arc<Database> {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(emails).unwrap();
        Arc::new(db)
    }

    fn subjects(db: &Database) -> Vec<(String, String)> {
        let conn = db.reader();
        let mut stmt = conn.prepare("SELECT id, subject FROM emails ORDER BY id").unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
    }

    #[tokio::test]
    async fn the_first_sync_records_a_baseline_and_touches_nothing() {
        let db = db_with(&[email("acc-1::5", "<a@example.com>", "A")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.set_folder_uid_validity("inbox", PREFIX, 100);

        reconcile_uid_validity(&db, &account(), &provider).await.unwrap();

        assert_eq!(db.get_folder_uid_validity("acc-1", "inbox").unwrap(), Some(100));
        assert!(provider.calls().is_empty(), "no mailbox listing on first sight");
        assert_eq!(subjects(&db), vec![("acc-1::5".to_string(), "A".to_string())]);
    }

    #[tokio::test]
    async fn an_unchanged_uid_validity_costs_no_listing() {
        let db = db_with(&[email("acc-1::5", "<a@example.com>", "A")]);
        db.set_folder_uid_validity("acc-1", "inbox", 100).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.set_folder_uid_validity("inbox", PREFIX, 100);

        reconcile_uid_validity(&db, &account(), &provider).await.unwrap();

        assert!(provider.calls().is_empty());
    }

    #[tokio::test]
    async fn a_renumbered_mailbox_has_its_rows_moved_to_their_new_ids_with_their_state() {
        let db = db_with(&[
            email("acc-1::5", "<a@example.com>", "A"),
            email("acc-1::6", "<b@example.com>", "B"),
            email("acc-1::7", "<deleted@example.com>", "deleted meanwhile"),
        ]);
        db.connection()
            .execute(
                "INSERT INTO email_tags (email_id, tag_type, tag_value, created_at) VALUES ('acc-1::5', 'topic', 'dental', 0)",
                [],
            )
            .unwrap();
        db.set_folder_uid_validity("acc-1", "inbox", 100).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.set_folder_uid_validity("inbox", PREFIX, 200);
        // After the rebuild: A sits where B used to, B moved to a fresh UID.
        provider.add_message(
            email("acc-1::6", "<a@example.com>", "A"),
            EmailCategory::Primary,
            vec![],
        );
        provider.add_message(
            email("acc-1::9", "<b@example.com>", "B"),
            EmailCategory::Primary,
            vec![],
        );

        reconcile_uid_validity(&db, &account(), &provider).await.unwrap();

        assert_eq!(
            subjects(&db),
            vec![
                ("acc-1::6".to_string(), "A".to_string()),
                ("acc-1::9".to_string(), "B".to_string()),
            ],
            "A and B keep their rows under the new UIDs; the vanished message is gone"
        );
        let tags: i64 = db
            .reader()
            .query_row("SELECT COUNT(*) FROM email_tags WHERE email_id = 'acc-1::6'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(tags, 1, "A's tag travelled with it");
        assert_eq!(db.get_folder_uid_validity("acc-1", "inbox").unwrap(), Some(200));
    }

    #[tokio::test]
    async fn a_renumbered_mailbox_has_its_sync_windows_reopened() {
        let db = db_with(&[email("acc-1::FOLDER::UHJvamVjdHM::5", "<a@example.com>", "A")]);
        let folder_prefix = "acc-1::FOLDER::UHJvamVjdHM::";
        db.set_folder_uid_validity("acc-1", "inbox", 100).unwrap();
        db.set_folder_uid_validity("acc-1", "folder:Projects", 100).unwrap();
        for key in crate::services::emails::sync::custom_folder_pref_keys("acc-1", "Projects") {
            db.set_preference(&key, "1700000000").unwrap();
        }
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.set_folder_uid_validity("inbox", PREFIX, 200);
        provider.set_folder_uid_validity("folder:Projects", folder_prefix, 200);

        reconcile_uid_validity(&db, &account(), &provider).await.unwrap();

        assert_eq!(
            db.get_preference("inbox_incremental_resume:acc-1").unwrap().as_deref(),
            Some("0"),
            "the inbox is listed again from the account's floor"
        );
        for key in crate::services::emails::sync::custom_folder_pref_keys("acc-1", "Projects") {
            assert_eq!(db.get_preference(&key).unwrap(), None, "{key} must be forgotten");
        }
    }

    #[tokio::test]
    async fn a_failed_listing_fails_the_sync_and_keeps_the_old_uid_validity() {
        // Going on would list the mailbox with stale ids in place. Keeping the
        // old value makes the next sync try the repair again.
        let db = db_with(&[email("acc-1::5", "<a@example.com>", "A")]);
        db.set_folder_uid_validity("acc-1", "inbox", 100).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.set_folder_uid_validity("inbox", PREFIX, 200);
        provider.fail_identity_listing("connection reset");

        let err = reconcile_uid_validity(&db, &account(), &provider).await.unwrap_err();

        assert!(err.to_string().contains("connection reset"), "unexpected: {err}");
        assert_eq!(db.get_folder_uid_validity("acc-1", "inbox").unwrap(), Some(100));
        assert_eq!(subjects(&db), vec![("acc-1::5".to_string(), "A".to_string())]);
    }

    #[tokio::test]
    async fn only_the_renumbered_mailbox_is_touched() {
        let mut sent = email("acc-1::SENT::5", "<s@example.com>", "S");
        sent.mailbox = "sent".to_string();
        let db = db_with(&[email("acc-1::5", "<a@example.com>", "A"), sent]);
        db.set_folder_uid_validity("acc-1", "inbox", 100).unwrap();
        db.set_folder_uid_validity("acc-1", "sent", 100).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.set_folder_uid_validity("inbox", PREFIX, 200);
        provider.set_folder_uid_validity("sent", "acc-1::SENT::", 100);
        provider.add_message(
            email("acc-1::1", "<a@example.com>", "A"),
            EmailCategory::Primary,
            vec![],
        );

        reconcile_uid_validity(&db, &account(), &provider).await.unwrap();

        assert_eq!(
            subjects(&db),
            vec![
                ("acc-1::1".to_string(), "A".to_string()),
                ("acc-1::SENT::5".to_string(), "S".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn a_provider_without_uids_is_a_no_op() {
        let db = db_with(&[email("m-1", "<a@example.com>", "A")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        reconcile_uid_validity(&db, &account(), &provider).await.unwrap();

        assert_eq!(subjects(&db), vec![("m-1".to_string(), "A".to_string())]);
    }
}
