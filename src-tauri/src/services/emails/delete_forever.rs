//! "Delete forever" on messages in Trash or Spam, and "Empty Trash" /
//! "Empty Spam" (issue #304).
//!
//! Pure planner [`plan_delete_forever`] decides what happens to each row; the
//! executor [`execute_delete_forever`] asks the provider in batches and
//! hard-deletes a row only once the provider confirmed its message is gone.
//! Cancelling stops between batches, and the report says what actually
//! happened.
//!
//! Only the folder being emptied is ever touched: the provider is asked only
//! about rows filed in that folder, and its own implementation refuses any id
//! outside it (`EmailProvider::delete_forever`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::db::emails::folder_ops::DeleteForeverRow;
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Account;
use crate::services::app_handle::AppHandle;
use crate::sync::provider::{
    provider_supports_delete_forever, DeleteForeverFolder, DeleteForeverTarget, EmailProvider,
};

/// Messages per provider call: the granularity of progress and of Cancel.
pub const DELETE_FOREVER_BATCH: usize = 50;

/// What "Delete forever" does with each stored row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteForeverPlan {
    /// Selected ids that are not in the folder (or not in the account):
    /// refused, never touched.
    pub refused: Vec<String>,
    /// Rows in the Trash view only because EmailOps deleted them: the account
    /// keeps no copy in its Trash under that id, so they are removed here and
    /// the provider is not asked.
    pub local_only: Vec<String>,
    /// What the provider deletes forever, in batches.
    pub batches: Vec<Vec<DeleteForeverTarget>>,
}

impl DeleteForeverPlan {
    /// Every message the run accounts for.
    pub fn total(&self) -> usize {
        self.refused.len() + self.local_only.len() + self.batches.iter().map(Vec::len).sum::<usize>()
    }
}

/// Whether `row` is in `folder` as its list view shows it: Trash is the
/// provider's Trash plus what EmailOps deleted; Spam is live spam only.
fn in_folder(folder: DeleteForeverFolder, row: &DeleteForeverRow) -> bool {
    match folder {
        DeleteForeverFolder::Trash => row.mailbox == folder.mailbox() || row.is_deleted,
        DeleteForeverFolder::Spam => row.mailbox == folder.mailbox() && !row.is_deleted,
    }
}

/// Pure. `rows` are the stored rows (from `Database::delete_forever_rows`);
/// `requested` is the user's selection, `None` for the whole folder.
pub fn plan_delete_forever(
    folder: DeleteForeverFolder,
    rows: Vec<DeleteForeverRow>,
    requested: Option<&[String]>,
    batch_size: usize,
) -> DeleteForeverPlan {
    // The selection's order, each id once; the rows' own order otherwise.
    let order: Vec<String> = match requested {
        Some(requested) => {
            let mut seen = std::collections::HashSet::new();
            requested.iter().filter(|id| seen.insert(*id)).cloned().collect()
        }
        None => rows.iter().map(|row| row.id.clone()).collect(),
    };
    let mut by_id: std::collections::HashMap<String, DeleteForeverRow> =
        rows.into_iter().map(|row| (row.id.clone(), row)).collect();

    let mut plan = DeleteForeverPlan::default();
    let mut targets = Vec::new();
    for id in order {
        match by_id.remove(&id) {
            Some(row) if in_folder(folder, &row) => {
                if row.mailbox == folder.mailbox() {
                    targets.push(DeleteForeverTarget {
                        id: row.id,
                        message_id_header: row.message_id,
                    });
                } else {
                    plan.local_only.push(row.id);
                }
            }
            _ => plan.refused.push(id),
        }
    }
    plan.batches = targets.chunks(batch_size.max(1)).map(<[_]>::to_vec).collect();
    plan
}

/// An error in the same `{code, params, message}` shape `AppError` has at
/// the command boundary, owned so a report can carry it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteForeverError {
    pub code: String,
    pub params: std::collections::BTreeMap<String, String>,
    pub message: String,
}

