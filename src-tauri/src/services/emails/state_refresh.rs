//! Server-to-local refresh of mail that is already stored.
//!
//! The fetch passes are insert-only: they drop every id the database already
//! holds before downloading, so whatever the user does to a stored message in
//! another client — reads it, deletes it, files it — never reached the app.
//! Once per [`REFRESH_INTERVAL_SECS`] a sync asks the provider for the current
//! state of the account's recent stored messages
//! ([`EmailProvider::fetch_message_states`]) and applies the difference.
//!
//! **Conflict rule:** a row with a read-state change still owed to the provider
//! (`read_push_pending_since`) is left alone — the pending local change wins.
//! For every other row the server wins.
//!
//! **Bounded cost:** only rows from the last [`REFRESH_WINDOW_SECS`], newest
//! first, at most [`MAX_REFRESH_ROWS`] per pass; at most
//! [`MAX_VANISHED_PER_PASS`] vanished messages are chased per pass (each one is
//! a provider lookup). Anything beyond the caps is simply not refreshed.

use std::collections::HashMap;
use std::sync::Arc;

use crate::db::emails::mailbox_state::StoredMessageState;
use crate::db::Database;
use crate::models::error::Result;
use crate::models::Account;
use crate::sync::provider::{EmailProvider, RemoteMessageState};

use super::emit_account_log;
use super::optimistic::LOCAL_SENT_ID_PREFIX;

/// How far back the refresh looks. State changes made elsewhere are
/// overwhelmingly about recent mail, and the pass has to stay cheap enough to
/// run on every few syncs.
const REFRESH_WINDOW_SECS: i64 = 30 * 86_400;

/// Most rows one pass checks. IMAP answers this in one `UID FETCH` per folder;
/// Graph in one `$batch` per twenty ids, i.e. at most ten requests.
const MAX_REFRESH_ROWS: usize = 200;

/// Minimum gap between two passes on one account. Gmail-style minute polling
/// would otherwise repeat the whole check sixty times an hour.
const REFRESH_INTERVAL_SECS: i64 = 2 * 60;

/// Most vanished messages one pass looks for. Finding where a message went is
/// one provider lookup each — on IMAP a connection and a header search in up to
/// a couple of dozen folders — so a bulk clean-up in another client converges
/// over a few passes instead of stalling one sync.
const MAX_VANISHED_PER_PASS: usize = 25;

fn last_refresh_key(account_id: &str) -> String {
    format!("mailbox_state_refresh_last:{account_id}")
}

/// One local change the refresh has decided on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RefreshAction {
    /// Take the provider's read flag.
    SetRead { id: String, is_read: bool },
    /// The provider no longer has the message under this id: find out where it
    /// went, or mark it deleted.
    Vanished { id: String, message_id: Option<String> },
}

/// Pure: diff what is stored against what the provider reports.
///
/// - A row the provider said nothing about is left alone — "could not check"
///   must never read as "gone".
/// - A row with a pending local push is left alone, whatever the provider says.
/// - Sent mail keeps its read flag: it is read by definition here, while the
///   provider's copy often carries no `\Seen`.
pub(super) fn plan_state_refresh(
    stored: &[StoredMessageState],
    remote: &HashMap<String, RemoteMessageState>,
) -> Vec<RefreshAction> {
    stored
        .iter()
        .filter(|row| !row.read_push_pending)
        .filter_map(|row| match remote.get(&row.id)? {
            RemoteMessageState::Present { is_read } if !row.is_sent && *is_read != row.is_read => {
                Some(RefreshAction::SetRead {
                    id: row.id.clone(),
                    is_read: *is_read,
                })
            }
            RemoteMessageState::Present { .. } => None,
            RemoteMessageState::Missing => Some(RefreshAction::Vanished {
                id: row.id.clone(),
                message_id: row.message_id.clone(),
            }),
        })
        .collect()
}

