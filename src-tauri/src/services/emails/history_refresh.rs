//! Server-to-local refresh of stored mail from the provider's change log
//! (Gmail's History API).
//!
//! Gmail keeps a message's id through every label change, so what the user
//! did in Gmail's own clients arrives as label changes: `UNREAD` is the read
//! state, and `TRASH` / `SPAM` / `SENT` / `INBOX` decide the mailbox through
//! the same mapping the sync stores messages with
//! ([`crate::sync::gmail::mailbox_from_labels`]).
//!
//! **Archive is a move here.** That mapping files a message without `INBOX`
//! (and not in Sent, Trash or Spam) under `archive`, so archiving in Gmail's
//! own clients takes it out of the inbox here too, and adding `INBOX` back
//! returns it. A message the user sent moves between `sent` and `inbox`.
//! `STARRED` is the star.
//!
//! **Conflict rule** (as in `state_refresh`): a row with a read-state or star
//! change still owed to the provider is never touched. Unlike a poll, the log says
//! each change once, so a page that had to skip such a row is not counted as
//! applied: the cursor stays before it and the page is replayed on the next
//! pass, after the pending push has been retried. Replaying is idempotent.
//!
//! **Spam** is left to the spam reconciliation: rows stored under `spam` are
//! not loaded, and a message that gained `SPAM` is re-filed by the Spam
//! listing pass.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::db::emails::mailbox_state::StoredMessageState;
use crate::db::Database;
use crate::models::Account;
use crate::sync::gmail::mailbox_from_labels;
use crate::sync::provider::{EmailProvider, HistoryListing, MessageChange, RemoteLabels};

use super::emit_account_log;
use super::optimistic::LOCAL_SENT_ID_PREFIX;
use super::state_refresh::{warn, MAX_REFRESH_ROWS, REFRESH_WINDOW_SECS};

/// Most pages of the change log one pass applies. What remains is picked up
/// by the next pass, from the last page that was fully applied.
const MAX_HISTORY_PAGES_PER_PASS: usize = 5;

/// Where the account's change log was last fully applied.
pub(crate) fn history_cursor_key(account_id: &str) -> String {
    format!("mailbox_history_cursor:{account_id}")
}

/// How a pass over the change log ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum HistoryRefresh {
    /// The provider has no change log: poll message states instead.
    Unsupported,
    /// The provider answered; whatever could be applied was.
    Answered,
    /// Nothing could be read this time: try again on the next sync.
    Failed,
}

/// Apply what the provider's change log reports since the stored cursor.
///
/// - **First run:** the cursor is seeded with the log's current position and
///   nothing is replayed — the rows were just synced.
/// - **Cursor:** moved only past pages that were fully applied, and written
///   once at the end of the pass. A page with a change that could not be
///   applied (a database error, a row with a pending local push) stays ahead
///   of the cursor; later pages are still applied, and replayed with it.
/// - **Bounded:** at most [`MAX_HISTORY_PAGES_PER_PASS`] pages per pass.
/// - **Expired cursor:** see [`reconcile_after_expired_cursor`].
pub(super) async fn refresh_from_history(
    db: &Arc<Database>,
    account: &Account,
    email_provider: &dyn EmailProvider,
    now: i64,
) -> HistoryRefresh {
    let key = history_cursor_key(&account.id);
    let start = match db.get_preference(&key) {
        Ok(Some(cursor)) => cursor,
        Ok(None) => {
            return match email_provider.history_cursor().await {
                Ok(None) => HistoryRefresh::Unsupported,
                Ok(Some(cursor)) => save_cursor(db, account, &key, &cursor),
                Err(e) => {
                    warn(
                        account,
                        &format!("Could not read where the mailbox's change history stands: {e}"),
                    );
                    HistoryRefresh::Failed
                }
            };
        }
        Err(e) => {
            warn(account, &format!("Could not read the change-history cursor: {e}"));
            return HistoryRefresh::Failed;
        }
    };

    let mut cursor = start.clone();
    let mut held = false;
    let mut answered = false;
    let mut page_token: Option<String> = None;
    let mut applied = Applied::default();
    for _ in 0..MAX_HISTORY_PAGES_PER_PASS {
        let page = match email_provider.list_history(&start, page_token.as_deref()).await {
            Ok(HistoryListing::Page(page)) => page,
            Ok(HistoryListing::CursorExpired) => {
                return reconcile_after_expired_cursor(db, account, email_provider, &key, now).await;
            }
            Err(e) => {
                warn(account, &format!("Could not read the mailbox's change history: {e}"));
                break;
            }
        };
        answered = true;
        held |= !apply_page(db, account, &page.changes, &mut applied);
        if !held {
            cursor = page.resume_cursor;
        }
        match page.next_page_token {
            Some(token) => page_token = Some(token),
            None => break,
        }
    }

    applied.log(account);
    if !answered {
        return HistoryRefresh::Failed;
    }
    if cursor != start {
        return save_cursor(db, account, &key, &cursor);
    }
    HistoryRefresh::Answered
}