impl From<&AppError> for DeleteForeverError {
    /// Through `AppError`'s own wire format, which keeps library and OS
    /// detail (SQLite messages, file paths) out of the webview and logs it.
    fn from(e: &AppError) -> Self {
        serde_json::to_value(e)
            .and_then(serde_json::from_value)
            .unwrap_or_else(|_| Self {
                code: e.code().to_string(),
                params: std::collections::BTreeMap::new(),
                message: e.code().to_string(),
            })
    }
}

/// A message that is still there, and why.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteForeverFailure {
    pub email_id: String,
    pub error: DeleteForeverError,
}

/// How far a run is: `processed` of `total` messages answered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteForeverProgress {
    pub processed: u32,
    pub total: u32,
    pub removed: u32,
    pub failed: u32,
}

/// What a run actually did. `removed + failed.len() + not_attempted ==
/// total`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteForeverReport {
    pub total: u32,
    /// Deleted forever (or already gone at the provider), and removed here.
    pub removed: u32,
    /// Asked about and still there.
    pub failed: Vec<DeleteForeverFailure>,
    /// Left as they were without being tried: the run was cancelled or
    /// stopped early.
    pub not_attempted: u32,
    pub cancelled: bool,
    /// What stopped the run early (offline, sign-in, a database error).
    pub error: Option<DeleteForeverError>,
}