/// Bring recent stored mail in line with the provider. Non-fatal: every
/// failure is logged and leaves the rows as they were until the next pass.
pub(super) async fn refresh_stored_mail_state(
    db: &Arc<Database>,
    account: &Account,
    email_provider: &dyn EmailProvider,
    now: i64,
) {
    // A stamp in the future (the clock moved back) does not count as recent.
    let last_key = last_refresh_key(&account.id);
    let last = db
        .get_preference(&last_key)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i64>().ok());
    if last.is_some_and(|t| (0..REFRESH_INTERVAL_SECS).contains(&(now - t))) {
        return;
    }

    let mut stored = match db.state_refresh_candidates(&account.id, now - REFRESH_WINDOW_SECS, MAX_REFRESH_ROWS) {
        Ok(rows) => rows,
        Err(e) => {
            warn(
                account,
                &format!("Could not read stored mail to refresh its state: {e}"),
            );
            return;
        }
    };
    // An optimistic Sent row the reconciler never matched keeps its synthetic
    // id for good. No provider knows that id, and "unknown id" there must not
    // be read as "deleted upstream" — the row may be the only copy.
    stored.retain(|row| !row.id.starts_with(LOCAL_SENT_ID_PREFIX));
    if stored.is_empty() {
        return;
    }
    let ids: Vec<String> = stored.iter().map(|row| row.id.clone()).collect();
    let remote = match email_provider.fetch_message_states(&ids).await {
        Ok(Some(remote)) => remote,
        // The provider has no refresh: nothing to do, now or later.
        Ok(None) => return,
        Err(e) => {
            warn(account, &format!("Could not refresh the state of stored mail: {e}"));
            return;
        }
    };
    // Stamped once the provider has answered, so a failed pass is retried on
    // the next sync rather than a whole interval later.
    if let Err(e) = db.set_preference(&last_key, &now.to_string()) {
        warn(
            account,
            &format!("Could not record when stored mail was last refreshed: {e}"),
        );
    }

    let mut read_changes: u32 = 0;
    let mut moved: u32 = 0;
    let mut removed: u32 = 0;
    let mut chased: usize = 0;
    for action in plan_state_refresh(&stored, &remote) {
        match action {
            RefreshAction::SetRead { id, is_read } => match db.apply_server_read_state(&id, is_read) {
                Ok(true) => read_changes += 1,
                Ok(false) => {}
                Err(e) => warn(account, &format!("Could not update the read state of {id}: {e}")),
            },
            RefreshAction::Vanished { id, message_id } => {
                if chased >= MAX_VANISHED_PER_PASS {
                    continue;
                }
                chased += 1;
                match follow_vanished(db, email_provider, &id, message_id.as_deref()).await {
                    Ok(Followed::Moved) => moved += 1,
                    Ok(Followed::Removed) => removed += 1,
                    Ok(Followed::StillThere) => {}
                    Err(e) => warn(account, &format!("Could not follow message {id} at the provider: {e}")),
                }
            }
        }
    }

    if read_changes + moved + removed > 0 {
        emit_account_log(
            "success",
            "sync",
            &account.email,
            &format!(
                "Matched the account: {read_changes} read-state change(s), {moved} moved, {removed} deleted elsewhere"
            ),
        );
    }
}

fn warn(account: &Account, message: &str) {
    emit_account_log("warn", "sync", &account.email, message);
}

enum Followed {
    /// Re-filed (and, on IMAP/Graph, re-keyed) where the provider has it now.
    Moved,
    /// Gone at the provider, or already stored under its new id.
    Removed,
    /// Back under its own id by the time we looked.
    StillThere,
}