/// The provider no longer keeps the log back to the stored cursor, so what
/// changed since cannot be replayed. Check the recent stored mail directly —
/// the same bounded set the state poll looks at — and start following the log
/// again from its current position. That position is read *before* the check,
/// so a change made while it runs is replayed rather than lost. The cursor is
/// only replaced once every row was checked and applied: otherwise the expired
/// one stays, and the next sync comes back here. A row with a pending local
/// push is skipped, as everywhere, and is not waited for.
async fn reconcile_after_expired_cursor(
    db: &Arc<Database>,
    account: &Account,
    email_provider: &dyn EmailProvider,
    key: &str,
    now: i64,
) -> HistoryRefresh {
    emit_account_log(
        "info",
        "sync",
        &account.email,
        "The mailbox's change history no longer reaches the last sync: checking recent mail directly",
    );
    let fresh = match email_provider.history_cursor().await {
        Ok(Some(cursor)) => cursor,
        Ok(None) => {
            warn(account, "The provider no longer reports a change history");
            return HistoryRefresh::Failed;
        }
        Err(e) => {
            warn(
                account,
                &format!("Could not read where the mailbox's change history stands: {e}"),
            );
            return HistoryRefresh::Failed;
        }
    };
    let mut stored = match db.state_refresh_candidates(&account.id, now - REFRESH_WINDOW_SECS, MAX_REFRESH_ROWS) {
        Ok(rows) => rows,
        Err(e) => {
            warn(
                account,
                &format!("Could not read stored mail to refresh its state: {e}"),
            );
            return HistoryRefresh::Failed;
        }
    };
    // No provider knows a locally-composed Sent row's synthetic id.
    stored.retain(|row| !row.id.starts_with(LOCAL_SENT_ID_PREFIX));

    let mut applied = Applied::default();
    if !stored.is_empty() {
        let ids: Vec<String> = stored.iter().map(|row| row.id.clone()).collect();
        let remote = match email_provider.fetch_message_labels(&ids).await {
            Ok(remote) => remote,
            Err(e) => {
                warn(account, &format!("Could not refresh the state of stored mail: {e}"));
                return HistoryRefresh::Failed;
            }
        };
        let all_applied = apply_changes(db, account, plan_label_snapshot(&stored, &remote), &mut applied);
        applied.log(account);
        // A message the provider could not check (a throttled sub-request)
        // may have changed too: come back for it instead of starting over.
        let all_checked = ids.iter().all(|id| remote.contains_key(id));
        if !all_applied || !all_checked {
            return HistoryRefresh::Failed;
        }
    }
    save_cursor(db, account, key, &fresh)
}

fn save_cursor(db: &Database, account: &Account, key: &str, cursor: &str) -> HistoryRefresh {
    match db.set_preference(key, cursor) {
        Ok(()) => HistoryRefresh::Answered,
        Err(e) => {
            warn(account, &format!("Could not record the change-history cursor: {e}"));
            HistoryRefresh::Failed
        }
    }
}

/// What a pass changed, for its one summary line.
#[derive(Default)]
struct Applied {
    read_changes: u32,
    star_changes: u32,
    moved: u32,
    removed: u32,
}

impl Applied {
    fn log(&self, account: &Account) {
        if self.read_changes + self.star_changes + self.moved + self.removed > 0 {
            emit_account_log(
                "success",
                "sync",
                &account.email,
                &format!(
                    "Matched the account: {} read-state change(s), {} star change(s), {} moved, {} deleted elsewhere",
                    self.read_changes, self.star_changes, self.moved, self.removed
                ),
            );
        }
    }
}

/// Apply one page of the change log. `false` when part of it has to be
/// replayed: a change failed, or a row with a pending local push was skipped.
fn apply_page(db: &Database, account: &Account, changes: &[MessageChange], applied: &mut Applied) -> bool {
    if changes.is_empty() {
        return true;
    }
    let mut ids: Vec<String> = changes
        .iter()
        .map(|change| match change {
            MessageChange::LabelsAdded { id, .. }
            | MessageChange::LabelsRemoved { id, .. }
            | MessageChange::Deleted { id } => id.clone(),
        })
        .collect();
    ids.sort();
    ids.dedup();
    let stored = match db.stored_states_for_ids(&account.id, &ids) {
        Ok(rows) => rows,
        Err(e) => {
            warn(
                account,
                &format!("Could not read stored mail to refresh its state: {e}"),
            );
            return false;
        }
    };
    let plan = plan_history_changes(&stored, changes);
    apply_changes(db, account, plan.changes, applied) && !plan.held_back
}