impl DeleteForeverReport {
    fn progress(&self) -> DeleteForeverProgress {
        DeleteForeverProgress {
            processed: self.removed + count(self.failed.len()),
            total: self.total,
            removed: self.removed,
            failed: count(self.failed.len()),
        }
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Carry out `plan`. `provider` may be `None` only when the plan has no
/// batches. `is_cancelled` is checked before every batch; `on_progress` is
/// called after the local-only rows and after every batch.
pub async fn execute_delete_forever(
    db: &Database,
    provider: Option<&dyn EmailProvider>,
    folder: DeleteForeverFolder,
    plan: DeleteForeverPlan,
    is_cancelled: &(dyn Fn() -> bool + Send + Sync),
    on_progress: &(dyn Fn(DeleteForeverProgress) + Send + Sync),
) -> DeleteForeverReport {
    let mut report = DeleteForeverReport {
        total: count(plan.total()),
        ..DeleteForeverReport::default()
    };
    let folder_name = folder_name(folder);
    for email_id in plan.refused {
        let refusal = AppError::InvalidInput(format!("The message is not in {folder_name}"));
        report.failed.push(DeleteForeverFailure {
            email_id,
            error: (&refusal).into(),
        });
    }
    let pending: usize = plan.batches.iter().map(Vec::len).sum();
    if !plan.local_only.is_empty() {
        if let Err(e) = db.hard_delete_emails(&plan.local_only) {
            report.error = Some((&e).into());
            report.not_attempted = count(plan.local_only.len() + pending);
            return report;
        }
        report.removed += count(plan.local_only.len());
    }
    on_progress(report.progress());

    let mut left = pending;
    for batch in plan.batches {
        if is_cancelled() {
            report.cancelled = true;
            report.not_attempted = count(left);
            return report;
        }
        left -= batch.len();
        let answers = match provider {
            Some(provider) => provider.delete_forever(folder, &batch).await,
            None => Err(AppError::InvalidInput(
                "no mail provider to delete these messages with".to_string(),
            )),
        };
        let mut answers = match answers {
            Ok(answers) => answers.into_iter(),
            Err(e) => {
                report.error = Some((&e).into());
                report.not_attempted = count(batch.len() + left);
                return report;
            }
        };
        let mut gone = Vec::new();
        for target in batch {
            // A message the provider said nothing about is still there.
            let answer = answers
                .next()
                .unwrap_or_else(|| Err(AppError::SyncError("the mail provider did not answer".to_string())));
            match answer {
                Ok(()) | Err(AppError::NotFound(_)) => gone.push(target.id),
                Err(e) => report.failed.push(DeleteForeverFailure {
                    email_id: target.id,
                    error: (&e).into(),
                }),
            }
        }
        // Gone from the account either way. Their rows stay in the view if
        // this fails, and a later run finds them gone and removes them.
        report.removed += count(gone.len());
        if let Err(e) = db.hard_delete_emails(&gone) {
            crate::services::logger::log(
                "error",
                "sync",
                format!(
                    "{} message(s) were deleted forever but stay listed here until the next Delete forever: {e}",
                    gone.len()
                ),
            );
            report.error = Some((&e).into());
            report.not_attempted = count(left);
            return report;
        }
        on_progress(report.progress());
    }
    report
}

/// The account and plan for a run. Refuses an account whose provider
/// cannot delete forever (Gmail: see `provider_supports_delete_forever`)
/// before anything is read or touched.
pub fn prepare_delete_forever(
    db: &Database,
    account_id: &str,
    folder: DeleteForeverFolder,
    ids: Option<&[String]>,
) -> Result<(Account, DeleteForeverPlan)> {
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {account_id} not found")))?;
    if !provider_supports_delete_forever(&account.provider) {
        return Err(AppError::InvalidInput(format!(
            "Deleting forever is not available for {} accounts",
            account.provider
        )));
    }
    let rows = db.delete_forever_rows(account_id, folder, ids)?;
    let plan = plan_delete_forever(folder, rows, ids, DELETE_FOREVER_BATCH);
    Ok((account, plan))
}

/// Event with the progress of a run (`runId`, `accountId`, `folder`,
/// `processed`, `total`, `removed`, `failed`).
pub const PROGRESS_EVENT: &str = "delete-forever-progress";
/// Event with how a run ended: `runId`, `accountId`, `folder` and the
/// [`DeleteForeverReport`] fields. Emitted exactly once per run.
pub const FINISHED_EVENT: &str = "delete-forever-finished";

/// Run one "Delete forever" / "Empty" request and report it: progress
/// events as it goes, one finished event at the end, and output-panel lines
/// at the start and the end. `ids: None` empties the whole folder.
pub async fn run_delete_forever(
    db: &Arc<Database>,
    account_id: &str,
    folder: DeleteForeverFolder,
    ids: Option<Vec<String>>,
    run_id: &str,
    app: Option<AppHandle>,
    ticket: &DeleteForeverTicket,
) -> DeleteForeverReport {
    let folder_name = folder_name(folder);
    let report = match prepare_delete_forever(db, account_id, folder, ids.as_deref()) {
        Err(e) => {
            crate::services::logger::log(
                "error",
                "sync",
                format!("Could not delete forever from {folder_name}: {e}"),
            );
            DeleteForeverReport {
                error: Some((&e).into()),
                ..DeleteForeverReport::default()
            }
        }
        Ok((account, plan)) => {
            let total = plan.total();
            super::emit_account_log(
                "info",
                "sync",
                &account.email,
                &format!("Deleting {total} message(s) forever from {folder_name}"),
            );
            let provider = if plan.batches.is_empty() {
                Ok(None)
            } else {
                super::build_provider(&account, app).await.map(Some)
            };
            let report = match provider {
                Ok(provider) => {
                    let on_progress = |progress: DeleteForeverProgress| {
                        crate::services::events::emit(
                            PROGRESS_EVENT,
                            serde_json::json!({
                                "runId": run_id,
                                "accountId": account_id,
                                "folder": folder,
                                "processed": progress.processed,
                                "total": progress.total,
                                "removed": progress.removed,
                                "failed": progress.failed,
                            }),
                        );
                    };
                    execute_delete_forever(
                        db,
                        provider.as_deref(),
                        folder,
                        plan,
                        &|| ticket.is_cancelled(),
                        &on_progress,
                    )
                    .await
                }
                // Offline or signed out: nothing was touched.
                Err(e) => DeleteForeverReport {
                    total: count(total),
                    not_attempted: count(total),
                    error: Some((&e).into()),
                    ..DeleteForeverReport::default()
                },
            };
            log_outcome(&account.email, folder_name, &report);
            report
        }
    };
    crate::services::events::emit(
        FINISHED_EVENT,
        DeleteForeverFinished {
            run_id,
            account_id,
            folder,
            report: &report,
        },
    );
    report
}

/// Payload of [`FINISHED_EVENT`].
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteForeverFinished<'a> {
    run_id: &'a str,
    account_id: &'a str,
    folder: DeleteForeverFolder,
    #[serde(flatten)]
    report: &'a DeleteForeverReport,
}

fn folder_name(folder: DeleteForeverFolder) -> &'static str {
    match folder {
        DeleteForeverFolder::Trash => "Trash",
        DeleteForeverFolder::Spam => "Spam",
    }
}