/// A stored message stopped answering to its id. Ask the provider where it is
/// now and follow it: re-key the row in place when it moved (tags, body and
/// embeddings travel with it), soft-delete it when the provider no longer has
/// it or the moved copy is already stored.
async fn follow_vanished(
    db: &Arc<Database>,
    email_provider: &dyn EmailProvider,
    id: &str,
    message_id: Option<&str>,
) -> Result<Followed> {
    let Some(location) = email_provider.locate_message(id, message_id).await? else {
        db.delete_email(id)?;
        return Ok(Followed::Removed);
    };
    if location.id == id {
        return Ok(Followed::StillThere);
    }
    let already_stored = db
        .emails_exist_batch(std::slice::from_ref(&location.id))?
        .contains(&location.id);
    if already_stored {
        db.delete_email(id)?;
        return Ok(Followed::Removed);
    }
    db.migrate_email_id(id, &location.id, &location.mailbox)?;
    Ok(Followed::Moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Email;
    use crate::sync::provider::{EmailCategory, FakeEmailProvider};

    const NOW: i64 = 40 * 86_400;

    fn stored(id: &str, is_read: bool) -> StoredMessageState {
        StoredMessageState {
            id: id.to_string(),
            message_id: Some(format!("<{id}@example.com>")),
            mailbox: "inbox".to_string(),
            is_read,
            is_sent: false,
            read_push_pending: false,
        }
    }

    fn present(is_read: bool) -> RemoteMessageState {
        RemoteMessageState::Present { is_read }
    }

    fn remote(entries: &[(&str, RemoteMessageState)]) -> HashMap<String, RemoteMessageState> {
        entries.iter().map(|(id, state)| (id.to_string(), *state)).collect()
    }

    // ── planner ───────────────────────────────────────────────────────────

    #[test]
    fn read_state_follows_the_server_in_both_directions() {
        let cases = [
            ("unread here, read there", false, true, Some(true)),
            ("read here, unread there", true, false, Some(false)),
            ("read on both sides", true, true, None),
            ("unread on both sides", false, false, None),
        ];
        for (label, local, server, expected) in cases {
            let plan = plan_state_refresh(&[stored("m-1", local)], &remote(&[("m-1", present(server))]));
            let expected: Vec<RefreshAction> = expected
                .map(|is_read| RefreshAction::SetRead {
                    id: "m-1".to_string(),
                    is_read,
                })
                .into_iter()
                .collect();
            assert_eq!(plan, expected, "{label}");
        }
    }

    #[test]
    fn a_message_the_server_no_longer_has_is_vanished_with_its_message_id() {
        let plan = plan_state_refresh(&[stored("m-1", true)], &remote(&[("m-1", RemoteMessageState::Missing)]));
        assert_eq!(
            plan,
            vec![RefreshAction::Vanished {
                id: "m-1".to_string(),
                message_id: Some("<m-1@example.com>".to_string()),
            }]
        );
    }

    #[test]
    fn a_row_the_server_said_nothing_about_is_left_alone() {
        // Its folder would not open, or its sub-request was throttled: that is
        // not the same as "deleted".
        let plan = plan_state_refresh(&[stored("m-1", false)], &remote(&[("other", present(true))]));
        assert!(plan.is_empty());
    }

    #[test]
    fn a_row_with_a_pending_local_push_is_never_touched() {
        for server in [present(false), present(true), RemoteMessageState::Missing] {
            let mut row = stored("m-1", true);
            row.read_push_pending = true;
            let plan = plan_state_refresh(&[row], &remote(&[("m-1", server)]));
            assert!(plan.is_empty(), "pending local change must win over {server:?}");
        }
    }

    #[test]
    fn sent_mail_keeps_its_read_flag_but_is_still_followed_when_it_vanishes() {
        let mut row = stored("s-1", true);
        row.is_sent = true;
        row.mailbox = "sent".to_string();

        assert!(plan_state_refresh(std::slice::from_ref(&row), &remote(&[("s-1", present(false))])).is_empty());
        assert_eq!(
            plan_state_refresh(&[row], &remote(&[("s-1", RemoteMessageState::Missing)])).len(),
            1
        );
    }

    #[test]
    fn each_row_is_planned_on_its_own() {
        let mut pending = stored("pending", true);
        pending.read_push_pending = true;
        let plan = plan_state_refresh(
            &[
                stored("same", true),
                stored("flip", false),
                pending,
                stored("gone", false),
                stored("unknown", false),
            ],
            &remote(&[
                ("same", present(true)),
                ("flip", present(true)),
                ("pending", present(false)),
                ("gone", RemoteMessageState::Missing),
            ]),
        );
        assert_eq!(
            plan,
            vec![
                RefreshAction::SetRead {
                    id: "flip".to_string(),
                    is_read: true
                },
                RefreshAction::Vanished {
                    id: "gone".to_string(),
                    message_id: Some("<gone@example.com>".to_string()),
                },
            ]
        );
    }

    #[test]
    fn nothing_stored_or_nothing_reported_plans_nothing() {
        assert!(plan_state_refresh(&[], &remote(&[("m-1", present(true))])).is_empty());
        assert!(plan_state_refresh(&[stored("m-1", false)], &HashMap::new()).is_empty());
    }

    // ── executor ──────────────────────────────────────────────────────────

    fn account(provider: &str) -> Account {
        Account {
            id: "acc-1".to_string(),
            provider: provider.to_string(),
            email: "me@example.com".to_string(),
            name: "Me".to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        }
    }

    fn email(id: &str, mailbox: &str, is_read: bool) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acc-1".to_string(),
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
            timestamp: NOW - 3_600,
            is_read,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: mailbox.to_string(),
            is_sent: mailbox == "sent",
            headers: None,
        }
    }

    /// A DB and a fake provider holding the same messages.
    fn synced(emails: &[Email]) -> (Arc<Database>, FakeEmailProvider) {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(emails).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.report_message_states();
        for email in emails {
            provider.add_message(email.clone(), EmailCategory::Primary, vec![]);
        }
        (Arc::new(db), provider)
    }

    /// `(mailbox, is_read)` of a row the user can still see; `None` once it is
    /// soft-deleted or gone.
    fn row(db: &Database, id: &str) -> Option<(String, bool)> {
        use rusqlite::OptionalExtension;
        db.reader()
            .query_row(
                "SELECT mailbox, is_read FROM emails WHERE id = ?1 AND is_deleted = 0",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i32>(1)? != 0)),
            )
            .optional()
            .unwrap()
    }

    #[tokio::test]
    async fn a_message_read_in_another_client_becomes_read_here() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        provider.set_remote_read("m-1", true);

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW).await;

        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), true)));
    }

    #[tokio::test]
    async fn a_message_marked_unread_in_another_client_becomes_unread_here() {
        let (db, provider) = synced(&[email("m-1", "inbox", true)]);
        provider.set_remote_read("m-1", false);

        refresh_stored_mail_state(&db, &account("outlook"), &provider, NOW).await;

        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), false)));
    }

    #[tokio::test]
    async fn a_pending_local_read_is_not_downgraded_by_the_servers_older_state() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        db.mark_as_read_pending_push("m-1", NOW - 60).unwrap();

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW).await;

        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), true)));
    }

    #[tokio::test]
    async fn a_message_deleted_forever_in_another_client_is_deleted_here() {
        let (db, provider) = synced(&[email("m-1", "inbox", true), email("m-2", "inbox", true)]);
        provider.remove_message("m-1");

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW).await;

        assert_eq!(row(&db, "m-1"), None, "soft-deleted locally");
        assert!(row(&db, "m-2").is_some());
        assert!(
            db.emails_exist_batch(&["m-1".to_string()]).unwrap().contains("m-1"),
            "the row is kept so the sync never re-downloads it"
        );
    }

    #[tokio::test]
    async fn a_message_moved_in_another_client_is_rekeyed_into_its_new_mailbox() {
        // IMAP and Graph give a moved message a new id; tags and embeddings
        // must travel with the row instead of being re-derived.
        let (db, provider) = synced(&[email("m-1", "inbox", true)]);
        db.connection()
            .execute(
                "INSERT INTO email_tags (email_id, tag_type, tag_value, created_at) VALUES ('m-1', 'topic', 'dental', 0)",
                [],
            )
            .unwrap();
        provider.relocate_message("m-1", "trash-9", "trash");

        refresh_stored_mail_state(&db, &account("outlook"), &provider, NOW).await;

        assert_eq!(row(&db, "m-1"), None);
        assert_eq!(row(&db, "trash-9"), Some(("trash".to_string(), true)));
        let tags: i64 = db
            .reader()
            .query_row("SELECT COUNT(*) FROM email_tags WHERE email_id = 'trash-9'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(tags, 1, "local AI state follows the message");
    }

    #[tokio::test]
    async fn a_moved_message_already_stored_under_its_new_id_drops_the_stale_row() {
        let (db, provider) = synced(&[email("m-1", "inbox", true)]);
        provider.relocate_message("m-1", "folder-5", "folder:Projects");
        // The folder pass already ingested the moved copy.
        let mut moved_copy = email("folder-5", "folder:Projects", true);
        moved_copy.message_id = Some("<m-1@example.com>".to_string());
        db.insert_emails_batch(&[moved_copy]).unwrap();

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW).await;

        assert_eq!(row(&db, "m-1"), None);
        assert_eq!(row(&db, "folder-5"), Some(("folder:Projects".to_string(), true)));
    }

    #[tokio::test]
    async fn a_message_the_provider_could_not_check_is_left_alone() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        provider.remove_message("m-1");
        provider.make_state_unverifiable("m-1");

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW).await;

        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), false)));
    }

    #[tokio::test]
    async fn mail_older_than_the_window_is_not_refreshed() {
        let mut old = email("m-old", "inbox", false);
        old.timestamp = NOW - REFRESH_WINDOW_SECS - 1;
        let (db, provider) = synced(&[old]);
        provider.remove_message("m-old");

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW).await;

        assert!(row(&db, "m-old").is_some());
        assert!(
            provider.calls().is_empty(),
            "nothing in the window, so no provider call"
        );
    }

    #[tokio::test]
    async fn a_locally_composed_sent_row_is_never_taken_for_a_deleted_message() {
        // An optimistic Sent row the reconciler never matched keeps its
        // synthetic id for good. No provider knows that id, and "unknown id"
        // must not be read as "deleted upstream" — it may be the only copy.
        let (db, _) = synced(&[email("local-sent-abc", "sent", true)]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.report_message_states();

        refresh_stored_mail_state(&db, &account("outlook"), &provider, NOW).await;

        assert!(row(&db, "local-sent-abc").is_some());
        assert!(provider.calls().is_empty(), "nothing the provider could know about");
    }

    #[tokio::test]
    async fn spam_is_left_to_the_spam_reconciliation() {
        let (db, provider) = synced(&[email("sp-1", "spam", false)]);
        provider.remove_message("sp-1");

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW).await;

        assert!(row(&db, "sp-1").is_some());
    }

    #[tokio::test]
    async fn a_provider_without_a_refresh_changes_nothing_and_is_not_throttled() {
        let (db, _) = synced(&[email("m-1", "inbox", false)]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");

        refresh_stored_mail_state(&db, &account("gmail"), &provider, NOW).await;

        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), false)));
        assert_eq!(db.get_preference(&last_refresh_key("acc-1")).unwrap(), None);
    }

    #[tokio::test]
    async fn the_refresh_runs_at_most_once_per_interval() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW).await;
        provider.set_remote_read("m-1", true);

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW + REFRESH_INTERVAL_SECS - 1).await;
        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), false)), "still throttled");

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW + REFRESH_INTERVAL_SECS).await;
        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), true)));
    }

    #[tokio::test]
    async fn one_pass_checks_a_bounded_number_of_rows_and_chases_a_bounded_number_of_vanished_ones() {
        let emails: Vec<Email> = (0..MAX_REFRESH_ROWS + 50)
            .map(|i| {
                let mut e = email(&format!("m-{i:04}"), "inbox", true);
                e.timestamp = NOW - 3_600 - i as i64;
                e
            })
            .collect();
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(&emails).unwrap();
        let db = Arc::new(db);
        // The provider has none of them: every checked row has vanished.
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.report_message_states();

        refresh_stored_mail_state(&db, &account("imap"), &provider, NOW).await;

        let locates = provider
            .calls()
            .iter()
            .filter(|c| c.as_str() == "locate_message")
            .count();
        assert_eq!(locates, MAX_VANISHED_PER_PASS);
        let live: i64 = db
            .reader()
            .query_row("SELECT COUNT(*) FROM emails WHERE is_deleted = 0", [], |r| r.get(0))
            .unwrap();
        assert_eq!(live as usize, emails.len() - MAX_VANISHED_PER_PASS);
    }
}