/// Write the planned changes. Each statement repeats the pending-push guard,
/// so a change made locally since the rows were read is not overwritten.
/// `false` when a write failed.
fn apply_changes(db: &Database, account: &Account, changes: Vec<LocalChange>, applied: &mut Applied) -> bool {
    let mut all_applied = true;
    for change in changes {
        let (result, counter, id) = match &change {
            LocalChange::SetRead { id, is_read } => {
                (db.apply_server_read_state(id, *is_read), &mut applied.read_changes, id)
            }
            LocalChange::SetMailbox { id, mailbox } => (db.apply_server_mailbox(id, mailbox), &mut applied.moved, id),
            LocalChange::SetStarred { id, is_starred } => {
                (db.apply_server_starred(id, *is_starred), &mut applied.star_changes, id)
            }
            LocalChange::Delete { id } => (db.apply_server_delete(id), &mut applied.removed, id),
        };
        match result {
            Ok(true) => *counter += 1,
            Ok(false) => {}
            Err(e) => {
                warn(
                    account,
                    &format!("Could not apply a change made elsewhere to {id}: {e}"),
                );
                all_applied = false;
            }
        }
    }
    all_applied
}

/// One local change the refresh has decided on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LocalChange {
    SetRead {
        id: String,
        is_read: bool,
    },
    SetMailbox {
        id: String,
        mailbox: String,
    },
    SetStarred {
        id: String,
        is_starred: bool,
    },
    /// Deleted for good at the provider: soft-delete the row.
    Delete {
        id: String,
    },
}

/// What one page of the change log means for the stored rows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct HistoryPlan {
    pub changes: Vec<LocalChange>,
    /// The page changed a row that has a pending local push. The row was left
    /// alone, so the page must be replayed later.
    pub held_back: bool,
}

/// Pure: fold a page of the change log onto the stored rows.
///
/// - Changes to messages that are not stored are ignored: the fetch passes
///   ingest new mail.
/// - Several changes to one message collapse, in order, to its final state.
/// - A row with a pending local push is left alone and holds the page back.
pub(super) fn plan_history_changes(stored: &[StoredMessageState], changes: &[MessageChange]) -> HistoryPlan {
    let rows: HashMap<&str, &StoredMessageState> = stored.iter().map(|row| (row.id.as_str(), row)).collect();
    // Final state per message, in order of first appearance: its labels, or
    // `None` once it is deleted for good.
    let mut order: Vec<&StoredMessageState> = Vec::new();
    let mut finals: HashMap<&str, Option<BTreeSet<String>>> = HashMap::new();
    let mut plan = HistoryPlan::default();

    for change in changes {
        let id = match change {
            MessageChange::LabelsAdded { id, .. }
            | MessageChange::LabelsRemoved { id, .. }
            | MessageChange::Deleted { id } => id.as_str(),
        };
        let Some(row) = rows.get(id).copied() else { continue };
        if row.read_push_pending || row.star_push_pending {
            plan.held_back = true;
            continue;
        }
        let state = finals.entry(row.id.as_str()).or_insert_with(|| {
            order.push(row);
            Some(implied_labels(row))
        });
        match (change, state.as_mut()) {
            (MessageChange::Deleted { .. }, _) => *state = None,
            (MessageChange::LabelsAdded { labels, .. }, Some(current)) => current.extend(labels.iter().cloned()),
            (MessageChange::LabelsRemoved { labels, .. }, Some(current)) => {
                for label in labels {
                    current.remove(label);
                }
            }
            // Already deleted for good: nothing later can apply.
            (_, None) => {}
        }
    }

    for row in order {
        match finals.get(row.id.as_str()) {
            Some(Some(labels)) => {
                let labels: Vec<String> = labels.iter().cloned().collect();
                plan.changes.extend(changes_for_labels(row, &labels));
            }
            Some(None) => plan.changes.push(LocalChange::Delete { id: row.id.clone() }),
            None => {}
        }
    }
    plan
}

/// Pure: diff the stored rows against the labels the provider holds now. A
/// row the provider said nothing about, or one with a pending local push, is
/// left alone.
pub(super) fn plan_label_snapshot(
    stored: &[StoredMessageState],
    remote: &HashMap<String, RemoteLabels>,
) -> Vec<LocalChange> {
    stored
        .iter()
        .filter(|row| !row.read_push_pending && !row.star_push_pending)
        .flat_map(|row| match remote.get(&row.id) {
            Some(RemoteLabels::Present(labels)) => changes_for_labels(row, labels),
            Some(RemoteLabels::Missing) => vec![LocalChange::Delete { id: row.id.clone() }],
            None => Vec::new(),
        })
        .collect()
}