/// One output-panel line with what the run did.
fn log_outcome(account_email: &str, folder_name: &str, report: &DeleteForeverReport) {
    let mut message = format!(
        "Deleted {} of {} message(s) forever from {folder_name}",
        report.removed, report.total
    );
    if !report.failed.is_empty() {
        message.push_str(&format!("; {} could not be deleted", report.failed.len()));
        if let Some(first) = report.failed.first() {
            message.push_str(&format!(" ({})", first.error.message));
        }
    }
    if report.cancelled {
        message.push_str(&format!("; cancelled, {} left untouched", report.not_attempted));
    }
    if let Some(error) = &report.error {
        message.push_str(&format!(
            "; stopped, {} left untouched: {}",
            report.not_attempted, error.message
        ));
    }
    let level = if report.error.is_some() || !report.failed.is_empty() {
        "error"
    } else if report.cancelled {
        "info"
    } else {
        "success"
    };
    super::emit_account_log(level, "sync", account_email, &message);
}

/// The runs in flight, by the caller's run id, so Cancel can reach one.
#[derive(Default)]
pub struct DeleteForeverRuns {
    running: std::sync::Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>,
}

/// A registered run. Dropping it unregisters the run.
pub struct DeleteForeverTicket {
    runs: Arc<DeleteForeverRuns>,
    run_id: String,
    cancelled: Arc<AtomicBool>,
}

