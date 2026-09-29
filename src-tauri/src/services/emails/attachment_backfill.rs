//! One-time backfill of `email_attachment_meta` for mail stored without it.
//!
//! Until the batch insert tolerated a repeated filename (two `image001.png`
//! in one newsletter), one such email failed the insert for its whole sync
//! chunk and the error was discarded — whole months of mail ended up with no
//! attachment rows, invisible to the attachments view, rules and rule
//! suggestions. Incremental sync never revisits stored mail, so the gap is
//! permanent without this pass.
//!
//! The provider names the messages that carry attachments (a cheap
//! server-side search); only those that have no rows locally are fetched.
//! It runs at the end of a sync until one pass fetches everything it needs,
//! which is then remembered in `user_preferences`.

use std::collections::HashSet;

use crate::db::Database;
use crate::models::error::Result;
use crate::sync::provider::EmailProvider;

const BACKFILL_DONE_PREFIX: &str = "attachment_meta_backfill_done:";
const FETCH_CHUNK: usize = 50;

pub fn backfill_done_key(account_id: &str) -> String {
    format!("{BACKFILL_DONE_PREFIX}{account_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillOutcome {
    /// Already ran for this account.
    AlreadyDone,
    /// The provider cannot list messages with attachments.
    Unsupported,
    /// The sync was cancelled between chunks; the next sync resumes.
    Aborted { recovered: usize },
    /// Every listed message was visited. Only marked done when `failed` is 0;
    /// otherwise the next sync retries the messages that still lack rows.
    Completed {
        /// Messages fetched from the provider.
        fetched: usize,
        /// Of those, messages whose attachment rows were recovered.
        recovered: usize,
        /// Messages the provider failed to return.
        failed: usize,
    },
}

/// Pure planner: the provider's messages-with-attachments that are stored
/// locally without a single attachment row, deduplicated, in provider order.
pub fn plan_backfill_targets(with_attachments: &[String], missing_meta: &HashSet<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    with_attachments
        .iter()
        .filter(|id| missing_meta.contains(*id) && seen.insert(id.as_str()))
        .cloned()
        .collect()
}

pub async fn backfill_attachment_meta(
    db: &Database,
    provider: &dyn EmailProvider,
    account_id: &str,
    account_email: &str,
    should_abort: &(dyn Fn() -> bool + Send + Sync),
) -> Result<BackfillOutcome> {
    let done_key = backfill_done_key(account_id);
    if db.get_preference(&done_key)?.is_some() {
        return Ok(BackfillOutcome::AlreadyDone);
    }
    let Some(with_attachments) = provider.list_message_ids_with_attachments().await? else {
        return Ok(BackfillOutcome::Unsupported);
    };
    let missing = db.get_email_ids_without_attachment_meta(account_id)?;
    let targets = plan_backfill_targets(&with_attachments, &missing);

    let (mut recovered, mut failed) = (0, 0);
    for chunk in targets.chunks(FETCH_CHUNK) {
        if should_abort() {
            return Ok(BackfillOutcome::Aborted { recovered });
        }
        let chunk_ids: Vec<&str> = chunk.iter().map(String::as_str).collect();
        for (id, result) in chunk_ids.iter().zip(provider.batch_get_messages(&chunk_ids).await?) {
            match result {
                Ok((_, _, infos)) if !infos.is_empty() => {
                    db.insert_attachment_infos(id, account_id, &infos)?;
                    recovered += 1;
                }
                Ok(_) => {}
                Err(e) => {
                    super::events::emit_account_log(
                        "warn",
                        "sync",
                        account_email,
                        &format!("Attachment backfill could not fetch {id}: {e}"),
                    );
                    failed += 1;
                }
            }
        }
    }

    // A message the provider failed to return (rate limit, network) stays
    // without rows; marking the account done would hide it from rules for
    // good, so the next sync retries — only the still-missing ones.
    if failed == 0 {
        db.set_preference(&done_key, &super::super::clock::now_secs().to_string())?;
    }
    Ok(BackfillOutcome::Completed {
        fetched: targets.len(),
        recovered,
        failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Email;
    use crate::sync::provider::{AttachmentInfo, EmailCategory, FakeEmailProvider};

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn setup() -> Database {
        let db = Database::new_for_testing().expect("db");
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES ('acc', 'gmail', 'me@example.com', 'Me', 0, 0, 1)",
                [],
            )
            .expect("account");
        db
    }

    fn email(id: &str) -> Email {
        Email {
            id: id.into(),
            account_id: "acc".into(),
            thread_id: id.into(),
            message_id: None,
            references: None,
            subject: "Invoice".into(),
            sender: "Billing".into(),
            sender_email: "billing@example.com".into(),
            recipients: vec!["me@example.com".into()],
            cc: vec![],
            body: "body".into(),
            snippet: "snippet".into(),
            timestamp: 1_750_000_000,
            is_read: true,
            triage_status: None,
            category: "updates".into(),
            mailbox: "inbox".into(),
            is_sent: false,
            headers: None,
        }
    }

    fn pdf(name: &str) -> AttachmentInfo {
        AttachmentInfo {
            attachment_id: format!("att-{name}"),
            filename: name.into(),
            mime_type: "application/pdf".into(),
            size: 10,
            inline_data: None,
        }
    }

    /// `id` stored locally without attachment rows; the provider has it with one PDF.
    fn stored_without_meta(db: &Database, provider: &FakeEmailProvider, id: &str) {
        db.insert_email(&email(id)).expect("insert");
        provider.add_message(email(id), EmailCategory::Updates, vec![pdf(&format!("{id}.pdf"))]);
    }

    fn never() -> bool {
        false
    }

    #[test]
    fn plan_keeps_only_listed_messages_that_lack_rows_locally() {
        let missing: HashSet<String> = ids(&["a", "c"]).into_iter().collect();
        assert_eq!(
            plan_backfill_targets(&ids(&["a", "b", "c", "a"]), &missing),
            ids(&["a", "c"])
        );
    }

    #[tokio::test]
    async fn recovers_attachment_rows_for_stored_mail_that_lost_them() {
        let db = setup();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        stored_without_meta(&db, &provider, "m1");

        let outcome = backfill_attachment_meta(&db, &provider, "acc", "me@example.com", &never)
            .await
            .expect("backfill");

        assert_eq!(
            outcome,
            BackfillOutcome::Completed {
                fetched: 1,
                recovered: 1,
                failed: 0
            }
        );
        assert_eq!(
            db.get_email_attachment_metas("m1").expect("metas")[0].filename,
            "m1.pdf"
        );
    }

    #[tokio::test]
    async fn mail_that_already_has_attachment_rows_is_not_fetched_again() {
        let db = setup();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        stored_without_meta(&db, &provider, "m1");
        db.insert_attachment_infos("m1", "acc", &[pdf("m1.pdf")]).expect("meta");

        let outcome = backfill_attachment_meta(&db, &provider, "acc", "me@example.com", &never)
            .await
            .expect("backfill");

        assert_eq!(
            outcome,
            BackfillOutcome::Completed {
                fetched: 0,
                recovered: 0,
                failed: 0
            }
        );
        assert!(!provider.calls().iter().any(|c| c == "batch_get_messages"));
    }

    #[tokio::test]
    async fn listed_messages_not_stored_locally_are_ignored() {
        let db = setup();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.add_message(email("remote-only"), EmailCategory::Updates, vec![pdf("x.pdf")]);

        let outcome = backfill_attachment_meta(&db, &provider, "acc", "me@example.com", &never)
            .await
            .expect("backfill");

        assert_eq!(
            outcome,
            BackfillOutcome::Completed {
                fetched: 0,
                recovered: 0,
                failed: 0
            }
        );
        assert!(db.get_email("remote-only").expect("get").is_none());
    }

    #[tokio::test]
    async fn a_completed_backfill_is_remembered_and_not_repeated() {
        let db = setup();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        stored_without_meta(&db, &provider, "m1");
        backfill_attachment_meta(&db, &provider, "acc", "me@example.com", &never)
            .await
            .expect("first");

        let again = FakeEmailProvider::new("me@example.com", "Me");
        let outcome = backfill_attachment_meta(&db, &again, "acc", "me@example.com", &never)
            .await
            .expect("second");

        assert_eq!(outcome, BackfillOutcome::AlreadyDone);
        assert!(
            again.calls().is_empty(),
            "no provider call once done, got {:?}",
            again.calls()
        );
    }

    #[tokio::test]
    async fn a_backfill_with_failed_fetches_is_not_marked_done_and_retries_only_those() {
        let db = setup();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        stored_without_meta(&db, &provider, "m1");
        stored_without_meta(&db, &provider, "m2");
        provider.fail_message("m2");

        let outcome = backfill_attachment_meta(&db, &provider, "acc", "me@example.com", &never)
            .await
            .expect("backfill");

        assert_eq!(
            outcome,
            BackfillOutcome::Completed {
                fetched: 2,
                recovered: 1,
                failed: 1
            }
        );
        assert!(
            db.get_preference(&backfill_done_key("acc")).expect("pref").is_none(),
            "a failed fetch must be retried by the next sync"
        );

        let retry = FakeEmailProvider::new("me@example.com", "Me");
        stored_without_meta(&db, &retry, "m1");
        retry.add_message(email("m2"), EmailCategory::Updates, vec![pdf("m2.pdf")]);
        let outcome = backfill_attachment_meta(&db, &retry, "acc", "me@example.com", &never)
            .await
            .expect("retry");

        assert_eq!(
            outcome,
            BackfillOutcome::Completed {
                fetched: 1,
                recovered: 1,
                failed: 0
            }
        );
        assert!(db.get_preference(&backfill_done_key("acc")).expect("pref").is_some());
    }

    #[tokio::test]
    async fn a_provider_without_attachment_search_is_left_unmarked() {
        let db = setup();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.set_attachment_listing(None);
        stored_without_meta(&db, &provider, "m1");

        let outcome = backfill_attachment_meta(&db, &provider, "acc", "me@example.com", &never)
            .await
            .expect("backfill");

        assert_eq!(outcome, BackfillOutcome::Unsupported);
        assert!(db.get_preference(&backfill_done_key("acc")).expect("pref").is_none());
    }

    #[tokio::test]
    async fn a_listed_message_that_comes_back_without_attachments_is_not_counted_recovered() {
        let db = setup();
        db.insert_email(&email("m1")).expect("insert");
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        provider.add_message(email("m1"), EmailCategory::Updates, vec![]);
        provider.set_attachment_listing(Some(vec!["m1".into()]));

        let outcome = backfill_attachment_meta(&db, &provider, "acc", "me@example.com", &never)
            .await
            .expect("backfill");

        assert_eq!(
            outcome,
            BackfillOutcome::Completed {
                fetched: 1,
                recovered: 0,
                failed: 0
            }
        );
    }

    #[tokio::test]
    async fn an_aborted_backfill_is_not_marked_done() {
        let db = setup();
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        stored_without_meta(&db, &provider, "m1");

        let outcome = backfill_attachment_meta(&db, &provider, "acc", "me@example.com", &|| true)
            .await
            .expect("backfill");

        assert_eq!(outcome, BackfillOutcome::Aborted { recovered: 0 });
        assert!(db.get_preference(&backfill_done_key("acc")).expect("pref").is_none());
    }
}