/// The labels a stored row stands for — the inverse of the mapping it was
/// stored with, for the labels that mapping reads.
fn implied_labels(row: &StoredMessageState) -> BTreeSet<String> {
    let mut labels = BTreeSet::new();
    let place = match row.mailbox.as_str() {
        "trash" => Some("TRASH"),
        "spam" => Some("SPAM"),
        "sent" => Some("SENT"),
        // Archived: none of the labels the mapping reads.
        "archive" => None,
        _ => Some("INBOX"),
    };
    labels.extend(place.map(str::to_string));
    if row.is_starred {
        labels.insert("STARRED".to_string());
    }
    if row.is_sent {
        labels.insert("SENT".to_string());
    }
    if !row.is_read {
        labels.insert("UNREAD".to_string());
    }
    labels
}

/// What has to change locally for `row` to match `labels`.
fn changes_for_labels(row: &StoredMessageState, labels: &[String]) -> Vec<LocalChange> {
    let mut changes = Vec::new();
    let is_read = !labels.iter().any(|l| l == "UNREAD");
    if is_read != row.is_read {
        changes.push(LocalChange::SetRead {
            id: row.id.clone(),
            is_read,
        });
    }
    let is_starred = labels.iter().any(|l| l == "STARRED");
    if is_starred != row.is_starred {
        changes.push(LocalChange::SetStarred {
            id: row.id.clone(),
            is_starred,
        });
    }
    // Spam, on either side, belongs to the spam reconciliation.
    let mailbox = mailbox_from_labels(labels);
    if mailbox != row.mailbox && mailbox != "spam" && row.mailbox != "spam" {
        changes.push(LocalChange::SetMailbox {
            id: row.id.clone(),
            mailbox: mailbox.to_string(),
        });
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(id: &str, mailbox: &str, is_read: bool) -> StoredMessageState {
        StoredMessageState {
            id: id.to_string(),
            message_id: Some(format!("<{id}@example.com>")),
            mailbox: mailbox.to_string(),
            is_read,
            is_sent: mailbox == "sent",
            read_push_pending: false,
            is_starred: false,
            star_push_pending: false,
        }
    }

    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|l| l.to_string()).collect()
    }

    fn added(id: &str, items: &[&str]) -> MessageChange {
        MessageChange::LabelsAdded {
            id: id.to_string(),
            labels: labels(items),
        }
    }

    fn removed(id: &str, items: &[&str]) -> MessageChange {
        MessageChange::LabelsRemoved {
            id: id.to_string(),
            labels: labels(items),
        }
    }

    fn deleted(id: &str) -> MessageChange {
        MessageChange::Deleted { id: id.to_string() }
    }

    fn set_read(id: &str, is_read: bool) -> LocalChange {
        LocalChange::SetRead {
            id: id.to_string(),
            is_read,
        }
    }

    fn set_mailbox(id: &str, mailbox: &str) -> LocalChange {
        LocalChange::SetMailbox {
            id: id.to_string(),
            mailbox: mailbox.to_string(),
        }
    }

    fn delete(id: &str) -> LocalChange {
        LocalChange::Delete { id: id.to_string() }
    }

    fn set_starred(id: &str, is_starred: bool) -> LocalChange {
        LocalChange::SetStarred {
            id: id.to_string(),
            is_starred,
        }
    }

    #[test]
    fn a_row_with_a_pending_star_holds_its_page_back() {
        let row = StoredMessageState {
            is_starred: true,
            star_push_pending: true,
            ..stored("m-1", "inbox", true)
        };
        let plan = plan_history_changes(std::slice::from_ref(&row), &[removed("m-1", &["STARRED"])]);
        assert!(plan.changes.is_empty());
        assert!(plan.held_back);
        assert!(plan_label_snapshot(&[row], &snapshot(&[("m-1", present(&["INBOX"]))])).is_empty());
    }

    // ── change log ────────────────────────────────────────────────────────

    #[test]
    fn a_page_of_the_change_log_becomes_local_changes() {
        let self_sent = StoredMessageState {
            is_sent: true,
            ..stored("m-1", "inbox", true)
        };
        let cases: Vec<(&str, StoredMessageState, Vec<MessageChange>, Vec<LocalChange>)> = vec![
            (
                "read elsewhere",
                stored("m-1", "inbox", false),
                vec![removed("m-1", &["UNREAD"])],
                vec![set_read("m-1", true)],
            ),
            (
                "marked unread elsewhere",
                stored("m-1", "inbox", true),
                vec![added("m-1", &["UNREAD"])],
                vec![set_read("m-1", false)],
            ),
            (
                "already read here",
                stored("m-1", "inbox", true),
                vec![removed("m-1", &["UNREAD"])],
                vec![],
            ),
            (
                "trashed elsewhere",
                stored("m-1", "inbox", true),
                vec![removed("m-1", &["INBOX"]), added("m-1", &["TRASH"])],
                vec![set_mailbox("m-1", "trash")],
            ),
            (
                "restored from Trash elsewhere",
                stored("m-1", "trash", true),
                vec![removed("m-1", &["TRASH"]), added("m-1", &["INBOX"])],
                vec![set_mailbox("m-1", "inbox")],
            ),
            (
                "a sent message restored from Trash goes back to Sent",
                StoredMessageState {
                    is_sent: true,
                    ..stored("m-1", "trash", true)
                },
                vec![removed("m-1", &["TRASH"])],
                vec![set_mailbox("m-1", "sent")],
            ),
            (
                "archived elsewhere",
                stored("m-1", "inbox", true),
                vec![removed("m-1", &["INBOX"])],
                vec![set_mailbox("m-1", "archive")],
            ),
            (
                "moved to a user label elsewhere is archived too",
                stored("m-1", "inbox", true),
                vec![removed("m-1", &["INBOX"]), added("m-1", &["Label_7"])],
                vec![set_mailbox("m-1", "archive")],
            ),
            (
                "moved back to the inbox elsewhere",
                stored("m-1", "archive", true),
                vec![added("m-1", &["INBOX"])],
                vec![set_mailbox("m-1", "inbox")],
            ),
            (
                "a label added to archived mail keeps it archived",
                stored("m-1", "archive", true),
                vec![added("m-1", &["Label_7"])],
                vec![],
            ),
            (
                "starred elsewhere",
                stored("m-1", "inbox", true),
                vec![added("m-1", &["STARRED"])],
                vec![set_starred("m-1", true)],
            ),
            (
                "unstarred elsewhere",
                StoredMessageState {
                    is_starred: true,
                    ..stored("m-1", "archive", true)
                },
                vec![removed("m-1", &["STARRED"])],
                vec![set_starred("m-1", false)],
            ),
            (
                "a message sent to oneself, archived, is only in Sent",
                self_sent.clone(),
                vec![removed("m-1", &["INBOX"])],
                vec![set_mailbox("m-1", "sent")],
            ),
            (
                "a sent message moved to the inbox",
                stored("m-1", "sent", true),
                vec![added("m-1", &["INBOX"])],
                vec![set_mailbox("m-1", "inbox")],
            ),
            (
                "marked as spam elsewhere: left to the spam reconciliation",
                stored("m-1", "inbox", true),
                vec![removed("m-1", &["INBOX"]), added("m-1", &["SPAM"])],
                vec![],
            ),
            (
                "deleted for good elsewhere",
                stored("m-1", "trash", true),
                vec![deleted("m-1")],
                vec![delete("m-1")],
            ),
            (
                "read and trashed in one page",
                stored("m-1", "inbox", false),
                vec![removed("m-1", &["UNREAD"]), added("m-1", &["TRASH"])],
                vec![set_read("m-1", true), set_mailbox("m-1", "trash")],
            ),
        ];
        for (label, row, changes, expected) in cases {
            let plan = plan_history_changes(&[row], &changes);
            assert_eq!(plan.changes, expected, "{label}");
            assert!(!plan.held_back, "{label}");
        }
    }

    #[test]
    fn several_records_for_one_message_collapse_to_the_final_state_in_order() {
        let cases: Vec<(&str, StoredMessageState, Vec<MessageChange>, Vec<LocalChange>)> = vec![
            (
                "read, then unread again",
                stored("m-1", "inbox", false),
                vec![removed("m-1", &["UNREAD"]), added("m-1", &["UNREAD"])],
                vec![],
            ),
            (
                "unread, then read",
                stored("m-1", "inbox", false),
                vec![added("m-1", &["UNREAD"]), removed("m-1", &["UNREAD"])],
                vec![set_read("m-1", true)],
            ),
            (
                "trashed, then restored",
                stored("m-1", "inbox", true),
                vec![
                    removed("m-1", &["INBOX"]),
                    added("m-1", &["TRASH"]),
                    removed("m-1", &["TRASH"]),
                    added("m-1", &["INBOX"]),
                ],
                vec![],
            ),
            (
                "restored, then trashed again",
                stored("m-1", "trash", true),
                vec![removed("m-1", &["TRASH"]), added("m-1", &["TRASH"])],
                vec![],
            ),
            (
                "trashed, then deleted for good",
                stored("m-1", "inbox", false),
                vec![added("m-1", &["TRASH"]), removed("m-1", &["UNREAD"]), deleted("m-1")],
                vec![delete("m-1")],
            ),
        ];
        for (label, row, changes, expected) in cases {
            assert_eq!(plan_history_changes(&[row], &changes).changes, expected, "{label}");
        }
    }

    #[test]
    fn changes_to_messages_that_are_not_stored_are_ignored() {
        let plan = plan_history_changes(
            &[stored("m-1", "inbox", true)],
            &[added("other", &["TRASH"]), deleted("gone"), removed("new", &["UNREAD"])],
        );
        assert_eq!(plan, HistoryPlan::default());
    }

    #[test]
    fn each_message_is_planned_on_its_own_in_order_of_first_appearance() {
        let plan = plan_history_changes(
            &[
                stored("a", "inbox", false),
                stored("b", "inbox", true),
                stored("c", "inbox", true),
            ],
            &[
                added("b", &["TRASH"]),
                removed("a", &["UNREAD"]),
                deleted("c"),
                removed("b", &["INBOX"]),
            ],
        );
        assert_eq!(
            plan.changes,
            vec![set_mailbox("b", "trash"), set_read("a", true), delete("c")]
        );
    }

    #[test]
    fn a_row_with_a_pending_local_push_is_never_touched_and_holds_the_page_back() {
        for change in [added("m-1", &["UNREAD"]), added("m-1", &["TRASH"]), deleted("m-1")] {
            let row = StoredMessageState {
                read_push_pending: true,
                ..stored("m-1", "inbox", true)
            };
            let plan = plan_history_changes(
                &[row, stored("m-2", "inbox", false)],
                &[change.clone(), removed("m-2", &["UNREAD"])],
            );
            assert_eq!(plan.changes, vec![set_read("m-2", true)], "{change:?}");
            assert!(plan.held_back, "{change:?}");
        }
    }

    #[test]
    fn a_pending_row_the_page_does_not_mention_holds_nothing_back() {
        let pending = StoredMessageState {
            read_push_pending: true,
            ..stored("m-1", "inbox", true)
        };
        let plan = plan_history_changes(
            &[pending, stored("m-2", "inbox", false)],
            &[removed("m-2", &["UNREAD"])],
        );
        assert!(!plan.held_back);
    }

    #[test]
    fn an_empty_page_or_no_stored_rows_plans_nothing() {
        assert_eq!(
            plan_history_changes(&[stored("m-1", "inbox", true)], &[]),
            HistoryPlan::default()
        );
        assert_eq!(plan_history_changes(&[], &[deleted("m-1")]), HistoryPlan::default());
    }

    // ── executor ──────────────────────────────────────────────────────────

    use crate::models::Email;
    use crate::services::emails::state_refresh::{
        refresh_stored_mail_state, REFRESH_INTERVAL_SECS, REFRESH_WINDOW_SECS,
    };
    use crate::sync::provider::{EmailCategory, FakeEmailProvider};

    const NOW: i64 = 40 * 86_400;
    /// Where the fake's change log stands when a test starts.
    const START: u64 = 1_000;

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
            is_starred: false,
            headers: None,
        }
    }

    /// A DB and a Gmail-like fake holding the same messages, with the change
    /// log already followed from [`START`].
    fn synced(emails: &[Email]) -> (Arc<Database>, FakeEmailProvider) {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(emails).unwrap();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.enable_history(START);
        for email in emails {
            provider.add_message(email.clone(), EmailCategory::Primary, vec![]);
        }
        db.set_preference(&history_cursor_key("acc-1"), &START.to_string())
            .unwrap();
        (Arc::new(db), provider)
    }

    fn cursor(db: &Database) -> Option<String> {
        db.get_preference(&history_cursor_key("acc-1")).unwrap()
    }

    /// `(mailbox, is_read)` of a row the user can still see.
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

    /// One pass, `passes` refresh intervals after [`NOW`].
    async fn pass(db: &Arc<Database>, provider: &FakeEmailProvider, passes: i64) {
        refresh_stored_mail_state(db, &account(), provider, NOW + passes * REFRESH_INTERVAL_SECS).await;
    }

    #[tokio::test]
    async fn the_first_pass_only_records_where_the_change_log_stands() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        db.delete_preference(&history_cursor_key("acc-1")).unwrap();
        provider.record_history(removed("m-1", &["UNREAD"]));

        pass(&db, &provider, 0).await;

        assert_eq!(cursor(&db), Some((START + 1).to_string()));
        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), false)), "nothing to replay");
        assert_eq!(provider.calls(), vec!["history_cursor".to_string()]);
    }

    #[tokio::test]
    async fn a_message_read_in_gmail_becomes_read_here() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        provider.record_history(removed("m-1", &["UNREAD"]));

        pass(&db, &provider, 0).await;

        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), true)));
    }

    #[tokio::test]
    async fn a_message_trashed_in_gmail_leaves_the_inbox_for_trash() {
        let (db, provider) = synced(&[email("m-1", "inbox", true)]);
        provider.record_history(removed("m-1", &["INBOX"]));
        provider.record_history(added("m-1", &["TRASH"]));

        pass(&db, &provider, 0).await;

        assert_eq!(row(&db, "m-1"), Some(("trash".to_string(), true)));
    }

    #[tokio::test]
    async fn a_message_archived_in_gmail_is_archived_here() {
        let (db, provider) = synced(&[email("m-1", "inbox", true)]);
        provider.record_history(removed("m-1", &["INBOX"]));

        pass(&db, &provider, 0).await;

        assert_eq!(row(&db, "m-1"), Some(("archive".to_string(), true)));
        assert_eq!(cursor(&db), Some((START + 1).to_string()));
    }

    #[tokio::test]
    async fn a_message_deleted_for_good_in_gmail_is_deleted_here_but_never_downloaded_again() {
        let (db, provider) = synced(&[email("m-1", "trash", true)]);
        provider.record_history(deleted("m-1"));

        pass(&db, &provider, 0).await;

        assert_eq!(row(&db, "m-1"), None);
        assert!(db.emails_exist_batch(&["m-1".to_string()]).unwrap().contains("m-1"));
    }

    #[tokio::test]
    async fn the_cursor_advances_once_a_pass_is_applied_and_nothing_is_replayed() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        provider.record_history(removed("m-1", &["UNREAD"]));
        pass(&db, &provider, 0).await;
        assert_eq!(cursor(&db), Some((START + 1).to_string()));

        // Marked unread again here; the old record must not be applied twice.
        db.apply_server_read_state("m-1", false).unwrap();
        pass(&db, &provider, 1).await;

        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), false)));
    }

    #[tokio::test]
    async fn a_failed_listing_leaves_the_cursor_and_is_retried_on_the_next_sync() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        provider.record_history(removed("m-1", &["UNREAD"]));
        provider.fail_history_listing(Some("503"));

        pass(&db, &provider, 0).await;
        assert_eq!(cursor(&db), Some(START.to_string()));
        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), false)));

        // Not throttled: the provider never answered.
        provider.fail_history_listing(None);
        refresh_stored_mail_state(&db, &account(), &provider, NOW + 1).await;
        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), true)));
        assert_eq!(cursor(&db), Some((START + 1).to_string()));
    }

    #[tokio::test]
    async fn a_pending_local_change_wins_and_its_page_is_replayed_once_it_is_pushed() {
        let (db, provider) = synced(&[email("m-1", "inbox", false), email("m-2", "inbox", false)]);
        db.mark_as_read_pending_push("m-1", NOW - 60).unwrap();
        provider.record_history(added("m-1", &["TRASH"]));
        provider.record_history(removed("m-2", &["UNREAD"]));

        pass(&db, &provider, 0).await;

        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), true)), "left alone");
        assert_eq!(row(&db, "m-2"), Some(("inbox".to_string(), true)), "the rest applies");
        assert_eq!(cursor(&db), Some(START.to_string()), "held before the skipped change");

        db.clear_read_push_pending("m-1").unwrap();
        pass(&db, &provider, 1).await;

        assert_eq!(row(&db, "m-1"), Some(("trash".to_string(), true)));
        assert_eq!(cursor(&db), Some((START + 2).to_string()));
    }

    #[tokio::test]
    async fn a_long_change_log_is_applied_a_bounded_number_of_pages_per_pass() {
        let emails: Vec<Email> = (0..MAX_HISTORY_PAGES_PER_PASS + 2)
            .map(|i| email(&format!("m-{i}"), "inbox", false))
            .collect();
        let (db, provider) = synced(&emails);
        provider.set_history_page_size(1);
        for email in &emails {
            provider.record_history(removed(&email.id, &["UNREAD"]));
        }
        let read = |db: &Database| emails.iter().filter(|e| row(db, &e.id).unwrap().1).count();

        pass(&db, &provider, 0).await;
        assert_eq!(read(&db), MAX_HISTORY_PAGES_PER_PASS);
        assert_eq!(
            cursor(&db),
            Some((START + MAX_HISTORY_PAGES_PER_PASS as u64).to_string()),
            "the last page applied"
        );

        pass(&db, &provider, 1).await;
        assert_eq!(read(&db), emails.len());
        assert_eq!(cursor(&db), Some((START + emails.len() as u64).to_string()));
    }

    #[tokio::test]
    async fn an_expired_cursor_falls_back_to_checking_recent_mail_and_starts_over() {
        let mut old = email("m-old", "inbox", false);
        old.timestamp = NOW - REFRESH_WINDOW_SECS - 1;
        let (db, provider) = synced(&[
            email("read", "inbox", false),
            email("trashed", "inbox", true),
            email("gone", "inbox", true),
            email("same", "inbox", true),
            old,
        ]);
        // Done in Gmail while the app was not following the log.
        provider.set_remote_read("read", true);
        provider.relocate_message("trashed", "trashed", "trash");
        provider.remove_message("gone");
        provider.remove_message("m-old");
        provider.expire_history();

        pass(&db, &provider, 0).await;

        assert_eq!(row(&db, "read"), Some(("inbox".to_string(), true)));
        assert_eq!(row(&db, "trashed"), Some(("trash".to_string(), true)));
        assert_eq!(row(&db, "gone"), None);
        assert_eq!(row(&db, "same"), Some(("inbox".to_string(), true)));
        assert!(row(&db, "m-old").is_some(), "outside the bounded window");
        assert_eq!(cursor(&db), Some((START + 1).to_string()), "reseeded");

        // The log is followed again from there.
        provider.record_history(added("same", &["UNREAD"]));
        pass(&db, &provider, 1).await;
        assert_eq!(row(&db, "same"), Some(("inbox".to_string(), false)));
    }

    #[tokio::test]
    async fn a_failed_fallback_keeps_the_expired_cursor_so_it_is_tried_again() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        provider.set_remote_read("m-1", true);
        provider.expire_history();
        provider.fail_label_fetch("503");

        pass(&db, &provider, 0).await;

        assert_eq!(cursor(&db), Some(START.to_string()));
        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), false)));
    }

    #[tokio::test]
    async fn a_fallback_that_could_not_check_every_message_is_repeated() {
        let (db, provider) = synced(&[email("m-1", "inbox", false), email("m-2", "inbox", false)]);
        provider.set_remote_read("m-1", true);
        provider.set_remote_read("m-2", true);
        provider.make_state_unverifiable("m-2");
        provider.expire_history();

        pass(&db, &provider, 0).await;

        assert_eq!(
            row(&db, "m-1"),
            Some(("inbox".to_string(), true)),
            "what was checked applies"
        );
        assert_eq!(row(&db, "m-2"), Some(("inbox".to_string(), false)));
        assert_eq!(
            cursor(&db),
            Some(START.to_string()),
            "not reseeded over an unchecked row"
        );
    }

    #[tokio::test]
    async fn changes_to_mail_that_is_not_stored_write_nothing() {
        let (db, provider) = synced(&[]);
        provider.record_history(added("unknown", &["TRASH"]));
        provider.record_history(deleted("other"));

        pass(&db, &provider, 0).await;

        let rows: i64 = db
            .reader()
            .query_row("SELECT COUNT(*) FROM emails", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
        assert_eq!(cursor(&db), Some((START + 2).to_string()));
    }

    #[tokio::test]
    async fn the_change_log_is_read_at_most_once_per_interval() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        pass(&db, &provider, 0).await;
        provider.record_history(removed("m-1", &["UNREAD"]));

        refresh_stored_mail_state(&db, &account(), &provider, NOW + REFRESH_INTERVAL_SECS - 1).await;
        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), false)), "still throttled");

        pass(&db, &provider, 1).await;
        assert_eq!(row(&db, "m-1"), Some(("inbox".to_string(), true)));
    }

    #[tokio::test]
    async fn a_provider_with_a_change_log_is_never_polled_for_message_states() {
        let (db, provider) = synced(&[email("m-1", "inbox", false)]);
        provider.report_message_states();

        pass(&db, &provider, 0).await;

        assert_eq!(provider.calls(), vec!["list_history".to_string()]);
    }

    // ── label snapshot (the fallback when the cursor expired) ─────────────

    fn snapshot(entries: &[(&str, RemoteLabels)]) -> HashMap<String, RemoteLabels> {
        entries.iter().map(|(id, l)| (id.to_string(), l.clone())).collect()
    }

    fn present(items: &[&str]) -> RemoteLabels {
        RemoteLabels::Present(labels(items))
    }

    #[test]
    fn a_label_snapshot_is_diffed_against_the_stored_rows() {
        let pending = StoredMessageState {
            read_push_pending: true,
            ..stored("pending", "inbox", true)
        };
        let plan = plan_label_snapshot(
            &[
                stored("same", "inbox", true),
                stored("read", "inbox", false),
                stored("trashed", "inbox", true),
                stored("archived", "inbox", true),
                stored("spammed", "inbox", true),
                stored("gone", "inbox", true),
                stored("unchecked", "inbox", false),
                pending,
            ],
            &snapshot(&[
                ("same", present(&["INBOX"])),
                ("read", present(&["INBOX"])),
                ("trashed", present(&["TRASH"])),
                ("archived", present(&["Label_7"])),
                ("spammed", present(&["SPAM"])),
                ("gone", RemoteLabels::Missing),
                ("pending", present(&["TRASH", "UNREAD"])),
            ]),
        );
        assert_eq!(
            plan,
            vec![
                set_read("read", true),
                set_mailbox("trashed", "trash"),
                set_mailbox("archived", "archive"),
                delete("gone")
            ]
        );
    }
}