impl DeleteForeverRuns {
    pub fn begin(self: &Arc<Self>, run_id: &str) -> DeleteForeverTicket {
        let cancelled = Arc::new(AtomicBool::new(false));
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run_id.to_string(), Arc::clone(&cancelled));
        DeleteForeverTicket {
            runs: Arc::clone(self),
            run_id: run_id.to_string(),
            cancelled,
        }
    }

    /// Ask the run to stop before its next batch; unknown ids are ignored.
    pub fn cancel(&self, run_id: &str) {
        if let Some(flag) = self
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(run_id)
        {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

impl DeleteForeverTicket {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || crate::services::task_queue::cancel_requested()
    }
}

impl Drop for DeleteForeverTicket {
    fn drop(&mut self) {
        let mut running = self
            .runs
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if running
            .get(&self.run_id)
            .is_some_and(|flag| Arc::ptr_eq(flag, &self.cancelled))
        {
            running.remove(&self.run_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, mailbox: &str, is_deleted: bool) -> DeleteForeverRow {
        DeleteForeverRow {
            id: id.to_string(),
            mailbox: mailbox.to_string(),
            is_deleted,
            message_id: Some(format!("<{id}@example.com>")),
        }
    }

    fn target(id: &str) -> DeleteForeverTarget {
        DeleteForeverTarget {
            id: id.to_string(),
            message_id_header: Some(format!("<{id}@example.com>")),
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn trash_rows_filed_in_trash_go_to_the_provider() {
        let plan = plan_delete_forever(
            DeleteForeverFolder::Trash,
            vec![row("t-1", "trash", false), row("t-2", "trash", true)],
            None,
            50,
        );

        assert_eq!(plan.batches, vec![vec![target("t-1"), target("t-2")]]);
        assert!(plan.local_only.is_empty() && plan.refused.is_empty());
    }

    // A message EmailOps deleted that is still filed elsewhere (the Sent copy
    // of a message sent to yourself, an Outlook id re-keyed by the move) has
    // no copy in the account's Trash under that id: asking the provider to
    // delete it forever would reach another folder.
    #[test]
    fn trash_rows_deleted_here_but_filed_elsewhere_are_removed_locally_only() {
        let plan = plan_delete_forever(
            DeleteForeverFolder::Trash,
            vec![row("gone-1", "inbox", true), row("gone-2", "sent", true)],
            None,
            50,
        );

        assert_eq!(plan.local_only, ids(&["gone-1", "gone-2"]));
        assert!(plan.batches.is_empty());
    }

    #[test]
    fn spam_rows_go_to_the_provider() {
        let plan = plan_delete_forever(DeleteForeverFolder::Spam, vec![row("s-1", "spam", false)], None, 50);

        assert_eq!(plan.batches, vec![vec![target("s-1")]]);
    }

    #[test]
    fn selected_messages_outside_the_folder_are_refused_and_never_sent() {
        let requested = ids(&["in-1", "hidden-spam", "t-1"]);
        let plan = plan_delete_forever(
            DeleteForeverFolder::Spam,
            vec![
                row("in-1", "inbox", false),
                row("hidden-spam", "spam", true),
                row("t-1", "trash", false),
            ],
            Some(&requested),
            50,
        );

        assert_eq!(plan.refused, ids(&["in-1", "hidden-spam", "t-1"]));
        assert!(plan.batches.is_empty() && plan.local_only.is_empty());
    }

    #[test]
    fn selected_ids_the_account_does_not_have_are_refused() {
        let requested = ids(&["t-1", "nope"]);
        let plan = plan_delete_forever(
            DeleteForeverFolder::Trash,
            vec![row("t-1", "trash", false)],
            Some(&requested),
            50,
        );

        assert_eq!(plan.refused, ids(&["nope"]));
        assert_eq!(plan.batches, vec![vec![target("t-1")]]);
    }

    #[test]
    fn a_selection_naming_a_message_twice_deletes_it_once() {
        let requested = ids(&["t-1", "t-1"]);
        let plan = plan_delete_forever(
            DeleteForeverFolder::Trash,
            vec![row("t-1", "trash", false)],
            Some(&requested),
            50,
        );

        assert_eq!(plan.batches, vec![vec![target("t-1")]]);
        assert_eq!(plan.total(), 1);
    }

    #[test]
    fn provider_targets_are_split_into_batches_in_order() {
        let rows: Vec<_> = (0..5).map(|i| row(&format!("t-{i}"), "trash", false)).collect();

        let plan = plan_delete_forever(DeleteForeverFolder::Trash, rows, None, 2);

        let batch_ids: Vec<Vec<String>> = plan
            .batches
            .iter()
            .map(|b| b.iter().map(|t| t.id.clone()).collect())
            .collect();
        assert_eq!(
            batch_ids,
            vec![ids(&["t-0", "t-1"]), ids(&["t-2", "t-3"]), ids(&["t-4"])]
        );
    }

    #[test]
    fn a_row_without_a_message_id_is_targeted_by_id_alone() {
        let mut bare = row("t-1", "trash", false);
        bare.message_id = None;

        let plan = plan_delete_forever(DeleteForeverFolder::Trash, vec![bare], None, 50);

        assert_eq!(
            plan.batches,
            vec![vec![DeleteForeverTarget {
                id: "t-1".to_string(),
                message_id_header: None
            }]]
        );
    }

    // Reports reach the webview: library and OS detail (SQLite messages,
    // file paths) stays out of them, as at the command boundary.
    #[test]
    fn an_internal_error_reaches_the_report_without_its_detail() {
        let error = AppError::IoError("/private/path/to/emailops.db is locked".to_string());

        let carried = DeleteForeverError::from(&error);

        assert_eq!(carried.code, "io");
        assert_eq!(carried.message, "File error");
        assert!(!carried.params.contains_key("detail"), "got {:?}", carried.params);
    }

    // ── executor ──────────────────────────────────────────────────────────

    use crate::models::Email;
    use crate::sync::provider::{FakeEmailProvider, FakeMailboxOp};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    fn mail(id: &str, mailbox: &str) -> Email {
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
            timestamp: 1_000,
            is_read: true,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: mailbox.to_string(),
            is_sent: false,
            is_starred: false,
            headers: None,
        }
    }

    fn db_with(emails: &[Email]) -> Database {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(emails).unwrap();
        db
    }

    fn stored(db: &Database, id: &str) -> bool {
        db.email_exists(id).unwrap()
    }

    fn trash_plan(db: &Database, batch_size: usize) -> DeleteForeverPlan {
        let rows = db
            .delete_forever_rows("acc-1", DeleteForeverFolder::Trash, None)
            .unwrap();
        plan_delete_forever(DeleteForeverFolder::Trash, rows, None, batch_size)
    }

    fn never() -> impl Fn() -> bool + Send + Sync {
        || false
    }

    fn ignore() -> impl Fn(DeleteForeverProgress) + Send + Sync {
        |_| {}
    }

    #[tokio::test]
    async fn messages_the_provider_deleted_are_removed_here_and_counted() {
        let db = db_with(&[mail("t-1", "trash"), mail("t-2", "trash"), mail("in-1", "inbox")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        let plan = trash_plan(&db, 50);

        let report = execute_delete_forever(
            &db,
            Some(&provider),
            DeleteForeverFolder::Trash,
            plan,
            &never(),
            &ignore(),
        )
        .await;

        assert_eq!((report.total, report.removed, report.not_attempted), (2, 2, 0));
        assert!(report.failed.is_empty() && report.error.is_none() && !report.cancelled);
        assert!(!stored(&db, "t-1") && !stored(&db, "t-2"), "hard-deleted, not soft");
        assert!(stored(&db, "in-1"), "another folder is untouched");
        let mut asked: Vec<FakeMailboxOp> = provider.mailbox_ops();
        asked.sort_by_key(|op| format!("{op:?}"));
        assert_eq!(
            asked,
            vec![
                FakeMailboxOp::DeleteForever {
                    folder: DeleteForeverFolder::Trash,
                    message_id: "t-1".to_string()
                },
                FakeMailboxOp::DeleteForever {
                    folder: DeleteForeverFolder::Trash,
                    message_id: "t-2".to_string()
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_message_the_provider_kept_stays_and_is_reported() {
        let db = db_with(&[mail("t-1", "trash"), mail("t-2", "trash")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.refuse_delete_forever("t-2");
        let plan = trash_plan(&db, 50);

        let report = execute_delete_forever(
            &db,
            Some(&provider),
            DeleteForeverFolder::Trash,
            plan,
            &never(),
            &ignore(),
        )
        .await;

        assert_eq!(report.removed, 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].email_id, "t-2");
        assert_eq!(report.failed[0].error.code, "sync");
        assert!(stored(&db, "t-2"), "the row stays until the provider confirms");
        assert!(!stored(&db, "t-1"));
    }

    #[tokio::test]
    async fn a_message_the_provider_no_longer_has_counts_as_removed() {
        let db = db_with(&[mail("t-1", "trash")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes_as_not_found();
        let plan = trash_plan(&db, 50);

        let report = execute_delete_forever(
            &db,
            Some(&provider),
            DeleteForeverFolder::Trash,
            plan,
            &never(),
            &ignore(),
        )
        .await;

        assert_eq!(report.removed, 1);
        assert!(report.failed.is_empty());
        assert!(!stored(&db, "t-1"));
    }

    #[tokio::test]
    async fn a_batch_the_provider_could_not_answer_stops_the_run_and_removes_nothing_of_it() {
        let db = db_with(&[mail("t-1", "trash"), mail("t-2", "trash"), mail("t-3", "trash")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.fail_mailbox_writes("offline");
        let plan = trash_plan(&db, 2);

        let report = execute_delete_forever(
            &db,
            Some(&provider),
            DeleteForeverFolder::Trash,
            plan,
            &never(),
            &ignore(),
        )
        .await;

        assert_eq!((report.removed, report.not_attempted), (0, 3));
        assert_eq!(report.error.as_ref().map(|e| e.code.as_str()), Some("sync"));
        assert_eq!(
            provider.calls().iter().filter(|c| *c == "delete_forever").count(),
            1,
            "stopped"
        );
        for id in ["t-1", "t-2", "t-3"] {
            assert!(stored(&db, id), "{id}");
        }
    }

    #[tokio::test]
    async fn cancel_stops_between_batches_and_leaves_the_rest_in_place() {
        let db = db_with(&[mail("t-1", "trash"), mail("t-2", "trash"), mail("t-3", "trash")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        let plan = trash_plan(&db, 2);
        let checks = AtomicUsize::new(0);
        // Cancel pressed while the first batch was out.
        let cancelled = move || checks.fetch_add(1, Ordering::SeqCst) >= 1;

        let report = execute_delete_forever(
            &db,
            Some(&provider),
            DeleteForeverFolder::Trash,
            plan,
            &cancelled,
            &ignore(),
        )
        .await;

        assert!(report.cancelled);
        assert_eq!((report.removed, report.not_attempted), (2, 1));
        assert!(report.error.is_none());
        assert_eq!(provider.mailbox_ops().len(), 2, "the second batch never went out");
        assert_eq!(["t-1", "t-2", "t-3"].iter().filter(|id| stored(&db, id)).count(), 1);
    }

    #[tokio::test]
    async fn rows_deleted_here_but_filed_elsewhere_are_removed_without_asking_the_provider() {
        let db = db_with(&[mail("gone-1", "sent")]);
        db.delete_email("gone-1").unwrap();
        let plan = trash_plan(&db, 50);

        let report = execute_delete_forever(&db, None, DeleteForeverFolder::Trash, plan, &never(), &ignore()).await;

        assert_eq!(report.removed, 1);
        assert!(!stored(&db, "gone-1"));
    }

    #[tokio::test]
    async fn selected_messages_outside_the_folder_stay_and_are_reported() {
        let db = db_with(&[mail("in-1", "inbox")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        let requested = ids(&["in-1"]);
        let rows = db
            .delete_forever_rows("acc-1", DeleteForeverFolder::Trash, Some(&requested))
            .unwrap();
        let plan = plan_delete_forever(DeleteForeverFolder::Trash, rows, Some(&requested), 50);

        let report = execute_delete_forever(
            &db,
            Some(&provider),
            DeleteForeverFolder::Trash,
            plan,
            &never(),
            &ignore(),
        )
        .await;

        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].email_id, "in-1");
        assert!(stored(&db, "in-1"));
        assert!(provider.mailbox_ops().is_empty(), "the provider is never asked");
    }

    #[tokio::test]
    async fn progress_is_reported_after_every_batch() {
        let db = db_with(&[mail("t-1", "trash"), mail("t-2", "trash"), mail("t-3", "trash")]);
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        let plan = trash_plan(&db, 2);
        let seen = Mutex::new(Vec::new());
        let record = |p: DeleteForeverProgress| seen.lock().unwrap().push((p.processed, p.total));

        execute_delete_forever(
            &db,
            Some(&provider),
            DeleteForeverFolder::Trash,
            plan,
            &never(),
            &record,
        )
        .await;

        assert_eq!(*seen.lock().unwrap(), vec![(0, 3), (2, 3), (3, 3)]);
    }

    // ── entry point ───────────────────────────────────────────────────────

    #[test]
    fn a_gmail_account_is_refused_before_anything_is_read() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account_with_provider("acc-g", "gmail");

        let result = prepare_delete_forever(&db, "acc-g", DeleteForeverFolder::Trash, None);

        assert!(
            matches!(result, Err(AppError::InvalidInput(_))),
            "got {:?}",
            result.map(|(_, p)| p)
        );
    }

    #[test]
    fn an_unknown_account_is_not_found() {
        let db = Database::new_for_testing().unwrap();

        let result = prepare_delete_forever(&db, "nope", DeleteForeverFolder::Spam, None);

        assert!(
            matches!(result, Err(AppError::NotFound(_))),
            "got {:?}",
            result.map(|(_, p)| p)
        );
    }

    #[test]
    fn an_imap_account_gets_the_plan_for_its_folder() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account_with_provider("acc-1", "imap");
        db.insert_emails_batch(&[mail("s-1", "spam"), mail("t-1", "trash")])
            .unwrap();

        let (account, plan) = prepare_delete_forever(&db, "acc-1", DeleteForeverFolder::Spam, None).unwrap();

        assert_eq!(account.id, "acc-1");
        assert_eq!(plan.batches, vec![vec![target("s-1")]]);
    }

    fn run_blocking(db: &Arc<Database>, account_id: &str, folder: DeleteForeverFolder) -> DeleteForeverReport {
        let runs = Arc::new(DeleteForeverRuns::default());
        let ticket = runs.begin("run-1");
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build test runtime")
            .block_on(run_delete_forever(db, account_id, folder, None, "run-1", None, &ticket))
    }

    // Sync `#[test]`s on their own runtime so the seam lock is never held
    // across an await point (`clippy::await_holding_lock`).
    #[test]
    fn a_refused_run_still_ends_with_one_finished_event_and_an_error_line() {
        let _seam = crate::services::events::seam_test_lock();
        let sink = crate::services::events::install_for_testing();
        let logs = crate::services::logger::install_for_testing();
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account_with_provider("acc-g", "gmail");

        let report = run_blocking(&db, "acc-g", DeleteForeverFolder::Trash);

        let finished = sink.payloads_for(FINISHED_EVENT);
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0]["runId"], "run-1");
        assert_eq!(finished[0]["accountId"], "acc-g");
        assert_eq!(finished[0]["folder"], "trash");
        assert_eq!(finished[0]["error"]["code"], "invalid_input");
        assert_eq!(report.removed, 0);
        assert!(logs
            .events()
            .iter()
            .any(|e| e.level == "error" && e.message.contains("delete forever")));
    }

    #[test]
    fn a_run_reports_progress_then_finishes_with_its_counts() {
        let _seam = crate::services::events::seam_test_lock();
        let sink = crate::services::events::install_for_testing();
        let logs = crate::services::logger::install_for_testing();
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account_with_provider("acc-1", "imap");
        // Deleted here, filed elsewhere: removed without a provider.
        db.insert_emails_batch(&[mail("gone-1", "inbox")]).unwrap();
        db.delete_email("gone-1").unwrap();

        let report = run_blocking(&db, "acc-1", DeleteForeverFolder::Trash);

        assert_eq!(report.removed, 1);
        let progress = sink.payloads_for(PROGRESS_EVENT);
        assert_eq!(
            progress.last().map(|p| p["processed"].clone()),
            Some(serde_json::json!(1))
        );
        assert_eq!(progress[0]["runId"], "run-1");
        let finished = sink.payloads_for(FINISHED_EVENT);
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0]["removed"], 1);
        assert_eq!(finished[0]["notAttempted"], 0);
        assert!(finished[0]["error"].is_null());
        let lines: Vec<_> = logs
            .events()
            .into_iter()
            .filter(|e| e.message.contains("acc-1"))
            .map(|e| e.level)
            .collect();
        assert_eq!(lines, vec!["info".to_string(), "success".to_string()]);
    }

    #[test]
    fn cancel_reaches_the_registered_run_only() {
        let runs = Arc::new(DeleteForeverRuns::default());
        let first = runs.begin("run-1");
        let second = runs.begin("run-2");

        runs.cancel("run-1");
        runs.cancel("run-unknown");

        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
    }

    #[test]
    fn a_finished_run_is_unregistered() {
        let runs = Arc::new(DeleteForeverRuns::default());
        drop(runs.begin("run-1"));

        let again = runs.begin("run-1");
        assert!(!again.is_cancelled());
        drop(again);

        assert!(runs.running.lock().unwrap().is_empty());
    }
}
