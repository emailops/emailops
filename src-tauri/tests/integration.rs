// Integration tests for the DB and service layers.
//
// Each test gets an isolated `Database::new_for_testing()` (in-memory SQLite
// with the full production schema). Tests never share state — no global mutexes,
// no shared DBs — so they can run fully in parallel.
//
// Run all:
//   cargo test --manifest-path src-tauri/Cargo.toml
// Fast iteration (skip llama-cpp build):
//   cargo test --manifest-path src-tauri/Cargo.toml --no-default-features

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use emailops_lib::ai::provider::FakeAiProvider;
use emailops_lib::db::Database;
use emailops_lib::models::error::AppError;
use emailops_lib::models::lens::{CreateLensInput, LensColumn, LensColumnType, LensSchema, LensScope};
use emailops_lib::models::{Account, Email, SaveDraftRequest};
use emailops_lib::services::background_tasks::{BackgroundTask, FakeDispatcher, TaskDispatcher};
use emailops_lib::services::task_queue::TaskQueue;
use emailops_lib::sync::provider::{
    AttachmentInfo, EmailAttachment, EmailCategory, EmailProvider, ExtraMailbox, FakeEmailProvider, MessageRef,
};

mod common;

// ── helpers ────────────────────────────────────────────────────────────────

fn test_db() -> Arc<Database> {
    Arc::new(Database::new_for_testing().expect("in-memory test DB"))
}

fn make_account(id: &str, email: &str) -> Account {
    Account {
        id: id.to_string(),
        provider: "gmail".to_string(),
        email: email.to_string(),
        name: "Test User".to_string(),
        created_at: 1_000_000,
        sort_order: 0,
        enabled: true,
        sync_from_timestamp: None,
    }
}

fn make_email(id: &str, account_id: &str, timestamp: i64) -> Email {
    Email {
        id: id.to_string(),
        account_id: account_id.to_string(),
        thread_id: format!("thread-{id}"),
        message_id: None,
        subject: format!("Subject {id}"),
        sender: "Test Sender".to_string(),
        sender_email: "sender@example.com".to_string(),
        recipients: vec!["recipient@example.com".to_string()],
        cc: vec![],
        body: format!("Body of email {id}"),
        snippet: format!("Snippet {id}"),
        timestamp,
        is_read: false,
        triage_status: None,
        category: "primary".to_string(),
        mailbox: "inbox".to_string(),
    }
}

fn make_lens_input(name: &str) -> CreateLensInput {
    CreateLensInput {
        name: name.to_string(),
        icon: None,
        template_key: None,
        account_id: None,
        scope: LensScope::default(),
        schema: LensSchema {
            columns: vec![LensColumn {
                key: "amount".to_string(),
                label: "Amount".to_string(),
                column_type: LensColumnType::Number,
                description: "Invoice amount".to_string(),
                enum_values: None,
                required: false,
                is_unique_key: false,
            }],
        },
        prompt_text: "Extract the invoice amount.".to_string(),
        model_provider: None,
        model_name: None,
    }
}

// ── accounts ───────────────────────────────────────────────────────────────

#[test]
fn list_accounts_empty_db() {
    let db = test_db();
    let accounts = db.list_accounts().expect("list_accounts");
    assert!(accounts.is_empty());
}

#[test]
fn insert_and_list_account() {
    let db = test_db();
    db.insert_account(&make_account("acc-1", "alice@example.com"))
        .expect("insert");
    let list = db.list_accounts().expect("list_accounts");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].email, "alice@example.com");
    assert_eq!(list[0].id, "acc-1");
}

#[test]
fn insert_two_accounts_and_list_both() {
    let db = test_db();
    db.insert_account(&make_account("a1", "a@example.com"))
        .expect("insert a1");
    db.insert_account(&make_account("a2", "b@example.com"))
        .expect("insert a2");
    let list = db.list_accounts().expect("list");
    assert_eq!(list.len(), 2);
    let ids: Vec<&str> = list.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"a1"));
    assert!(ids.contains(&"a2"));
}

#[test]
fn delete_account_removes_it() {
    let db = test_db();
    db.insert_account(&make_account("acc-del", "del@example.com"))
        .expect("insert");
    db.delete_account("acc-del").expect("delete_account");
    let list = db.list_accounts().expect("list");
    assert!(list.is_empty());
}

#[test]
fn account_exists_by_email_false_when_absent() {
    let db = test_db();
    assert!(!db.account_exists_by_email("missing@example.com").expect("check"));
}

#[test]
fn account_exists_by_email_true_after_insert() {
    let db = test_db();
    db.insert_account(&make_account("acc-chk", "chk@example.com"))
        .expect("insert");
    assert!(db.account_exists_by_email("chk@example.com").expect("check"));
}

#[test]
fn get_account_returns_correct_record() {
    let db = test_db();
    let acc = make_account("acc-get", "get@example.com");
    db.insert_account(&acc).expect("insert");
    let fetched = db.get_account("acc-get").expect("get").expect("Some");
    assert_eq!(fetched.email, "get@example.com");
    assert_eq!(fetched.provider, "gmail");
    assert!(fetched.enabled);
}

#[test]
fn get_account_missing_returns_none() {
    let db = test_db();
    let result = db.get_account("no-such-id").expect("get");
    assert!(result.is_none());
}

// ── preferences ────────────────────────────────────────────────────────────

#[test]
fn preference_missing_returns_none() {
    let db = test_db();
    let val = db.get_preference("no_such_key").expect("get");
    assert!(val.is_none());
}

#[test]
fn set_and_get_preference_roundtrips() {
    let db = test_db();
    db.set_preference("theme", "dark").expect("set");
    let val = db.get_preference("theme").expect("get").expect("Some");
    assert_eq!(val, "dark");
}

#[test]
fn set_preference_twice_keeps_last_value() {
    let db = test_db();
    db.set_preference("k", "first").expect("set 1");
    db.set_preference("k", "second").expect("set 2");
    let val = db.get_preference("k").expect("get").expect("Some");
    assert_eq!(val, "second");
}

#[test]
fn multiple_independent_preferences() {
    let db = test_db();
    db.set_preference("theme", "dark").expect("set theme");
    db.set_preference("locale", "en-US").expect("set locale");
    assert_eq!(db.get_preference("theme").unwrap().unwrap(), "dark");
    assert_eq!(db.get_preference("locale").unwrap().unwrap(), "en-US");
}

// ── lenses ─────────────────────────────────────────────────────────────────

#[test]
fn list_lenses_empty_db() {
    let db = test_db();
    assert!(db.list_lenses().expect("list").is_empty());
}

#[test]
fn create_lens_returns_correct_fields() {
    let db = test_db();
    let lens = db.create_lens(&make_lens_input("Invoice Tracker")).expect("create");
    assert_eq!(lens.name, "Invoice Tracker");
    assert!(!lens.id.is_empty());
    assert!(lens.is_enabled);
    assert_eq!(lens.schema.columns.len(), 1);
    assert_eq!(lens.schema.columns[0].key, "amount");
}

#[test]
fn create_and_list_lens() {
    let db = test_db();
    db.create_lens(&make_lens_input("Payments")).expect("create");
    let list = db.list_lenses().expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Payments");
}

#[test]
fn get_lens_returns_full_record() {
    let db = test_db();
    let created = db.create_lens(&make_lens_input("Payment Lens")).expect("create");
    let fetched = db.get_lens(&created.id).expect("get");
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "Payment Lens");
    assert_eq!(fetched.schema.columns.len(), 1);
}

#[test]
fn delete_lens_removes_from_list() {
    let db = test_db();
    let lens = db.create_lens(&make_lens_input("Temp")).expect("create");
    db.delete_lens(&lens.id).expect("delete");
    assert!(db.list_lenses().expect("list").is_empty());
}

#[test]
fn multiple_lenses_all_returned() {
    let db = test_db();
    db.create_lens(&make_lens_input("Alpha")).expect("create 1");
    db.create_lens(&make_lens_input("Beta")).expect("create 2");
    db.create_lens(&make_lens_input("Gamma")).expect("create 3");
    let list = db.list_lenses().expect("list");
    assert_eq!(list.len(), 3);
    let names: Vec<&str> = list.iter().map(|l| l.name.as_str()).collect();
    assert!(names.contains(&"Alpha"));
    assert!(names.contains(&"Beta"));
    assert!(names.contains(&"Gamma"));
}

#[test]
fn lens_row_count_starts_at_zero() {
    let db = test_db();
    let lens = db.create_lens(&make_lens_input("Empty Lens")).expect("create");
    let summaries = db.list_lenses().expect("list");
    let summary = summaries.iter().find(|s| s.id == lens.id).expect("found");
    assert_eq!(summary.row_count, 0);
}

// ── emails ─────────────────────────────────────────────────────────────────

#[test]
fn insert_email_and_get_by_id() {
    let db = test_db();
    db.insert_account(&make_account("acc-1", "a@example.com"))
        .expect("account");
    let email = make_email("email-1", "acc-1", 1_000_000);
    db.insert_email(&email).expect("insert");
    let fetched = db.get_email_by_id("email-1").expect("get").expect("Some");
    assert_eq!(fetched.id, "email-1");
    assert_eq!(fetched.subject, "Subject email-1");
    assert_eq!(fetched.account_id, "acc-1");
    assert!(!fetched.is_read);
}

#[test]
fn get_email_by_id_missing_returns_none() {
    let db = test_db();
    assert!(db.get_email_by_id("nonexistent").expect("ok").is_none());
}

#[test]
fn get_emails_returns_correct_account_only() {
    let db = test_db();
    db.insert_account(&make_account("acc-A", "a@example.com"))
        .expect("acc-A");
    db.insert_account(&make_account("acc-B", "b@example.com"))
        .expect("acc-B");
    db.insert_email(&make_email("e-1", "acc-A", 1000)).expect("insert e-1");
    db.insert_email(&make_email("e-2", "acc-A", 900)).expect("insert e-2");
    db.insert_email(&make_email("e-3", "acc-B", 800)).expect("insert e-3");

    let emails = db.get_emails("acc-A", 50, 0, None, None, None).expect("get");
    assert_eq!(emails.len(), 2);
    let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"e-1") && ids.contains(&"e-2"));
    assert!(!ids.contains(&"e-3"));
}

#[test]
fn get_emails_empty_for_unknown_account() {
    let db = test_db();
    // No emails for "acc-other" — no account needed, just expect empty.
    let result = db.get_emails("acc-other", 50, 0, None, None, None).expect("get");
    assert!(result.is_empty());
}

#[test]
fn get_emails_respects_limit() {
    let db = test_db();
    db.insert_account(&make_account("acc-lim", "lim@example.com"))
        .expect("account");
    for i in 0..10_i64 {
        db.insert_email(&make_email(&format!("e-{i}"), "acc-lim", (10 - i) * 100))
            .expect("insert");
    }
    let emails = db.get_emails("acc-lim", 3, 0, None, None, None).expect("get");
    assert_eq!(emails.len(), 3);
}

#[test]
fn get_emails_ordered_newest_first() {
    let db = test_db();
    db.insert_account(&make_account("acc-ord", "ord@example.com"))
        .expect("account");
    db.insert_email(&make_email("e-old", "acc-ord", 1000))
        .expect("insert old");
    db.insert_email(&make_email("e-new", "acc-ord", 9000))
        .expect("insert new");
    let emails = db.get_emails("acc-ord", 50, 0, None, None, None).expect("get");
    assert_eq!(emails.len(), 2);
    assert_eq!(emails[0].id, "e-new", "newest first");
    assert_eq!(emails[1].id, "e-old");
}

#[test]
fn insert_email_replaces_on_conflict() {
    let db = test_db();
    db.insert_account(&make_account("acc-dup", "dup@example.com"))
        .expect("account");
    let mut email = make_email("e-dup", "acc-dup", 1000);
    db.insert_email(&email).expect("insert first");
    email.subject = "Updated Subject".to_string();
    db.insert_email(&email).expect("insert again (replace)");

    let fetched = db.get_email_by_id("e-dup").expect("get").expect("Some");
    assert_eq!(fetched.subject, "Updated Subject");
}

// ── sync status ────────────────────────────────────────────────────────────

#[test]
fn sync_status_defaults_to_idle() {
    let db = test_db();
    let status = db.get_sync_status("acc-new").expect("get");
    assert_eq!(status.status, "idle");
    assert!(status.last_sync_at.is_none());
    assert!(status.error.is_none());
}

#[test]
fn upsert_sync_status_persists() {
    let db = test_db();
    db.insert_account(&make_account("acc-s", "s@example.com"))
        .expect("account");
    db.upsert_sync_status("acc-s", "syncing", None, None).expect("upsert");
    let s = db.get_sync_status("acc-s").expect("get");
    assert_eq!(s.status, "syncing");
    assert_eq!(s.account_id, "acc-s");
}

#[test]
fn sync_status_error_message_stored() {
    let db = test_db();
    db.insert_account(&make_account("acc-err", "err@example.com"))
        .expect("account");
    db.upsert_sync_status("acc-err", "error", None, Some("auth failed"))
        .expect("upsert");
    let s = db.get_sync_status("acc-err").expect("get");
    assert_eq!(s.status, "error");
    assert_eq!(s.error.as_deref(), Some("auth failed"));
}

#[test]
fn sync_status_transition_idle_to_syncing_to_idle() {
    let db = test_db();
    db.insert_account(&make_account("acc-t", "t@example.com"))
        .expect("account");
    let ts = 1_700_000_000_i64;
    db.upsert_sync_status("acc-t", "syncing", None, None).expect("syncing");
    db.upsert_sync_status("acc-t", "idle", Some(ts), None).expect("idle");
    let s = db.get_sync_status("acc-t").expect("get");
    assert_eq!(s.status, "idle");
    assert_eq!(s.last_sync_at, Some(ts));
    assert!(s.error.is_none());
}

// ── background task dispatcher ─────────────────────────────────────────────

#[tokio::test]
async fn fake_dispatcher_starts_empty() {
    let d = FakeDispatcher::new();
    assert!(d.recorded().is_empty());
}

#[tokio::test]
async fn fake_dispatcher_records_task_without_running_future() {
    let d = FakeDispatcher::new();

    d.dispatch(
        BackgroundTask::GenerateDraft {
            email_id: "e-draft".into(),
            request_id: "r-1".into(),
        },
        Box::new(|| Box::pin(async { panic!("must not run") })),
    )
    .await;

    let recorded = d.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0],
        BackgroundTask::GenerateDraft {
            email_id: "e-draft".into(),
            request_id: "r-1".into()
        }
    );
}

#[tokio::test]
async fn fake_dispatcher_preserves_order() {
    let d = FakeDispatcher::new();

    d.dispatch(
        BackgroundTask::ClassifyEmails {
            account_id: "acc-1".into(),
            request_id: "r1".into(),
        },
        Box::new(|| Box::pin(async {})),
    )
    .await;

    d.dispatch(
        BackgroundTask::RunLens { lens_id: 3, run_id: 7 },
        Box::new(|| Box::pin(async {})),
    )
    .await;

    d.dispatch(
        BackgroundTask::SyncAccount {
            account_id: "acc-2".into(),
            request_id: "r3".into(),
        },
        Box::new(|| Box::pin(async {})),
    )
    .await;

    let recorded = d.recorded();
    assert_eq!(recorded.len(), 3);
    assert!(matches!(recorded[0], BackgroundTask::ClassifyEmails { .. }));
    assert!(matches!(recorded[1], BackgroundTask::RunLens { lens_id: 3, run_id: 7 }));
    assert!(matches!(recorded[2], BackgroundTask::SyncAccount { .. }));
}

#[tokio::test]
async fn fake_dispatcher_recorded_returns_clone_not_drain() {
    let d = FakeDispatcher::new();
    d.dispatch(
        BackgroundTask::DownloadModel {
            model_id: "gemma-2b".into(),
        },
        Box::new(|| Box::pin(async {})),
    )
    .await;

    // Calling recorded() twice must return the same content both times.
    let first = d.recorded();
    let second = d.recorded();
    assert_eq!(first, second);
}

// ── lens run lifecycle ──────────────────────────────────────────────────────

#[test]
fn lens_run_insert_and_finish() {
    use emailops_lib::models::lens::LensRunKind;

    let db = test_db();
    let lens = db.create_lens(&make_lens_input("Run Test")).expect("create");

    let run_id = db
        .insert_lens_run(&lens.id, LensRunKind::Backfill, 42)
        .expect("insert_run");
    assert!(!run_id.is_empty());

    // LensRunProgress = (run_id, kind, processed, total, succeeded, failed)
    let run = db.current_lens_run(&lens.id).expect("current_run").expect("Some");
    assert_eq!(run.0, run_id); // run_id matches
    assert_eq!(run.1, "backfill"); // kind
    assert_eq!(run.3, 42); // total stored at insert time

    db.finish_lens_run(&run_id, "succeeded", None).expect("finish");
    // Finished run is no longer "running" — current_lens_run returns None.
    assert!(db.current_lens_run(&lens.id).expect("after finish").is_none());
}

#[test]
fn orphan_lens_run_reset() {
    use emailops_lib::models::lens::LensRunKind;

    let db = test_db();
    let lens = db.create_lens(&make_lens_input("Orphan Test")).expect("create");
    let _run_id = db
        .insert_lens_run(&lens.id, LensRunKind::Incremental, 0)
        .expect("insert");

    // reset_orphan_lens_runs marks every in-progress run as failed.
    let count = db.reset_orphan_lens_runs().expect("reset_orphans");
    assert_eq!(count, 1, "exactly one running run should have been reset");

    // After reset there is no in-progress run.
    assert!(db.current_lens_run(&lens.id).expect("current").is_none());
}

// ── helpers for new test sections ──────────────────────────────────────────

fn make_draft(account_id: &str) -> SaveDraftRequest {
    SaveDraftRequest {
        id: None,
        email_id: None,
        account_id: account_id.to_string(),
        to_addresses: vec!["recipient@example.com".to_string()],
        subject: "Draft Subject".to_string(),
        body: "Draft body text.".to_string(),
    }
}

fn make_draft_with_id(id: &str, account_id: &str) -> SaveDraftRequest {
    SaveDraftRequest {
        id: Some(id.to_string()),
        ..make_draft(account_id)
    }
}

/// Email with configurable sender_email and mailbox for mailbox-view tests.
fn make_email_with(id: &str, account_id: &str, timestamp: i64, sender_email: &str, mailbox: &str) -> Email {
    Email {
        id: id.to_string(),
        account_id: account_id.to_string(),
        thread_id: format!("thread-{id}"),
        message_id: None,
        subject: format!("Subject {id}"),
        sender: "Test Sender".to_string(),
        sender_email: sender_email.to_string(),
        recipients: vec!["recipient@example.com".to_string()],
        cc: vec![],
        body: format!("Body of email {id}"),
        snippet: format!("Snippet {id}"),
        timestamp,
        is_read: false,
        triage_status: None,
        category: "primary".to_string(),
        mailbox: mailbox.to_string(),
    }
}

// ── P0: draft lifecycle ────────────────────────────────────────────────────

#[test]
fn save_draft_creates_with_status_draft() {
    let db = test_db();
    db.insert_account(&make_account("acc-d", "d@example.com")).unwrap();
    let draft = db.save_draft(&make_draft("acc-d")).unwrap();
    assert_eq!(draft.status, "draft");
    assert!(!draft.id.is_empty());
    assert_eq!(draft.account_id, "acc-d");
}

#[test]
fn save_draft_roundtrips_fields() {
    let db = test_db();
    db.insert_account(&make_account("acc-d", "d@example.com")).unwrap();
    let req = SaveDraftRequest {
        id: None,
        email_id: None,
        account_id: "acc-d".to_string(),
        to_addresses: vec!["a@x.com".to_string(), "b@y.com".to_string()],
        subject: "Hello".to_string(),
        body: "World".to_string(),
    };
    let draft = db.save_draft(&req).unwrap();
    assert_eq!(draft.subject, "Hello");
    assert_eq!(draft.body, "World");
    assert_eq!(draft.to_addresses, vec!["a@x.com", "b@y.com"]);
    assert!(!draft.ai_generated);
}

#[test]
fn save_draft_update_by_id_changes_body() {
    let db = test_db();
    db.insert_account(&make_account("acc-d", "d@example.com")).unwrap();
    let created = db.save_draft(&make_draft_with_id("draft-1", "acc-d")).unwrap();
    assert_eq!(created.body, "Draft body text.");

    let updated = db
        .save_draft(&SaveDraftRequest {
            id: Some("draft-1".to_string()),
            body: "Updated body".to_string(),
            ..make_draft("acc-d")
        })
        .unwrap();
    assert_eq!(updated.id, "draft-1");
    assert_eq!(updated.body, "Updated body");
}

#[test]
fn save_draft_preserves_created_at_on_update() {
    let db = test_db();
    db.insert_account(&make_account("acc-d", "d@example.com")).unwrap();
    let created = db.save_draft(&make_draft_with_id("draft-2", "acc-d")).unwrap();
    let original_created_at = created.created_at;

    let updated = db
        .save_draft(&SaveDraftRequest {
            id: Some("draft-2".to_string()),
            body: "Changed".to_string(),
            ..make_draft("acc-d")
        })
        .unwrap();
    assert_eq!(
        updated.created_at, original_created_at,
        "created_at must not change on update"
    );
}

#[test]
fn save_draft_with_multiple_recipients_roundtrips() {
    let db = test_db();
    db.insert_account(&make_account("acc-d", "d@example.com")).unwrap();
    let req = SaveDraftRequest {
        id: None,
        email_id: None,
        account_id: "acc-d".to_string(),
        to_addresses: vec!["x@a.com".to_string(), "y@b.com".to_string(), "z@c.com".to_string()],
        subject: "Multi-recipient".to_string(),
        body: "body".to_string(),
    };
    let draft = db.save_draft(&req).unwrap();
    assert_eq!(draft.to_addresses.len(), 3);
    assert!(draft.to_addresses.contains(&"y@b.com".to_string()));
}

#[test]
fn list_drafts_empty_when_no_drafts() {
    let db = test_db();
    db.insert_account(&make_account("acc-d", "d@example.com")).unwrap();
    let drafts = db.list_drafts("acc-d").unwrap();
    assert!(drafts.is_empty());
}

#[test]
fn list_drafts_scoped_to_account() {
    let db = test_db();
    db.insert_account(&make_account("acc-1", "a1@example.com")).unwrap();
    db.insert_account(&make_account("acc-2", "a2@example.com")).unwrap();
    db.save_draft(&make_draft("acc-1")).unwrap();
    db.save_draft(&make_draft("acc-2")).unwrap();

    let acc1_drafts = db.list_drafts("acc-1").unwrap();
    let acc2_drafts = db.list_drafts("acc-2").unwrap();
    assert_eq!(acc1_drafts.len(), 1);
    assert_eq!(acc2_drafts.len(), 1);
    assert_eq!(acc1_drafts[0].account_id, "acc-1");
    assert_eq!(acc2_drafts[0].account_id, "acc-2");
}

#[test]
fn delete_draft_removes_from_list() {
    let db = test_db();
    db.insert_account(&make_account("acc-d", "d@example.com")).unwrap();
    let d1 = db.save_draft(&make_draft("acc-d")).unwrap();
    let d2 = db.save_draft(&make_draft("acc-d")).unwrap();
    assert_eq!(db.list_drafts("acc-d").unwrap().len(), 2);

    db.delete_draft(&d1.id, "acc-d").unwrap();
    let remaining = db.list_drafts("acc-d").unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, d2.id);
}

#[test]
fn delete_draft_wrong_account_does_not_delete() {
    let db = test_db();
    db.insert_account(&make_account("acc-owner", "owner@example.com"))
        .unwrap();
    db.insert_account(&make_account("acc-other", "other@example.com"))
        .unwrap();
    let draft = db.save_draft(&make_draft_with_id("d-owned", "acc-owner")).unwrap();

    // Deleting with a different account_id must be a no-op
    db.delete_draft(&draft.id, "acc-other").unwrap();
    let remaining = db.list_drafts("acc-owner").unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "draft must still exist after wrong-account delete attempt"
    );
}

// ── P0: send contract via FakeEmailProvider ────────────────────────────────
//
// The service functions `send_reply` / `send_new_email` in
// `services/emails/send.rs` require a Tauri `AppHandle` (via `emit_progress`
// in `events.rs`). Until `emit_progress` is refactored to use the Logger seam,
// those functions cannot be called from integration tests. These tests verify
// the provider seam contract — the surface that the service layer calls into.

#[tokio::test]
async fn fake_provider_send_reply_records_all_fields() {
    use emailops_lib::sync::provider::EmailBody;
    let provider = FakeEmailProvider::new("me@example.com", "Me");
    provider
        .send_reply(
            "me@example.com",
            &["them@example.com".to_string()],
            &["cc@example.com".to_string()],
            "thread-xyz",
            Some("orig-msg-id"),
            "Re: Hello",
            &EmailBody::plain("reply body"),
            &[],
        )
        .await
        .unwrap();

    let sent = provider.sent();
    assert_eq!(sent.len(), 1);
    let msg = &sent[0];
    assert_eq!(msg.from_email, "me@example.com");
    assert_eq!(msg.to_emails, vec!["them@example.com"]);
    assert_eq!(msg.cc_emails, vec!["cc@example.com"]);
    assert_eq!(msg.thread_id.as_deref(), Some("thread-xyz"));
    assert_eq!(msg.original_message_id.as_deref(), Some("orig-msg-id"));
    assert_eq!(msg.subject, "Re: Hello");
    assert_eq!(msg.body.text, "reply body");
    assert!(msg.body.html.is_none());
}

#[tokio::test]
async fn fake_provider_send_new_email_records_attachments() {
    use emailops_lib::sync::provider::EmailBody;
    let provider = FakeEmailProvider::new("me@example.com", "Me");
    let attach = EmailAttachment {
        filename: "report.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        data: "base64data".to_string(),
        content_id: None,
        is_inline: false,
    };
    provider
        .send_new_email(
            "me@example.com",
            &["x@y.com".to_string()],
            &[],
            "Report",
            &EmailBody::plain("see attached"),
            &[attach],
        )
        .await
        .unwrap();

    let sent = provider.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].thread_id.is_none(), "new email must not have a thread_id");
    assert_eq!(sent[0].attachments.len(), 1);
    assert_eq!(sent[0].attachments[0].filename, "report.pdf");
}

// ── P1: account CRUD cascade ───────────────────────────────────────────────

#[test]
fn delete_account_cascades_emails() {
    let db = test_db();
    db.insert_account(&make_account("acc-c", "c@example.com")).unwrap();
    db.insert_email(&make_email("e-c1", "acc-c", 1000)).unwrap();
    db.insert_email(&make_email("e-c2", "acc-c", 2000)).unwrap();

    db.delete_account("acc-c").unwrap();

    // Emails must be gone (get_email uses a reader and includes soft-deleted rows)
    assert!(db.get_email("e-c1").unwrap().is_none());
    assert!(db.get_email("e-c2").unwrap().is_none());
}

#[test]
fn delete_account_cascades_sync_state() {
    let db = test_db();
    db.insert_account(&make_account("acc-c", "c@example.com")).unwrap();
    db.upsert_sync_status("acc-c", "syncing", None, None).unwrap();

    db.delete_account("acc-c").unwrap();

    // Sync status after deletion should default to idle (no stored row)
    let status = db.get_sync_status("acc-c").unwrap();
    assert_eq!(
        status.status, "idle",
        "sync_state row must be gone after account deletion"
    );
    assert!(status.last_sync_at.is_none());
}

#[test]
fn delete_account_cascades_drafts() {
    let db = test_db();
    db.insert_account(&make_account("acc-c", "c@example.com")).unwrap();
    db.save_draft(&make_draft_with_id("draft-c", "acc-c")).unwrap();

    db.delete_account("acc-c").unwrap();

    let drafts = db.list_drafts("acc-c").unwrap();
    assert!(drafts.is_empty(), "drafts must be removed when account is deleted");
}

#[test]
fn update_account_enabled_persists() {
    let db = test_db();
    db.insert_account(&make_account("acc-e", "e@example.com")).unwrap();

    db.update_account_enabled("acc-e", false).unwrap();
    let fetched = db.get_account("acc-e").unwrap().unwrap();
    assert!(!fetched.enabled, "account must be disabled after update");

    db.update_account_enabled("acc-e", true).unwrap();
    let fetched2 = db.get_account("acc-e").unwrap().unwrap();
    assert!(fetched2.enabled, "account must be re-enabled after second update");
}

#[test]
fn reorder_accounts_persists_sort_order() {
    let db = test_db();
    db.insert_account(&make_account("acc-r1", "r1@example.com")).unwrap();
    db.insert_account(&make_account("acc-r2", "r2@example.com")).unwrap();

    // Reverse order: r2 first, r1 second
    db.update_account_order(&["acc-r2".to_string(), "acc-r1".to_string()])
        .unwrap();

    let list = db.list_accounts().unwrap();
    assert_eq!(list[0].id, "acc-r2", "acc-r2 should be first after reorder");
    assert_eq!(list[1].id, "acc-r1", "acc-r1 should be second after reorder");
}

#[test]
fn update_account_sync_from_persists() {
    let db = test_db();
    db.insert_account(&make_account("acc-sf", "sf@example.com")).unwrap();

    let ts: i64 = 1_700_000_000;
    db.update_account_sync_from("acc-sf", Some(ts)).unwrap();
    let fetched = db.get_account("acc-sf").unwrap().unwrap();
    assert_eq!(fetched.sync_from_timestamp, Some(ts));

    db.update_account_sync_from("acc-sf", None).unwrap();
    let fetched2 = db.get_account("acc-sf").unwrap().unwrap();
    assert!(fetched2.sync_from_timestamp.is_none());
}

// ── P1: filter evaluation ──────────────────────────────────────────────────

#[test]
fn pin_filter_sets_status_pinned() {
    let db = test_db();
    db.insert_account(&make_account("acc-f", "f@example.com")).unwrap();

    emailops_lib::services::filters::pin_filter(&db, "acc-f", "domain", "gmail.com").unwrap();

    let prefs = db.get_filter_prefs("acc-f").unwrap();
    let pref = prefs.iter().find(|p| p.filter_value == "gmail.com").unwrap();
    assert_eq!(pref.status, "pinned");
    assert_eq!(pref.filter_type, "domain");
    assert_eq!(pref.account_id, "acc-f");
}

#[test]
fn remove_filter_sets_status_removed() {
    let db = test_db();
    db.insert_account(&make_account("acc-f", "f@example.com")).unwrap();

    emailops_lib::services::filters::remove_filter(&db, "acc-f", "sender", "boss@corp.com").unwrap();

    let prefs = db.get_filter_prefs("acc-f").unwrap();
    let pref = prefs.iter().find(|p| p.filter_value == "boss@corp.com").unwrap();
    assert_eq!(pref.status, "removed");
}

#[test]
fn delete_filter_pref_removes_row() {
    let db = test_db();
    db.insert_account(&make_account("acc-f", "f@example.com")).unwrap();
    emailops_lib::services::filters::pin_filter(&db, "acc-f", "domain", "github.com").unwrap();

    assert_eq!(db.get_filter_prefs("acc-f").unwrap().len(), 1);

    emailops_lib::services::filters::delete_filter_pref(&db, "acc-f", "domain", "github.com").unwrap();
    assert!(db.get_filter_prefs("acc-f").unwrap().is_empty());
}

// Regression test for bug: filter pref id did not include account_id, so two accounts
// pinning the same filter would overwrite each other's row in the DB.
#[test]
fn pin_filter_two_accounts_same_value_are_independent() {
    let db = test_db();
    db.insert_account(&make_account("acc-1", "a1@example.com")).unwrap();
    db.insert_account(&make_account("acc-2", "a2@example.com")).unwrap();

    emailops_lib::services::filters::pin_filter(&db, "acc-1", "domain", "gmail.com").unwrap();
    emailops_lib::services::filters::pin_filter(&db, "acc-2", "domain", "gmail.com").unwrap();

    let prefs1 = db.get_filter_prefs("acc-1").unwrap();
    let prefs2 = db.get_filter_prefs("acc-2").unwrap();

    assert_eq!(prefs1.len(), 1, "acc-1 must retain its own pin");
    assert_eq!(prefs2.len(), 1, "acc-2 must have its own pin");
    assert_eq!(prefs1[0].account_id, "acc-1");
    assert_eq!(prefs2[0].account_id, "acc-2");
}

#[test]
fn removed_filter_excluded_from_quick_filter_stats() {
    let db = test_db();
    db.insert_account(&make_account("acc-f", "f@example.com")).unwrap();
    // Insert emails from two domains
    db.insert_email(&make_email_with("e1", "acc-f", 1000, "a@gmail.com", "inbox"))
        .unwrap();
    db.insert_email(&make_email_with("e2", "acc-f", 2000, "b@github.com", "inbox"))
        .unwrap();
    db.insert_email(&make_email_with("e3", "acc-f", 3000, "c@github.com", "inbox"))
        .unwrap();

    // Mark gmail.com as removed
    emailops_lib::services::filters::remove_filter(&db, "acc-f", "domain", "gmail.com").unwrap();

    let prefs = db.get_filter_prefs("acc-f").unwrap();
    let excluded_domains: Vec<String> = prefs
        .iter()
        .filter(|p| p.status == "removed" && p.filter_type == "domain")
        .map(|p| p.filter_value.clone())
        .collect();

    let stats = db.get_quick_filter_stats("acc-f", &excluded_domains, &[]).unwrap();
    let domain_values: Vec<&str> = stats.top_domains.iter().map(|d| d.value.as_str()).collect();

    assert!(
        !domain_values.contains(&"gmail.com"),
        "removed domain must not appear in suggestions"
    );
    assert!(
        domain_values.contains(&"github.com"),
        "non-removed domain must appear in suggestions"
    );
}

#[test]
fn get_filtered_emails_by_domain_returns_matching_threads() {
    let db = test_db();
    db.insert_account(&make_account("acc-f", "f@example.com")).unwrap();

    // Two emails from gmail.com, one from outlook.com
    db.insert_email(&make_email_with("e1", "acc-f", 1000, "x@gmail.com", "inbox"))
        .unwrap();
    db.insert_email(&make_email_with("e2", "acc-f", 2000, "y@gmail.com", "inbox"))
        .unwrap();
    db.insert_email(&make_email_with("e3", "acc-f", 3000, "z@outlook.com", "inbox"))
        .unwrap();

    let result = db
        .get_filtered_emails("acc-f", Some("gmail.com"), None, None, None, None, 50, 0)
        .unwrap();
    let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
    assert!(
        ids.contains(&"e1") || ids.contains(&"e2"),
        "at least one gmail.com email must match"
    );
    assert!(
        !ids.contains(&"e3"),
        "outlook.com email must not match gmail.com filter"
    );
}

#[test]
fn get_filtered_emails_by_sender_returns_matching_threads() {
    let db = test_db();
    db.insert_account(&make_account("acc-f", "f@example.com")).unwrap();

    db.insert_email(&make_email_with("e1", "acc-f", 1000, "boss@corp.com", "inbox"))
        .unwrap();
    db.insert_email(&make_email_with("e2", "acc-f", 2000, "other@corp.com", "inbox"))
        .unwrap();

    let result = db
        .get_filtered_emails("acc-f", None, Some("boss@corp.com"), None, None, None, 50, 0)
        .unwrap();
    let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"e1"), "boss@corp.com email must match");
    assert!(!ids.contains(&"e2"), "other@corp.com email must not match");
}

// ── P1: email CRUD edge cases ──────────────────────────────────────────────

#[test]
fn emails_exist_batch_handles_empty_input() {
    let db = test_db();
    let result = db.emails_exist_batch(&[]).unwrap();
    assert!(result.is_empty(), "empty input must produce empty result");
}

#[test]
fn emails_exist_batch_includes_soft_deleted() {
    let db = test_db();
    db.insert_account(&make_account("acc-b", "b@example.com")).unwrap();
    db.insert_email(&make_email("e-del", "acc-b", 1000)).unwrap();
    db.delete_email("e-del").unwrap();

    let ids = vec!["e-del".to_string()];
    let existing = db.emails_exist_batch(&ids).unwrap();
    assert!(
        existing.contains("e-del"),
        "soft-deleted email must still appear in batch check to prevent re-download"
    );
}

#[test]
fn emails_exist_batch_returns_only_existing_ids() {
    let db = test_db();
    db.insert_account(&make_account("acc-b", "b@example.com")).unwrap();
    db.insert_email(&make_email("e-yes", "acc-b", 1000)).unwrap();

    let ids = vec!["e-yes".to_string(), "e-no".to_string()];
    let existing = db.emails_exist_batch(&ids).unwrap();
    assert!(existing.contains("e-yes"));
    assert!(!existing.contains("e-no"));
    assert_eq!(existing.len(), 1);
}

#[test]
fn get_email_body_returns_empty_string_when_missing() {
    // get_email_body must return "" for an email that has no body row yet
    // (e.g. sync downloaded metadata but body fetch failed)
    let db = test_db();
    let body = db.get_email_body("nonexistent-email-id").unwrap();
    assert_eq!(body, "", "missing body must return empty string, not an error");
}

#[test]
fn get_emails_sent_view_filters_by_sender_address() {
    let db = test_db();
    // Account email is "me@example.com"
    db.insert_account(&make_account("acc-s", "me@example.com")).unwrap();
    // Email sent by me
    db.insert_email(&make_email_with("e-sent", "acc-s", 2000, "me@example.com", "inbox"))
        .unwrap();
    // Email received from someone else
    db.insert_email(&make_email_with("e-recv", "acc-s", 1000, "other@example.com", "inbox"))
        .unwrap();

    let sent = db.get_emails("acc-s", 50, 0, None, Some("sent"), None).unwrap();
    let ids: Vec<&str> = sent.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"e-sent"), "email sent by me must appear in sent view");
    assert!(
        !ids.contains(&"e-recv"),
        "email received from others must not appear in sent view"
    );
}

#[test]
fn get_emails_spam_view_filters_by_mailbox() {
    let db = test_db();
    db.insert_account(&make_account("acc-sp", "sp@example.com")).unwrap();
    db.insert_email(&make_email_with("e-spam", "acc-sp", 1000, "spammer@bad.com", "spam"))
        .unwrap();
    db.insert_email(&make_email_with("e-inbox", "acc-sp", 2000, "good@example.com", "inbox"))
        .unwrap();

    let spam = db.get_emails("acc-sp", 50, 0, None, Some("spam"), None).unwrap();
    let ids: Vec<&str> = spam.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"e-spam"), "spam mailbox email must appear in spam view");
    assert!(!ids.contains(&"e-inbox"), "inbox email must not appear in spam view");
}

#[test]
fn count_emails_counts_inbox_threads_only() {
    let db = test_db();
    db.insert_account(&make_account("acc-cnt", "cnt@example.com")).unwrap();

    // 2 inbox emails in different threads
    db.insert_email(&make_email_with("e1", "acc-cnt", 1000, "a@x.com", "inbox"))
        .unwrap();
    db.insert_email(&make_email_with("e2", "acc-cnt", 2000, "b@x.com", "inbox"))
        .unwrap();
    // 1 spam email (must NOT be counted)
    db.insert_email(&make_email_with("e3", "acc-cnt", 3000, "s@spam.com", "spam"))
        .unwrap();

    let count = db.count_emails("acc-cnt").unwrap();
    assert_eq!(count, 2, "count_emails must only count inbox threads, not spam");
}

// ── P0: send service (via FakeEmailProvider) ──────────────────────────────

#[tokio::test]
async fn send_reply_with_provider_routes_to_provider() {
    let db = test_db();
    db.insert_account(&make_account("acc-s", "me@example.com")).unwrap();
    let email = make_email_with("orig-1", "acc-s", 1000, "sender@other.com", "inbox");
    db.insert_email(&email).unwrap();

    let provider = FakeEmailProvider::new("me@example.com", "Me");
    emailops_lib::services::emails::send_reply_with_provider(
        &db,
        "orig-1",
        &emailops_lib::sync::provider::EmailBody::plain("Hello back!"),
        None,
        None,
        None,
        vec![],
        &provider,
    )
    .await
    .expect("send_reply_with_provider");

    let sent = provider.sent();
    assert_eq!(sent.len(), 1, "exactly one message must be recorded");
    assert_eq!(sent[0].body.text, "Hello back!");
    assert_eq!(sent[0].from_email, "me@example.com");
    // When to_emails is None, defaults to the original sender
    assert_eq!(sent[0].to_emails, vec!["sender@other.com"]);
}

#[tokio::test]
async fn send_reply_with_provider_uses_explicit_to() {
    let db = test_db();
    db.insert_account(&make_account("acc-s2", "me2@example.com")).unwrap();
    let email = make_email_with("orig-2", "acc-s2", 1000, "original@other.com", "inbox");
    db.insert_email(&email).unwrap();

    let provider = FakeEmailProvider::new("me2@example.com", "Me");
    emailops_lib::services::emails::send_reply_with_provider(
        &db,
        "orig-2",
        &emailops_lib::sync::provider::EmailBody::plain("Explicit reply"),
        None,
        Some(vec!["override@dest.com".to_string()]),
        None,
        vec![],
        &provider,
    )
    .await
    .expect("send_reply_with_provider");

    let sent = provider.sent();
    assert_eq!(sent[0].to_emails, vec!["override@dest.com"]);
    assert_eq!(sent[0].cc_emails.len(), 0);
}

#[tokio::test]
async fn send_reply_with_provider_forwards_attachments() {
    let db = test_db();
    db.insert_account(&make_account("acc-ra", "me@example.com")).unwrap();
    let email = make_email_with("orig-ra", "acc-ra", 1000, "sender@other.com", "inbox");
    db.insert_email(&email).unwrap();

    let provider = FakeEmailProvider::new("me@example.com", "Me");
    let attachment = EmailAttachment {
        filename: "report.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        data: "QUFB".to_string(),
        content_id: None,
        is_inline: false,
    };
    emailops_lib::services::emails::send_reply_with_provider(
        &db,
        "orig-ra",
        &emailops_lib::sync::provider::EmailBody::plain("see attached"),
        None,
        None,
        None,
        vec![attachment],
        &provider,
    )
    .await
    .expect("send_reply_with_provider");

    let sent = provider.sent();
    assert_eq!(sent.len(), 1, "exactly one message must be recorded");
    assert_eq!(sent[0].attachments.len(), 1, "attachment must reach the provider");
    assert_eq!(sent[0].attachments[0].filename, "report.pdf");
    assert_eq!(sent[0].attachments[0].mime_type, "application/pdf");
    assert!(sent[0].original_message_id.is_some() || sent[0].thread_id.is_some());
}

#[tokio::test]
async fn send_reply_with_provider_fails_when_email_missing() {
    let db = test_db();
    db.insert_account(&make_account("acc-s3", "s3@example.com")).unwrap();

    let provider = FakeEmailProvider::new("s3@example.com", "Me");
    let result = emailops_lib::services::emails::send_reply_with_provider(
        &db,
        "no-such-email",
        &emailops_lib::sync::provider::EmailBody::plain("body"),
        None,
        None,
        None,
        vec![],
        &provider,
    )
    .await;

    assert!(result.is_err(), "must fail when email does not exist");
}

#[tokio::test]
async fn send_new_email_with_provider_records_message() {
    let db = test_db();
    db.insert_account(&make_account("acc-n", "from@example.com")).unwrap();

    let provider = FakeEmailProvider::new("from@example.com", "Me");
    emailops_lib::services::emails::send_new_email_with_provider(
        &db,
        "acc-n",
        vec!["to1@dest.com".to_string(), "to2@dest.com".to_string()],
        vec!["cc@dest.com".to_string()],
        "Test Subject",
        &emailops_lib::sync::provider::EmailBody::plain("Test body"),
        vec![],
        &provider,
    )
    .await
    .expect("send_new_email_with_provider");

    let sent = provider.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].subject, "Test Subject");
    assert_eq!(sent[0].body.text, "Test body");
    assert_eq!(sent[0].from_email, "from@example.com");
    assert_eq!(sent[0].to_emails, vec!["to1@dest.com", "to2@dest.com"]);
    assert_eq!(sent[0].cc_emails, vec!["cc@dest.com"]);
    assert!(sent[0].thread_id.is_none(), "new email has no thread_id");
}

#[tokio::test]
async fn send_applies_ui_language_to_footer() {
    // The "Sent with EmailOps" footer must follow the user's UI-language
    // preference. The service resolves it and stamps it onto the EmailBody the
    // provider receives; the MIME builder then renders the localized footer.
    let db = test_db();
    db.insert_account(&make_account("acc-es", "from@example.com")).unwrap();
    db.set_preference("ui_language", "es").unwrap();

    let provider = FakeEmailProvider::new("from@example.com", "Me");
    emailops_lib::services::emails::send_new_email_with_provider(
        &db,
        "acc-es",
        vec!["to@dest.com".to_string()],
        vec![],
        "Hola",
        &emailops_lib::sync::provider::EmailBody::plain("cuerpo"),
        vec![],
        &provider,
    )
    .await
    .expect("send_new_email_with_provider");

    let sent = provider.sent();
    assert_eq!(
        sent[0].body.language,
        emailops_lib::services::i18n::Language::Es,
        "footer language must follow the UI-language preference"
    );
    // Body text itself is untouched — the footer is added at the MIME layer.
    assert_eq!(sent[0].body.text, "cuerpo");
}

#[tokio::test]
async fn send_new_email_with_provider_fails_for_unknown_account() {
    let db = test_db();
    let provider = FakeEmailProvider::new("x@example.com", "X");
    let result = emailops_lib::services::emails::send_new_email_with_provider(
        &db,
        "no-such-account",
        vec!["to@dest.com".to_string()],
        vec![],
        "Subject",
        &emailops_lib::sync::provider::EmailBody::plain("Body"),
        vec![],
        &provider,
    )
    .await;
    assert!(result.is_err(), "must fail when account does not exist");
}

// ── P0: sync service (via FakeEmailProvider) ──────────────────────────────

/// Helper: empty sync infrastructure for tests (no concurrency controls needed).
/// The return type mirrors what production sync code constructs in `lib.rs`;
/// extracting a type alias just for the test would obscure that parallel.
#[allow(clippy::type_complexity)]
fn test_sync_state() -> (Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>, TaskQueue) {
    let abort_flags = Arc::new(Mutex::new(HashMap::new()));
    let ai_queue = TaskQueue::new(1, "test-ai");
    (abort_flags, ai_queue)
}

#[tokio::test]
async fn sync_with_provider_stores_new_emails() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-sy", "sy@example.com")).unwrap();
    let account = db.get_account("acc-sy").unwrap().unwrap();

    // FakeEmailProvider has two messages available
    let provider = FakeEmailProvider::new("sy@example.com", "Sy");
    provider.add_message(
        make_email_with("msg-1", "acc-sy", 1000, "a@x.com", "inbox"),
        EmailCategory::Primary,
        vec![],
    );
    provider.add_message(
        make_email_with("msg-2", "acc-sy", 2000, "b@y.com", "inbox"),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync_account_with_provider");

    let emails = db.get_emails("acc-sy", 50, 0, None, None, None).unwrap();
    assert_eq!(emails.len(), 2, "both new emails must be stored");
}

#[tokio::test]
async fn sync_with_provider_deduplicates_existing_emails() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-dd", "dd@example.com")).unwrap();
    let account = db.get_account("acc-dd").unwrap().unwrap();

    // Pre-populate with one email that already exists
    let existing = make_email_with("existing-1", "acc-dd", 1000, "old@x.com", "inbox");
    db.insert_email(&existing).unwrap();

    // FakeEmailProvider returns the same existing email PLUS one new one
    let provider = FakeEmailProvider::new("dd@example.com", "Dd");
    provider.add_message(existing.clone(), EmailCategory::Primary, vec![]);
    provider.add_message(
        make_email_with("new-1", "acc-dd", 2000, "new@x.com", "inbox"),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync_account_with_provider");

    let emails = db.get_emails("acc-dd", 50, 0, None, None, None).unwrap();
    assert_eq!(
        emails.len(),
        2,
        "existing email must not be duplicated; only the new one is added"
    );
    let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"existing-1"));
    assert!(ids.contains(&"new-1"));
}

#[tokio::test]
async fn sync_with_provider_incremental_uses_latest_timestamp() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-inc", "inc@example.com")).unwrap();
    let account = db.get_account("acc-inc").unwrap().unwrap();

    // Pre-populate with an email at timestamp 1000
    db.insert_email(&make_email_with("old-1", "acc-inc", 1000, "old@x.com", "inbox"))
        .unwrap();

    // Provider has one old message (ts=1000) and one new (ts=2000)
    let provider = FakeEmailProvider::new("inc@example.com", "Inc");
    provider.add_message(
        make_email_with("old-1", "acc-inc", 1000, "old@x.com", "inbox"),
        EmailCategory::Primary,
        vec![],
    );
    provider.add_message(
        make_email_with("new-2", "acc-inc", 2000, "new@x.com", "inbox"),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync_account_with_provider");

    // Only the new email should be added (old-1 is already there)
    let emails = db.get_emails("acc-inc", 50, 0, None, None, None).unwrap();
    assert_eq!(emails.len(), 2, "total must be old + new");
    let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"new-2"), "new email must be synced");
}

#[tokio::test]
async fn sync_with_provider_no_new_emails_leaves_db_unchanged() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-noop", "noop@example.com"))
        .unwrap();
    let account = db.get_account("acc-noop").unwrap().unwrap();

    let existing = make_email_with("e-existing", "acc-noop", 1000, "x@x.com", "inbox");
    db.insert_email(&existing).unwrap();

    // Provider only returns already-known email
    let provider = FakeEmailProvider::new("noop@example.com", "Noop");
    provider.add_message(existing, EmailCategory::Primary, vec![]);

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync_account_with_provider");

    let emails = db.get_emails("acc-noop", 50, 0, None, None, None).unwrap();
    assert_eq!(emails.len(), 1, "no emails must be added when nothing is new");
}

#[tokio::test]
async fn inbox_incremental_watermark_is_not_poisoned_by_sent_timestamp() {
    // Regression for: "Gmail last received email never appears."
    //
    // The incremental sync watermark must be the latest INBOX timestamp,
    // not the latest timestamp across all mailboxes. Otherwise sending a
    // reply pushes the watermark ahead of any received emails that
    // landed between syncs but before the sent email's timestamp, and
    // those received emails are silently missed forever.
    //
    // Scenario:
    //   - DB has one received inbox email at ts=1000 (last sync).
    //   - User sends a reply at ts=5000 — stored locally with mailbox='sent'.
    //   - Meanwhile a new email arrives in inbox at ts=2000 (1000 < 2000 < 5000).
    //   - Next sync: provider's `list_messages(after_timestamp=?)` is
    //     called with the inbox watermark. With the bug it's `after:5000`
    //     and the ts=2000 email is filtered out at the provider; with the
    //     fix it's `after:1000` and the ts=2000 email is picked up.
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-watermark", "wm@example.com"))
        .unwrap();
    let account = db.get_account("acc-watermark").unwrap().unwrap();

    // Pre-existing state: one received email at ts=1000 + one locally
    // stored sent email at ts=5000.
    db.insert_email(&make_email_with(
        "received-old",
        "acc-watermark",
        1000,
        "external@x.com",
        "inbox",
    ))
    .unwrap();
    db.insert_email(&make_email_with(
        "sent-reply",
        "acc-watermark",
        5000,
        "wm@example.com",
        "sent",
    ))
    .unwrap();

    // Provider has the existing received email + a NEW inbox email at
    // ts=2000 (between the two existing timestamps).
    let provider = FakeEmailProvider::new("wm@example.com", "Wm");
    provider.add_message(
        make_email_with("received-old", "acc-watermark", 1000, "external@x.com", "inbox"),
        EmailCategory::Primary,
        vec![],
    );
    provider.add_message(
        make_email_with("received-new", "acc-watermark", 2000, "another@x.com", "inbox"),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync_account_with_provider");

    let emails = db.get_emails("acc-watermark", 50, 0, None, None, None).unwrap();
    let ids: std::collections::HashSet<&str> = emails.iter().map(|e| e.id.as_str()).collect();
    assert!(
        ids.contains("received-new"),
        "new inbox email at ts=2000 MUST be synced even though a locally \
         stored sent email at ts=5000 has a later timestamp — got {:?}",
        ids
    );
}

// ── Failing email provider for send/sync error-path tests ──────────────────

struct FailingEmailProvider;

#[async_trait::async_trait]
impl EmailProvider for FailingEmailProvider {
    async fn get_profile(&self) -> emailops_lib::models::error::Result<(String, String)> {
        Ok(("fail@example.com".to_string(), "Failing Provider".to_string()))
    }

    async fn list_messages(
        &self,
        _max_results: u32,
        _page_token: Option<&str>,
        _after_timestamp: Option<i64>,
        _before_timestamp: Option<i64>,
        _label_filter: Option<&str>,
    ) -> emailops_lib::models::error::Result<(Vec<MessageRef>, Option<String>)> {
        Ok((Vec::new(), None))
    }

    async fn get_message(
        &self,
        message_id: &str,
    ) -> emailops_lib::models::error::Result<(emailops_lib::models::Email, EmailCategory, Vec<AttachmentInfo>)> {
        Err(AppError::NotFound(format!("message not found: {message_id}")))
    }

    async fn send_reply(
        &self,
        _from_email: &str,
        _to_emails: &[String],
        _cc_emails: &[String],
        _thread_id: &str,
        _original_message_id: Option<&str>,
        _subject: &str,
        _body: &emailops_lib::sync::provider::EmailBody,
        _attachments: &[EmailAttachment],
    ) -> emailops_lib::models::error::Result<()> {
        Err(AppError::SyncError("Deliberate provider send failure".to_string()))
    }

    async fn send_new_email(
        &self,
        _from_email: &str,
        _to_emails: &[String],
        _cc_emails: &[String],
        _subject: &str,
        _body: &emailops_lib::sync::provider::EmailBody,
        _attachments: &[EmailAttachment],
    ) -> emailops_lib::models::error::Result<()> {
        Err(AppError::SyncError("Deliberate provider send failure".to_string()))
    }

    async fn fetch_attachment_bytes(
        &self,
        _message_id: &str,
        _attachment_id: &str,
    ) -> emailops_lib::models::error::Result<Vec<u8>> {
        Ok(Vec::new())
    }

    async fn list_mailbox_messages(
        &self,
        _mailbox: ExtraMailbox,
        _max_results: u32,
        _after_timestamp: Option<i64>,
        _before_timestamp: Option<i64>,
    ) -> emailops_lib::models::error::Result<Vec<MessageRef>> {
        Ok(Vec::new())
    }
}

// ── P1: email CRUD — mark as read / soft delete ────────────────────────────

#[test]
fn mark_as_read_flips_is_read_flag() {
    let db = test_db();
    db.insert_account(&make_account("acc-r", "r@example.com")).unwrap();
    let email = make_email("e-unread", "acc-r", 1000);
    db.insert_email(&email).unwrap();

    // Starts unread
    let before = db.get_email_by_id("e-unread").unwrap().unwrap();
    assert!(!before.is_read, "email must start unread");

    emailops_lib::services::emails::mark_as_read(&db, "e-unread").unwrap();

    let after = db.get_email_by_id("e-unread").unwrap().unwrap();
    assert!(after.is_read, "email must be read after mark_as_read");
    // Other fields unchanged
    assert_eq!(after.subject, before.subject);
    assert_eq!(after.sender_email, before.sender_email);
}

#[test]
fn mark_as_read_is_idempotent() {
    let db = test_db();
    db.insert_account(&make_account("acc-r2", "r2@example.com")).unwrap();
    db.insert_email(&make_email("e-r2", "acc-r2", 1000)).unwrap();

    emailops_lib::services::emails::mark_as_read(&db, "e-r2").unwrap();
    // Second call must not error
    emailops_lib::services::emails::mark_as_read(&db, "e-r2").unwrap();
    assert!(db.get_email_by_id("e-r2").unwrap().unwrap().is_read);
}

#[test]
fn delete_email_hides_it_from_get_emails() {
    let db = test_db();
    db.insert_account(&make_account("acc-del", "del@example.com")).unwrap();
    db.insert_email(&make_email("e-to-delete", "acc-del", 1000)).unwrap();
    db.insert_email(&make_email("e-keep", "acc-del", 2000)).unwrap();

    db.delete_email("e-to-delete").unwrap();

    // get_emails must exclude the soft-deleted email
    let emails = db.get_emails("acc-del", 50, 0, None, None, None).unwrap();
    let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
    assert!(
        !ids.contains(&"e-to-delete"),
        "soft-deleted email must be excluded from listing"
    );
    assert!(ids.contains(&"e-keep"), "non-deleted email must still appear");
}

#[test]
fn delete_email_still_visible_in_exist_batch() {
    // emails_exist_batch must return true for soft-deleted rows so the sync
    // layer never re-downloads an email the user has deleted.
    let db = test_db();
    db.insert_account(&make_account("acc-eb", "eb@example.com")).unwrap();
    db.insert_email(&make_email("e-soft", "acc-eb", 1000)).unwrap();
    db.delete_email("e-soft").unwrap();

    let existing = db.emails_exist_batch(&["e-soft".to_string()]).unwrap();
    assert!(
        existing.contains("e-soft"),
        "soft-deleted email must appear in exist_batch to prevent re-download"
    );
}

// ── P1: account re-add after delete ───────────────────────────────────────

#[test]
fn re_add_same_email_after_account_delete_succeeds() {
    let db = test_db();
    db.insert_account(&make_account("acc-orig", "same@example.com"))
        .unwrap();
    db.delete_account("acc-orig").unwrap();

    // Re-inserting with the same email address (new ID) must not fail
    let new_acc = Account {
        id: "acc-new".to_string(),
        provider: "gmail".to_string(),
        email: "same@example.com".to_string(),
        name: "Re-added User".to_string(),
        created_at: 2_000_000,
        sort_order: 0,
        enabled: true,
        sync_from_timestamp: None,
    };
    db.insert_account(&new_acc).unwrap();

    assert!(db.account_exists_by_email("same@example.com").unwrap());
    let fetched = db.get_account("acc-new").unwrap().unwrap();
    assert_eq!(fetched.name, "Re-added User");
}

// ── P0: sync sets status to idle on success ───────────────────────────────

#[tokio::test]
async fn sync_sets_status_idle_on_success() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-si", "si@example.com")).unwrap();
    let account = db.get_account("acc-si").unwrap().unwrap();

    let provider = FakeEmailProvider::new("si@example.com", "Si");
    provider.add_message(
        make_email_with("msg-si", "acc-si", 5000, "x@y.com", "inbox"),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync must succeed");

    let status = db.get_sync_status("acc-si").unwrap();
    assert_eq!(status.status, "idle", "sync status must be idle after successful sync");
    assert!(
        status.last_sync_at.is_some(),
        "last_sync_at must be set after successful sync"
    );
}

#[tokio::test]
async fn sync_sets_status_idle_on_no_new_emails() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-si2", "si2@example.com")).unwrap();
    let account = db.get_account("acc-si2").unwrap().unwrap();

    // Provider returns nothing new
    let provider = FakeEmailProvider::new("si2@example.com", "Si2");

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync must succeed even with no new emails");

    let status = db.get_sync_status("acc-si2").unwrap();
    assert_eq!(status.status, "idle", "sync status must be idle when nothing new");
    assert!(status.last_sync_at.is_some(), "last_sync_at must be recorded");
}

// ── P1: FTS keyword search ─────────────────────────────────────────────────

#[test]
fn search_emails_fts_keyword_finds_matching_email() {
    let db = test_db();
    db.insert_account(&make_account("acc-fts", "fts@example.com")).unwrap();

    // Use insert_emails_batch so the FTS index is populated
    let mut distinctive = make_email("e-distinctive", "acc-fts", 1000);
    distinctive.subject = "Zetamorphic invoice discussion".to_string();
    distinctive.snippet = "zetamorphic content here".to_string();
    distinctive.body = "This email contains zetamorphic subject matter for testing purposes".to_string();

    let mut other = make_email("e-other", "acc-fts", 2000);
    other.subject = "Ordinary email subject".to_string();
    other.body = "Nothing distinctive in this one".to_string();

    db.insert_emails_batch(&[distinctive, other]).unwrap();

    let results = db
        .search_emails("acc-fts", "zetamorphic", None, None, None, None, None, None, None, 10)
        .unwrap();
    let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();
    assert!(
        ids.contains(&"e-distinctive"),
        "FTS must find email with matching keyword"
    );
    assert!(!ids.contains(&"e-other"), "FTS must not return unrelated email");
}

#[test]
fn search_emails_fts_is_account_scoped() {
    let db = test_db();
    db.insert_account(&make_account("acc-a", "a@example.com")).unwrap();
    db.insert_account(&make_account("acc-b", "b@example.com")).unwrap();

    let mut email_a = make_email("e-acc-a", "acc-a", 1000);
    email_a.subject = "Zytoplankton discovery report".to_string();
    email_a.body = "zytoplankton".to_string();

    let mut email_b = make_email("e-acc-b", "acc-b", 1000);
    email_b.subject = "Zytoplankton discovery report".to_string();
    email_b.body = "zytoplankton".to_string();

    db.insert_emails_batch(&[email_a, email_b]).unwrap();

    let results_a = db
        .search_emails("acc-a", "zytoplankton", None, None, None, None, None, None, None, 10)
        .unwrap();
    let results_b = db
        .search_emails("acc-b", "zytoplankton", None, None, None, None, None, None, None, 10)
        .unwrap();

    let ids_a: Vec<&str> = results_a.iter().map(|e| e.id.as_str()).collect();
    let ids_b: Vec<&str> = results_b.iter().map(|e| e.id.as_str()).collect();

    assert!(ids_a.contains(&"e-acc-a"), "acc-a search must return acc-a's email");
    assert!(
        !ids_a.contains(&"e-acc-b"),
        "acc-a search must not return acc-b's email"
    );
    assert!(ids_b.contains(&"e-acc-b"), "acc-b search must return acc-b's email");
    assert!(
        !ids_b.contains(&"e-acc-a"),
        "acc-b search must not return acc-a's email"
    );
}

// ── P0: send service — provider failure path ──────────────────────────────

#[tokio::test]
async fn send_reply_failing_provider_returns_error() {
    let db = test_db();
    db.insert_account(&make_account("acc-fp", "fp@example.com")).unwrap();
    let email = make_email_with("orig-fp", "acc-fp", 1000, "sender@other.com", "inbox");
    db.insert_email(&email).unwrap();

    let result = emailops_lib::services::emails::send_reply_with_provider(
        &db,
        "orig-fp",
        &emailops_lib::sync::provider::EmailBody::plain("reply body"),
        None,
        None,
        None,
        vec![],
        &FailingEmailProvider,
    )
    .await;

    assert!(result.is_err(), "must return error when provider send fails");
}

#[tokio::test]
async fn send_new_email_failing_provider_returns_error() {
    let db = test_db();
    db.insert_account(&make_account("acc-fp2", "fp2@example.com")).unwrap();

    let result = emailops_lib::services::emails::send_new_email_with_provider(
        &db,
        "acc-fp2",
        vec!["to@dest.com".to_string()],
        vec![],
        "Subject",
        &emailops_lib::sync::provider::EmailBody::plain("Body"),
        vec![],
        &FailingEmailProvider,
    )
    .await;

    assert!(result.is_err(), "must return error when provider send fails");
}

// ── P1: sync — list_messages error propagation ────────────────────────────
//
// A provider whose list_messages always fails. Used to verify that sync
// propagates network/API errors rather than silently swallowing them.

struct ListFailingEmailProvider;

#[async_trait::async_trait]
impl EmailProvider for ListFailingEmailProvider {
    async fn get_profile(&self) -> emailops_lib::models::error::Result<(String, String)> {
        Ok(("listfail@example.com".to_string(), "List Fail".to_string()))
    }

    async fn list_messages(
        &self,
        _max_results: u32,
        _page_token: Option<&str>,
        _after_timestamp: Option<i64>,
        _before_timestamp: Option<i64>,
        _label_filter: Option<&str>,
    ) -> emailops_lib::models::error::Result<(Vec<MessageRef>, Option<String>)> {
        Err(AppError::SyncError("deliberate list_messages failure".to_string()))
    }

    async fn get_message(
        &self,
        _message_id: &str,
    ) -> emailops_lib::models::error::Result<(emailops_lib::models::Email, EmailCategory, Vec<AttachmentInfo>)> {
        unimplemented!()
    }

    async fn send_reply(
        &self,
        _from_email: &str,
        _to_emails: &[String],
        _cc_emails: &[String],
        _thread_id: &str,
        _original_message_id: Option<&str>,
        _subject: &str,
        _body: &emailops_lib::sync::provider::EmailBody,
        _attachments: &[EmailAttachment],
    ) -> emailops_lib::models::error::Result<()> {
        unimplemented!()
    }

    async fn send_new_email(
        &self,
        _from_email: &str,
        _to_emails: &[String],
        _cc_emails: &[String],
        _subject: &str,
        _body: &emailops_lib::sync::provider::EmailBody,
        _attachments: &[EmailAttachment],
    ) -> emailops_lib::models::error::Result<()> {
        unimplemented!()
    }

    async fn fetch_attachment_bytes(
        &self,
        _message_id: &str,
        _attachment_id: &str,
    ) -> emailops_lib::models::error::Result<Vec<u8>> {
        unimplemented!()
    }

    async fn list_mailbox_messages(
        &self,
        _mailbox: ExtraMailbox,
        _max_results: u32,
        _after_timestamp: Option<i64>,
        _before_timestamp: Option<i64>,
    ) -> emailops_lib::models::error::Result<Vec<MessageRef>> {
        unimplemented!()
    }
}

#[tokio::test]
async fn sync_list_error_propagates() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-lf", "lf@example.com")).unwrap();
    let account = db.get_account("acc-lf").unwrap().unwrap();

    let (abort_flags, ai_queue) = test_sync_state();
    let result = emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(ListFailingEmailProvider),
    )
    .await;

    assert!(result.is_err(), "sync must propagate list_messages failure");
}

// ── Lenses: runner integration ─────────────────────────────────────────────

/// Backfill is idempotent: running it twice on the same email produces exactly
/// one lens_rows entry. The second run skips already-completed rows without
/// calling the AI provider.
#[tokio::test]
async fn lens_backfill_idempotent() {
    let db = test_db();
    db.insert_account(&make_account("acc-li", "li@example.com")).unwrap();
    db.insert_email(&make_email("e-li-1", "acc-li", 1000)).unwrap();

    let lens = db.create_lens(&make_lens_input("Idempotent lens")).unwrap();

    let fake = Arc::new(FakeAiProvider::new());
    // One canned response: consumed by the first backfill. Second backfill
    // must skip the completed row without calling the AI.
    fake.push_chat_response(r#"{"amount": 42.0}"#);

    emailops_lib::services::lenses::runner::backfill_lens(db.clone(), fake.clone(), lens.id.clone(), None, None)
        .await
        .expect("first backfill");

    emailops_lib::services::lenses::runner::backfill_lens(db.clone(), fake.clone(), lens.id.clone(), None, None)
        .await
        .expect("second backfill");

    let page = db.get_lens_rows(&lens.id, None, 50, 0).unwrap();
    assert_eq!(page.rows.len(), 1, "idempotent backfill must not duplicate rows");
    assert_eq!(page.rows[0].email_id, "e-li-1");
    assert_eq!(page.rows[0].status, "ok");
}

/// on_emails_synced processes newly synced email IDs against enabled lenses
/// and creates lens_rows for matching emails.
#[tokio::test]
async fn lens_on_emails_synced_extracts_matching() {
    let db = test_db();
    db.insert_account(&make_account("acc-los", "los@example.com")).unwrap();
    db.insert_email(&make_email("e-los-1", "acc-los", 1000)).unwrap();

    let lens = db.create_lens(&make_lens_input("Sync hook lens")).unwrap();

    let fake = Arc::new(FakeAiProvider::new());
    fake.push_chat_response(r#"{"amount": 99.0}"#);

    let n = emailops_lib::services::lenses::runner::on_emails_synced(
        db.clone(),
        fake.clone(),
        &["e-los-1".to_string()],
        None,
    )
    .await
    .expect("on_emails_synced");

    assert_eq!(n, 1, "one extraction must be performed");
    assert!(
        db.lens_row_exists(&lens.id, "e-los-1").unwrap(),
        "lens row must be persisted after sync hook"
    );
}

/// Deleting a lens must also remove its rows, runs, and exclusions. The explicit
/// transaction in delete_lens replaces the missing ON DELETE CASCADE.
#[tokio::test]
async fn lens_delete_cascades_rows() {
    let db = test_db();
    db.insert_account(&make_account("acc-ldc", "ldc@example.com")).unwrap();
    db.insert_email(&make_email("e-ldc-1", "acc-ldc", 1000)).unwrap();

    let lens = db.create_lens(&make_lens_input("Delete cascade lens")).unwrap();
    let lens_id = lens.id.clone();

    let fake = Arc::new(FakeAiProvider::new());
    fake.push_chat_response(r#"{"amount": 7.0}"#);

    emailops_lib::services::lenses::runner::backfill_lens(db.clone(), fake.clone(), lens_id.clone(), None, None)
        .await
        .expect("backfill");

    // Confirm row was created before deletion.
    assert!(
        db.lens_row_exists(&lens_id, "e-ldc-1").unwrap(),
        "row must exist before delete"
    );

    db.delete_lens(&lens_id).expect("delete_lens");

    // The lens itself must be gone.
    assert!(
        db.list_lenses().unwrap().is_empty(),
        "lens must be removed from lenses table"
    );
    // The rows must be cleaned up by the explicit DELETE in delete_lens.
    let page = db.get_lens_rows(&lens_id, None, 50, 0).unwrap();
    assert!(page.rows.is_empty(), "lens_rows must be removed when lens is deleted");
}

// ── Lenses: regression — category case-insensitivity in on_emails_synced ──
//
// Sync writes email.category lowercase ("primary", "updates"). The Lens scope
// editor sends capitalized names ("Primary", "Updates"). email_matches_scope
// (used by on_emails_synced) was doing a case-sensitive contains() check,
// so inbox emails were silently skipped on every sync hook call.
// scope::evaluate (used by backfill) was already correct because it uses
// LOWER(e.category). This regression test pins the fix.

/// on_emails_synced must process an email whose DB-stored category is lowercase
/// ("primary") when the Lens scope uses the UI-capitalized form ("Primary").
#[tokio::test]
async fn on_emails_synced_category_case_insensitive() {
    use emailops_lib::models::lens::Direction;

    let db = test_db();
    db.insert_account(&make_account("acc-cat", "cat@example.com")).unwrap();

    // Sync writes category as lowercase — this is the real production state.
    // Use a timestamp close to "now" so the test isn't sensitive to the wall clock.
    let recent_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 86_400; // yesterday
    let mut email = make_email("e-cat-1", "acc-cat", recent_ts);
    email.category = "primary".to_string();
    email.mailbox = "inbox".to_string();
    db.insert_email(&email).unwrap();

    // Scope uses the UI-capitalized form sent from the frontend.
    // No date_range so the test only exercises category case-sensitivity.
    let input = CreateLensInput {
        name: "Category case test".to_string(),
        icon: None,
        template_key: None,
        account_id: None,
        scope: LensScope {
            categories: Some(vec!["Primary".to_string()]),
            mailboxes: Some(vec!["inbox".to_string()]),
            direction: Some(Direction::Inbound),
            ..Default::default()
        },
        schema: emailops_lib::models::lens::LensSchema {
            columns: vec![LensColumn {
                key: "amount".to_string(),
                label: "Amount".to_string(),
                column_type: LensColumnType::Number,
                description: "Amount".to_string(),
                enum_values: None,
                required: false,
                is_unique_key: false,
            }],
        },
        prompt_text: "Extract amount.".to_string(),
        model_provider: None,
        model_name: None,
    };
    let lens = db.create_lens(&input).unwrap();

    let fake = Arc::new(FakeAiProvider::new());
    fake.push_chat_response(r#"{"amount": 5.0}"#);

    let n = emailops_lib::services::lenses::runner::on_emails_synced(db.clone(), fake, &["e-cat-1".to_string()], None)
        .await
        .expect("on_emails_synced");

    assert_eq!(
        n, 1,
        "on_emails_synced must process the email even though scope uses 'Primary' \
         and DB stores 'primary'"
    );
    assert!(
        db.lens_row_exists(&lens.id, "e-cat-1").unwrap(),
        "lens row must be created for the email"
    );
}

// ── P1: tag filter ─────────────────────────────────────────────────────────

/// get_filtered_emails with tag_type+tag_value returns only emails carrying
/// that tag. Untagged emails in the same account must not appear.
#[test]
fn get_filtered_emails_by_tag_returns_only_tagged() {
    let db = test_db();
    db.insert_account(&make_account("acc-tag", "tag@example.com")).unwrap();
    let tagged = make_email_with("e-tagged", "acc-tag", 2000, "a@x.com", "inbox");
    let untagged = make_email_with("e-untagged", "acc-tag", 1000, "b@y.com", "inbox");
    db.insert_email(&tagged).unwrap();
    db.insert_email(&untagged).unwrap();

    db.upsert_email_tag("e-tagged", "priority", "urgent", None).unwrap();

    let result = db
        .get_filtered_emails("acc-tag", None, None, Some("priority"), Some("urgent"), None, 50, 0)
        .unwrap();

    let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
    assert!(
        ids.contains(&"e-tagged"),
        "tagged email must appear in filtered results"
    );
    assert!(
        !ids.contains(&"e-untagged"),
        "untagged email must not appear in filtered results"
    );
}

// ── P1: send validation ────────────────────────────────────────────────────

/// `send_new_email_with_provider` must return an error immediately when the
/// recipient list is empty — before the provider is contacted. This prevents
/// sending a blank-recipient email to the SMTP/Gmail API which would either
/// silently drop it or return an ambiguous error.
#[tokio::test]
async fn send_new_email_empty_to_returns_validation_error() {
    let db = test_db();
    db.insert_account(&make_account("acc-val", "val@example.com")).unwrap();
    let provider = FakeEmailProvider::new("val@example.com", "Val");

    let result = emailops_lib::services::emails::send_new_email_with_provider(
        &db,
        "acc-val",
        vec![], // ← empty To
        vec![],
        "Hello",
        &emailops_lib::sync::provider::EmailBody::plain("body"),
        vec![],
        &provider,
    )
    .await;

    assert!(result.is_err(), "empty To must return an error");
    // Provider must NOT have been called — validation fires before any I/O.
    assert!(
        provider.sent().is_empty(),
        "provider must not be called when To is empty"
    );
}

/// A Subject with an embedded newline must be rejected to prevent SMTP header
/// injection. The check fires before the provider is contacted.
#[tokio::test]
async fn send_new_email_newline_in_subject_returns_validation_error() {
    let db = test_db();
    db.insert_account(&make_account("acc-inj", "inj@example.com")).unwrap();
    let provider = FakeEmailProvider::new("inj@example.com", "Inj");

    let injected_subject = "Legitimate subject\r\nBcc: attacker@evil.com";
    let result = emailops_lib::services::emails::send_new_email_with_provider(
        &db,
        "acc-inj",
        vec!["to@dest.com".to_string()],
        vec![],
        injected_subject,
        &emailops_lib::sync::provider::EmailBody::plain("body"),
        vec![],
        &provider,
    )
    .await;

    assert!(result.is_err(), "subject with newline must be rejected");
    assert!(
        provider.sent().is_empty(),
        "provider must not be called when subject contains newline"
    );
}

// ── P1: filter — no-filter (AllInbox) baseline ────────────────────────────

/// `get_filtered_emails` with no domain/sender/tag filter returns all inbox
/// emails for the account. This is the "AllInbox" baseline: every filter field
/// is None so no WHERE condition restricts the result set beyond account scope
/// and `is_deleted = 0`.
#[test]
fn get_filtered_emails_no_filter_returns_all_inbox_emails() {
    let db = test_db();
    db.insert_account(&make_account("acc-all", "all@example.com")).unwrap();

    db.insert_email(&make_email_with("e1", "acc-all", 3000, "a@x.com", "inbox"))
        .unwrap();
    db.insert_email(&make_email_with("e2", "acc-all", 2000, "b@y.com", "inbox"))
        .unwrap();
    db.insert_email(&make_email_with("e3", "acc-all", 1000, "c@z.com", "inbox"))
        .unwrap();
    // A different account's email must NOT appear.
    db.insert_account(&make_account("acc-other", "other@example.com"))
        .unwrap();
    db.insert_email(&make_email_with("e-other", "acc-other", 4000, "x@w.com", "inbox"))
        .unwrap();

    let result = db
        .get_filtered_emails("acc-all", None, None, None, None, None, 50, 0)
        .unwrap();

    let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"e1"), "e1 must be in all-inbox result");
    assert!(ids.contains(&"e2"), "e2 must be in all-inbox result");
    assert!(ids.contains(&"e3"), "e3 must be in all-inbox result");
    assert!(!ids.contains(&"e-other"), "other-account email must not appear");
}

/// `get_filtered_emails` must not return soft-deleted emails even without a
/// filter. Deleted rows stay in the DB (for dedup) but must be hidden from every
/// query path.
#[test]
fn get_filtered_emails_no_filter_excludes_soft_deleted() {
    let db = test_db();
    db.insert_account(&make_account("acc-del", "del@example.com")).unwrap();
    db.insert_email(&make_email_with("e-live", "acc-del", 2000, "a@x.com", "inbox"))
        .unwrap();
    db.insert_email(&make_email_with("e-gone", "acc-del", 1000, "b@y.com", "inbox"))
        .unwrap();
    db.delete_email("e-gone").unwrap();

    let result = db
        .get_filtered_emails("acc-del", None, None, None, None, None, 50, 0)
        .unwrap();

    let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"e-live"), "live email must appear");
    assert!(!ids.contains(&"e-gone"), "soft-deleted email must NOT appear");
}

// ── P1: contact name canonicalization ──────────────────────────────────────

/// When the same sender address appears with different display names across
/// multiple emails, `get_contacts` must surface exactly one contact entry for
/// that address. SQLite's `MAX(sender)` picks the alphabetically last name —
/// this test pins that behaviour so a future refactor (e.g. most-frequent name)
/// is an intentional change, not an accidental regression.
#[test]
fn get_contacts_deduplicates_same_email_different_display_names() {
    let db = test_db();
    db.insert_account(&make_account("acc-ct", "ct@example.com")).unwrap();

    // Same sender_email, three different display names.
    let mut e1 = make_email_with("e-ct1", "acc-ct", 1000, "alice@corp.com", "inbox");
    e1.sender = "Alice A".to_string();
    let mut e2 = make_email_with("e-ct2", "acc-ct", 2000, "alice@corp.com", "inbox");
    e2.sender = "Alice B".to_string();
    let mut e3 = make_email_with("e-ct3", "acc-ct", 3000, "alice@corp.com", "inbox");
    e3.sender = "Alice Z".to_string();
    db.insert_email(&e1).unwrap();
    db.insert_email(&e2).unwrap();
    db.insert_email(&e3).unwrap();

    let contacts = db.get_contacts("acc-ct").unwrap();

    // Only one entry for alice@corp.com.
    let alice_entries: Vec<_> = contacts.iter().filter(|c| c.email == "alice@corp.com").collect();
    assert_eq!(
        alice_entries.len(),
        1,
        "same sender_email must appear only once in contacts"
    );
    assert_eq!(alice_entries[0].email_count, 3, "email_count must reflect all 3 emails");
    // MAX(sender) = "Alice Z" (alphabetically last). Pin the canonical name.
    assert_eq!(
        alice_entries[0].name, "Alice Z",
        "canonical name must be the MAX(sender) across all emails for this address"
    );
}

// ── Lenses: failure-mode regression tests ─────────────────────────────────
//
// These tests pin the exact conditions that produce "last run failed" in the UI
// vs. "complete with zero rows" vs. "complete with some failed rows". They were
// written to reproduce the user's issue: the sidebar showing a permanent "failed"
// badge with no logs explaining why.
//
// Key invariants:
//   - Run status = "failed"   → only when there is a DB error or an orphan reset
//   - Run status = "complete" → scope matches nothing (0 rows) OR AI fails per-row
//   - Row status  = "failed"  → AI returned an unusable response for that email
//
// The most common cause of a "failed" run badge with no log: the app was quit
// while a run was in progress. reset_orphan_lens_runs() marks it "failed" at
// next startup (now logged as a visible warning in the output panel).

/// Scope that finds no matching emails produces a clean run: status="complete",
/// 0 rows, 0 succeeded, 0 failed. The run must NOT show status="failed".
#[tokio::test]
async fn backfill_scope_no_matches_completes_with_zero_rows() {
    let db = test_db();
    // Account and email exist but scope filters to a different account.
    db.insert_account(&make_account("acc-real", "real@example.com"))
        .unwrap();
    db.insert_email(&make_email("e-real", "acc-real", 1000)).unwrap();

    let mut input = make_lens_input("No-match lens");
    input.scope.account_ids = Some(vec!["nonexistent-account-id".to_string()]);
    let lens = db.create_lens(&input).unwrap();

    let fake = Arc::new(FakeAiProvider::new()); // no responses queued — should not be called

    emailops_lib::services::lenses::runner::backfill_lens(db.clone(), fake.clone(), lens.id.clone(), None, None)
        .await
        .expect("backfill must succeed even with zero scope matches");

    // Run must be "complete", not "failed".
    let runs = db.list_lens_runs(&lens.id, 5).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].status, "complete",
        "run status must be 'complete' when scope matches nothing — not 'failed'"
    );
    assert_eq!(runs[0].processed, 0);
    assert_eq!(runs[0].succeeded, 0);
    assert_eq!(runs[0].failed, 0);

    let page = db.get_lens_rows(&lens.id, None, 50, 0).unwrap();
    assert!(
        page.rows.is_empty(),
        "no rows must be created when scope matches nothing"
    );

    // AI must not have been called at all.
    assert!(
        fake.chat_calls().is_empty(),
        "AI must not be called when scope is empty"
    );
}

/// When the AI returns an empty/invalid response for every email, each row gets
/// status="failed" but the run itself finishes as "complete" (not "failed").
/// This is intentional: per-email AI failures are soft — we log them and move on
/// so one bad email doesn't abort the whole backfill.
#[tokio::test]
async fn backfill_ai_failure_per_row_run_still_completes() {
    let db = test_db();
    db.insert_account(&make_account("acc-ai-fail", "ai@example.com"))
        .unwrap();
    db.insert_email(&make_email("e-ai-fail", "acc-ai-fail", 1000)).unwrap();

    let lens = db.create_lens(&make_lens_input("AI-failure lens")).unwrap();

    // FakeAiProvider with NO queued responses: chat_with_tools returns empty
    // content + no tool_calls → extractor returns ExtractionStatus::Failed.
    let fake = Arc::new(FakeAiProvider::new());

    emailops_lib::services::lenses::runner::backfill_lens(db.clone(), fake.clone(), lens.id.clone(), None, None)
        .await
        .expect("backfill must succeed even when AI fails on every row");

    // Run must be "complete" — AI failures are per-row, not fatal to the run.
    let runs = db.list_lens_runs(&lens.id, 5).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].status, "complete",
        "run status must be 'complete' even when AI fails on all rows — \
         per-row AI failures must NOT flip the run to 'failed'"
    );
    assert_eq!(runs[0].processed, 1, "one email was attempted");
    assert_eq!(runs[0].succeeded, 0);
    assert_eq!(runs[0].failed, 1, "the one attempted row must be counted as failed");

    // The row must exist in lens_rows with status="failed" so it is retried on
    // the next backfill run. `get_lens_rows` intentionally filters to status='ok'
    // (the spreadsheet view only shows successfully-extracted rows), so we use
    // `lens_row_exists` to confirm the DB row was actually written.
    assert!(
        db.lens_row_exists(&lens.id, "e-ai-fail").unwrap(),
        "a lens_row must be created even on AI failure so the email is retried on next backfill"
    );
    // Failed rows must remain eligible for retry (lens_row_completed_or_excluded
    // returns false for them), which is what makes the next backfill pick them back up.
    assert!(
        !db.lens_row_completed_or_excluded(&lens.id, "e-ai-fail").unwrap(),
        "failed rows must not be considered 'completed' — they must be retried next run"
    );
    // The spreadsheet view (get_lens_rows) hides failed rows — only 'ok' rows appear.
    let page = db.get_lens_rows(&lens.id, None, 50, 0).unwrap();
    assert_eq!(
        page.rows.len(),
        0,
        "failed rows must not appear in the spreadsheet view"
    );
}

/// When a scope includes an FTS keyword query, emails whose body/subject does
/// not contain the terms are filtered out at the scope-evaluation stage. The
/// run must complete with "complete" and 0 rows — NOT "failed".
///
/// This is the shape of the user's Lens (scope had
/// `query = "invoice OR receipt OR factura"` but some emails don't match).
#[tokio::test]
async fn backfill_fts_scope_skips_non_matching_emails() {
    use emailops_lib::models::lens::LensScope;

    let db = test_db();
    db.insert_account(&make_account("acc-fts-lens", "fts@example.com"))
        .unwrap();

    // Insert two emails using batch so FTS index is populated.
    let mut matching = make_email("e-fts-match", "acc-fts-lens", 2000);
    matching.subject = "Invoice from Acme".to_string();
    matching.body = "Please find attached invoice INV-001.".to_string();

    let mut non_matching = make_email("e-fts-skip", "acc-fts-lens", 1000);
    non_matching.subject = "Meeting tomorrow".to_string();
    non_matching.body = "Let us sync at 3pm.".to_string();

    db.insert_emails_batch(&[matching, non_matching]).unwrap();

    let mut input = make_lens_input("FTS scope lens");
    input.scope = LensScope {
        query: Some("invoice OR receipt OR factura".to_string()),
        ..Default::default()
    };
    let lens = db.create_lens(&input).unwrap();

    let fake = Arc::new(FakeAiProvider::new());
    // One response for the one email that matches the FTS query.
    fake.push_chat_response(r#"{"amount": 100.0}"#);

    emailops_lib::services::lenses::runner::backfill_lens(db.clone(), fake.clone(), lens.id.clone(), None, None)
        .await
        .expect("backfill");

    let runs = db.list_lens_runs(&lens.id, 5).unwrap();
    assert_eq!(runs[0].status, "complete");
    assert_eq!(runs[0].processed, 1, "only the FTS-matching email should be processed");
    assert_eq!(runs[0].succeeded, 1);

    let page = db.get_lens_rows(&lens.id, None, 50, 0).unwrap();
    assert_eq!(page.rows.len(), 1, "only the invoice email must produce a row");
    assert_eq!(page.rows[0].email_id, "e-fts-match");

    // AI must NOT have been called for the non-matching email.
    assert_eq!(
        fake.chat_calls().len(),
        1,
        "AI must be called exactly once (for the FTS-matching email only)"
    );
}

/// Cancelling a run mid-flight sets status="cancelled", not "failed".
/// The cancel flag is checked between emails. With a single email the run
/// finishes that email then exits cleanly with "cancelled".
#[tokio::test]
async fn backfill_cancelled_sets_status_cancelled_not_failed() {
    use std::sync::atomic::AtomicBool;

    let db = test_db();
    db.insert_account(&make_account("acc-cancel", "cancel@example.com"))
        .unwrap();
    db.insert_email(&make_email("e-cancel", "acc-cancel", 1000)).unwrap();

    let lens = db.create_lens(&make_lens_input("Cancel test lens")).unwrap();

    let fake = Arc::new(FakeAiProvider::new());
    fake.push_chat_response(r#"{"amount": 1.0}"#);

    // Set cancel flag before the run starts — the run checks it between emails.
    // With a single email the run processes it first, then checks the flag and exits.
    let cancel = Arc::new(AtomicBool::new(true));

    emailops_lib::services::lenses::runner::backfill_lens(
        db.clone(),
        fake.clone(),
        lens.id.clone(),
        None,
        Some(cancel),
    )
    .await
    .expect("backfill");

    let runs = db.list_lens_runs(&lens.id, 5).unwrap();
    assert_eq!(
        runs[0].status, "cancelled",
        "cancelled run must show 'cancelled', not 'failed'"
    );
}

// ── Extra mailbox sync (Sent / Spam / Trash) ───────────────────────────────
//
// Regression coverage for the production bug where Sent emails between
// 2024-10 and 2025-12 silently never landed in the DB because:
//   1. `ExtraMailbox::all()` did not include `Sent`, so it had no dedicated
//      pass — sent mail relied entirely on the 500-cap inbox query.
//   2. When inbox had no new emails, `sync_account_with_provider`
//      early-returned before reaching `sync_extra_mailboxes`, so even with
//      the new Sent pass enabled a steady-state account would never run it.
//   3. The backfill cursor anchored at the oldest stored timestamp, which
//      skipped gaps inside `[oldest, latest]` — a real-user mailbox with
//      2018 + 2026 sent emails would NEVER reach the missing 2025 ones.
//
// These tests pin those three behaviors down.

/// Build a Sent-mailbox test email. The sender MUST match the account's own
/// email — `get_emails(mailbox="sent")` matches on
/// `sender_email = accounts.email`, so anything else is invisible to that
/// query even though the row is in `emails` with `mailbox='sent'`.
fn sent_email(id: &str, account_id: &str, account_email: &str, timestamp: i64) -> Email {
    make_email_with(id, account_id, timestamp, account_email, "sent")
}

fn spam_email(id: &str, account_id: &str, timestamp: i64) -> Email {
    make_email_with(id, account_id, timestamp, "junk@spam.com", "spam")
}

fn trash_email(id: &str, account_id: &str, timestamp: i64) -> Email {
    make_email_with(id, account_id, timestamp, "deleted@x.com", "trash")
}

/// IDs of rows for an account in the given mailbox, queried directly via the
/// `mailbox` column. Tests use this instead of `db.get_emails(.., Some("sent"))`
/// because the "sent view" filters by `sender_email = accounts.email`, which
/// projects out rows whose sender doesn't match. We're verifying the sync
/// layer's storage, not the view layer's projection.
fn ids_in_mailbox(db: &Database, account_id: &str, mailbox: &str) -> std::collections::HashSet<String> {
    use rusqlite::params;
    let conn = db.reader();
    let mut stmt = conn
        .prepare(
            "SELECT id FROM emails
             WHERE account_id = ?1 AND mailbox = ?2 AND is_deleted = 0",
        )
        .expect("prepare ids_in_mailbox");
    stmt.query_map(params![account_id, mailbox], |row| row.get::<_, String>(0))
        .expect("query ids_in_mailbox")
        .filter_map(|r| r.ok())
        .collect()
}

#[test]
fn extra_mailbox_all_includes_sent_spam_trash() {
    let all = ExtraMailbox::all();
    assert!(
        all.contains(&ExtraMailbox::Sent),
        "Sent MUST be in ExtraMailbox::all — without it, sent mail relies on the capped inbox pass"
    );
    assert!(all.contains(&ExtraMailbox::Spam));
    assert!(all.contains(&ExtraMailbox::Trash));
}

#[tokio::test]
async fn sync_ingests_sent_via_dedicated_pass() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-sent", "sent@example.com"))
        .unwrap();
    let account = db.get_account("acc-sent").unwrap().unwrap();

    let provider = FakeEmailProvider::new("sent@example.com", "Sent");
    // No inbox messages — the main pass is a no-op.
    provider.add_message(
        sent_email("s-1", "acc-sent", "sent@example.com", 1000),
        EmailCategory::Primary,
        vec![],
    );
    provider.add_message(
        sent_email("s-2", "acc-sent", "sent@example.com", 2000),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync_account_with_provider");

    // Both sent emails must be stored with mailbox = "sent".
    let ids = ids_in_mailbox(&db, "acc-sent", "sent");
    assert!(ids.contains("s-1"), "s-1 must be stored: {:?}", ids);
    assert!(ids.contains("s-2"), "s-2 must be stored: {:?}", ids);
}

#[tokio::test]
async fn sync_extra_mailboxes_runs_when_inbox_has_no_new_emails() {
    // Regression: when inbox is "up to date" (no new emails), the early
    // return must NOT skip the Sent / Spam / Trash pass — that was the
    // production bug where a steady-state account never recovered missing
    // sent mail.
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-idle", "idle@example.com"))
        .unwrap();
    let account = db.get_account("acc-idle").unwrap().unwrap();

    let provider = FakeEmailProvider::new("idle@example.com", "Idle");
    // Zero inbox messages, but the user has a sent email that needs syncing.
    provider.add_message(
        sent_email("s-only", "acc-idle", "idle@example.com", 5000),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync_account_with_provider");

    let ids = ids_in_mailbox(&db, "acc-idle", "sent");
    assert!(
        ids.contains("s-only"),
        "extra mailbox sync MUST run even when inbox has zero new emails — got {:?}",
        ids
    );
}

#[tokio::test]
async fn sync_with_no_new_emails_emits_no_up_to_date_log() {
    // When nothing new is downloaded, the output panel should stay quiet —
    // no "All messages already synced" / "nothing new" line. Those lines were
    // pure noise on idle syncs.
    let logger = emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-quiet", "quiet@example.com"))
        .unwrap();
    let account = db.get_account("acc-quiet").unwrap().unwrap();

    // Provider with no messages at all → new_count == 0.
    let provider = FakeEmailProvider::new("quiet@example.com", "Quiet");

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync_account_with_provider");

    let noisy: Vec<_> = logger
        .events()
        .into_iter()
        .filter(|e| {
            e.source == "sync"
                && (e.message.contains("already synced")
                    || e.message.contains("nothing new")
                    || e.message.contains("up to date"))
        })
        .collect();
    assert!(
        noisy.is_empty(),
        "idle sync must not emit up-to-date / nothing-new log lines, got: {:?}",
        noisy
    );
    emailops_lib::services::logger::install(std::sync::Arc::new(emailops_lib::services::logger::NoopLogger));
}

#[tokio::test]
async fn sync_ingests_spam_and_trash_into_correct_mailboxes() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-st", "st@example.com")).unwrap();
    let account = db.get_account("acc-st").unwrap().unwrap();

    let provider = FakeEmailProvider::new("st@example.com", "St");
    provider.add_message(spam_email("sp-1", "acc-st", 1000), EmailCategory::Primary, vec![]);
    provider.add_message(trash_email("tr-1", "acc-st", 2000), EmailCategory::Primary, vec![]);

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync_account_with_provider");

    let spam_ids = ids_in_mailbox(&db, "acc-st", "spam");
    let trash_ids = ids_in_mailbox(&db, "acc-st", "trash");
    assert!(spam_ids.contains("sp-1"), "spam mailbox must contain sp-1");
    assert!(trash_ids.contains("tr-1"), "trash mailbox must contain tr-1");
}

#[tokio::test]
async fn extra_mailbox_forward_watermark_advances_and_filters_next_sync() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-wm", "wm@example.com")).unwrap();
    let account = db.get_account("acc-wm").unwrap().unwrap();

    // First sync: provider has one sent email at ts=1000.
    {
        let provider = FakeEmailProvider::new("wm@example.com", "Wm");
        provider.add_message(
            sent_email("s-old", "acc-wm", "wm@example.com", 1000),
            EmailCategory::Primary,
            vec![],
        );
        let (abort_flags, ai_queue) = test_sync_state();
        emailops_lib::services::emails::sync_account_with_provider(
            &db,
            &account,
            std::path::Path::new("/tmp"),
            None,
            ai_queue,
            abort_flags,
            Box::new(provider),
        )
        .await
        .expect("first sync");
    }

    // Watermark for Sent must now be at 1000 (newest ingested ts).
    let watermark = db
        .get_preference("extra_mailbox_sync:acc-wm:sent")
        .unwrap()
        .and_then(|s| s.parse::<i64>().ok());
    assert_eq!(
        watermark,
        Some(1000),
        "forward watermark for Sent must equal newest ingested timestamp"
    );

    // Second sync: provider returns the same old + one new at ts=5000.
    // The forward pass uses `after_timestamp > watermark`, so the old one
    // must NOT be returned again, and only s-new is freshly inserted.
    {
        let provider = FakeEmailProvider::new("wm@example.com", "Wm");
        provider.add_message(
            sent_email("s-old", "acc-wm", "wm@example.com", 1000),
            EmailCategory::Primary,
            vec![],
        );
        provider.add_message(
            sent_email("s-new", "acc-wm", "wm@example.com", 5000),
            EmailCategory::Primary,
            vec![],
        );
        let (abort_flags, ai_queue) = test_sync_state();
        emailops_lib::services::emails::sync_account_with_provider(
            &db,
            &account,
            std::path::Path::new("/tmp"),
            None,
            ai_queue,
            abort_flags,
            Box::new(provider),
        )
        .await
        .expect("second sync");
    }

    let ids = ids_in_mailbox(&db, "acc-wm", "sent");
    assert!(ids.contains("s-old"), "s-old must be present: {:?}", ids);
    assert!(ids.contains("s-new"), "s-new must be present: {:?}", ids);

    let watermark2 = db
        .get_preference("extra_mailbox_sync:acc-wm:sent")
        .unwrap()
        .and_then(|s| s.parse::<i64>().ok());
    assert_eq!(
        watermark2,
        Some(5000),
        "forward watermark must advance to newest ingested timestamp on each sync"
    );
}

#[tokio::test]
async fn extra_mailbox_backfill_fills_interior_gap() {
    // The actual production-impacting bug shape:
    // DB already has Sent emails at ts=1 (very old) and ts=NOW.
    // Provider has a middle Sent email at ts=500_000 that's missing from DB.
    // Forward incremental cannot fix this (watermark is already at NOW), so
    // backfill MUST find the interior-gap message by walking from NOW down.
    //
    // The cursor design is the load-bearing piece: anchoring backfill at the
    // oldest stored ts would IMMEDIATELY skip below ts=1, miss the entire
    // interior, and mark done.
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-gap", "gap@example.com")).unwrap();
    let account = db.get_account("acc-gap").unwrap().unwrap();

    // Pre-populate DB with the boundary sent emails (old + recent).
    let recent_ts: i64 = chrono::Utc::now().timestamp();
    db.insert_email(&sent_email("s-2018", "acc-gap", "gap@example.com", 1))
        .unwrap();
    db.insert_email(&sent_email("s-2026", "acc-gap", "gap@example.com", recent_ts))
        .unwrap();

    // Set the forward watermark to the recent ts so forward incremental
    // returns nothing — backfill must be the path that finds the gap.
    db.set_preference("extra_mailbox_sync:acc-gap:sent", &recent_ts.to_string())
        .unwrap();

    // Provider has all three — the missing 2025-shaped one in the middle.
    let provider = FakeEmailProvider::new("gap@example.com", "Gap");
    provider.add_message(
        sent_email("s-2018", "acc-gap", "gap@example.com", 1),
        EmailCategory::Primary,
        vec![],
    );
    let gap_ts = 500_000_i64;
    provider.add_message(
        sent_email("s-gap", "acc-gap", "gap@example.com", gap_ts),
        EmailCategory::Primary,
        vec![],
    );
    provider.add_message(
        sent_email("s-2026", "acc-gap", "gap@example.com", recent_ts),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync");

    let ids = ids_in_mailbox(&db, "acc-gap", "sent");
    assert!(
        ids.contains("s-gap"),
        "backfill MUST pick up the interior-gap email — got {:?}",
        ids
    );
    assert!(ids.contains("s-2018"));
    assert!(ids.contains("s-2026"));
}

#[tokio::test]
async fn extra_mailbox_backfill_done_flag_skips_repeat_scans() {
    // Once the backfill has reached the start of mailbox history (empty
    // page from provider), the done flag is set to "1" and subsequent
    // syncs must NOT walk the history again — that would waste API quota.
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-done", "done@example.com"))
        .unwrap();
    let account = db.get_account("acc-done").unwrap().unwrap();

    // Provider has one sent email.
    let provider = FakeEmailProvider::new("done@example.com", "Done");
    provider.add_message(
        sent_email("s-only", "acc-done", "done@example.com", 1000),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("first sync");

    // After one sync that ingested everything available, the backfill
    // should have reached an empty page and marked done.
    let done = db.get_preference("extra_mailbox_backfill:acc-done:sent").unwrap();
    assert_eq!(
        done.as_deref(),
        Some("1"),
        "backfill done flag must be set once provider returns empty page"
    );
}

#[tokio::test]
async fn extra_mailbox_backfill_respects_sync_from_floor() {
    // If the account configures `sync_from_timestamp = T_floor`, the
    // backfill must stop at T_floor and not walk older history. This is the
    // user's "don't bother fetching pre-2020 mail" knob.
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-floor", "floor@example.com"))
        .unwrap();

    // Set sync_from_timestamp = 1_000_000.
    db.update_account_sync_from("acc-floor", Some(1_000_000)).unwrap();
    let account = db.get_account("acc-floor").unwrap().unwrap();

    let provider = FakeEmailProvider::new("floor@example.com", "Floor");
    // Pre-floor sent email — the forward pass on first sync still pulls it
    // (the floor only constrains backfill walking). The point of this test
    // is the BACKFILL stops walking once the cursor crosses the floor.
    provider.add_message(
        sent_email("s-old", "acc-floor", "floor@example.com", 100),
        EmailCategory::Primary,
        vec![],
    );
    // Above the floor — must be ingested.
    provider.add_message(
        sent_email("s-above", "acc-floor", "floor@example.com", 2_000_000),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync");

    // Forward pass also uses no-watermark on first run, so both refs would
    // be returned by `list_mailbox_messages(after=None)`. The forward pass
    // ingests them all without filtering by floor — but the backfill done
    // flag and the floor check together must STILL prevent infinite loops
    // and pre-floor walks. Verify the floor is honored at the done-flag
    // level: once the cursor crosses below the floor, backfill must mark
    // done. We assert s-above is present (forward) and that done is "1".
    let ids = ids_in_mailbox(&db, "acc-floor", "sent");
    assert!(ids.contains("s-above"), "above-floor email must be ingested");

    let done = db.get_preference("extra_mailbox_backfill:acc-floor:sent").unwrap();
    assert_eq!(
        done.as_deref(),
        Some("1"),
        "backfill must mark done so we don't keep walking below the floor"
    );
}

#[tokio::test]
async fn extra_mailbox_backfill_cursor_persists_between_calls() {
    // The backfill cursor pref must be persisted each iteration so that
    // partial progress survives across sync calls. We can't easily force
    // a multi-page descent without raising MAX_BACKFILL_PAGES_PER_SYNC, but
    // we CAN verify the cursor pref exists after a sync that ingested
    // anything via backfill.
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-cur", "cur@example.com")).unwrap();
    let account = db.get_account("acc-cur").unwrap().unwrap();

    let provider = FakeEmailProvider::new("cur@example.com", "Cur");
    provider.add_message(
        sent_email("s-1", "acc-cur", "cur@example.com", 1000),
        EmailCategory::Primary,
        vec![],
    );
    provider.add_message(
        sent_email("s-2", "acc-cur", "cur@example.com", 2000),
        EmailCategory::Primary,
        vec![],
    );

    let (abort_flags, ai_queue) = test_sync_state();
    emailops_lib::services::emails::sync_account_with_provider(
        &db,
        &account,
        std::path::Path::new("/tmp"),
        None,
        ai_queue,
        abort_flags,
        Box::new(provider),
    )
    .await
    .expect("sync");

    let cursor = db.get_preference("extra_mailbox_backfill_cursor:acc-cur:sent").unwrap();
    // The cursor will have advanced to the min timestamp of returned refs
    // (1000) on the first iteration, then the second iteration's empty
    // page set the done flag — so cursor must exist and be <= 1000.
    let cursor_val = cursor.and_then(|s| s.parse::<i64>().ok());
    assert!(
        matches!(cursor_val, Some(ts) if ts <= 1000),
        "backfill cursor must be persisted at or below oldest provider timestamp (got {:?})",
        cursor_val
    );
}

#[tokio::test]
async fn resync_mailbox_full_recovers_gap_and_returns_delta() {
    // The manual recovery path used by the `start_resync_mailbox` Tauri
    // command. It must:
    //   - Reset the per-mailbox backfill done flag and cursor.
    //   - Walk the full history (no `max_pages` cap).
    //   - Return the count of newly inserted emails.
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-rec", "rec@example.com")).unwrap();
    let account = db.get_account("acc-rec").unwrap().unwrap();

    // Pre-state: backfill was previously marked done (simulating the old
    // forward-only sync that never picked up Sent).
    db.set_preference("extra_mailbox_backfill:acc-rec:sent", "1").unwrap();
    // One sent email already present (the user's 2026 emails).
    db.insert_email(&sent_email("s-existing", "acc-rec", "rec@example.com", 9999))
        .unwrap();

    // Provider has the existing one PLUS three new ones to recover.
    let provider = FakeEmailProvider::new("rec@example.com", "Rec");
    provider.add_message(
        sent_email("s-existing", "acc-rec", "rec@example.com", 9999),
        EmailCategory::Primary,
        vec![],
    );
    provider.add_message(
        sent_email("s-rec-1", "acc-rec", "rec@example.com", 100),
        EmailCategory::Primary,
        vec![],
    );
    provider.add_message(
        sent_email("s-rec-2", "acc-rec", "rec@example.com", 200),
        EmailCategory::Primary,
        vec![],
    );
    provider.add_message(
        sent_email("s-rec-3", "acc-rec", "rec@example.com", 300),
        EmailCategory::Primary,
        vec![],
    );

    let inserted = emailops_lib::services::emails::resync_mailbox_full(&db, &account, ExtraMailbox::Sent, &provider)
        .await
        .expect("resync_mailbox_full");

    assert_eq!(
        inserted, 3,
        "resync_mailbox_full must report the 3 newly inserted sent emails"
    );

    let ids = ids_in_mailbox(&db, "acc-rec", "sent");
    assert_eq!(
        ids.len(),
        4,
        "DB must now have existing + 3 recovered sent emails: {:?}",
        ids
    );
    for id in &["s-existing", "s-rec-1", "s-rec-2", "s-rec-3"] {
        assert!(ids.contains(*id), "{} must be present after recovery", id);
    }
}

#[tokio::test]
async fn resync_mailbox_full_resets_done_flag_and_cursor() {
    emailops_lib::services::logger::install_for_testing();
    let db = test_db();
    db.insert_account(&make_account("acc-reset", "reset@example.com"))
        .unwrap();
    let account = db.get_account("acc-reset").unwrap().unwrap();

    // Pre-state: backfill marked done AND cursor stuck at a high value
    // (simulating prior incomplete progress).
    db.set_preference("extra_mailbox_backfill:acc-reset:sent", "1").unwrap();
    db.set_preference("extra_mailbox_backfill_cursor:acc-reset:sent", "1")
        .unwrap();

    // Provider has no messages, so resync just resets state and produces 0 inserts.
    let provider = FakeEmailProvider::new("reset@example.com", "Reset");

    let inserted = emailops_lib::services::emails::resync_mailbox_full(&db, &account, ExtraMailbox::Sent, &provider)
        .await
        .expect("resync_mailbox_full");

    assert_eq!(inserted, 0, "no provider messages → 0 inserted");

    // Cursor must have been reset to a fresh "now" value, much larger than 1.
    let cursor = db
        .get_preference("extra_mailbox_backfill_cursor:acc-reset:sent")
        .unwrap()
        .and_then(|s| s.parse::<i64>().ok())
        .expect("cursor pref must exist after reset");
    assert!(
        cursor > 1_000_000,
        "resync_mailbox_full must reset cursor to ~now() (got {})",
        cursor
    );

    // After running, the backfill must be marked done again (empty
    // provider → reached end of history immediately), but that's OK
    // because we already walked it fresh.
    let done = db.get_preference("extra_mailbox_backfill:acc-reset:sent").unwrap();
    assert_eq!(done.as_deref(), Some("1"));
}

// ── Provider HTTP cassette replay ─────────────────────────────────────────────
//
// These tests boot a wiremock server from a hand-crafted cassette JSON file,
// point a real OutlookClient / GmailClient at it via `with_base_url`, and
// assert the production HTTP-parsing layer reads the recorded shapes
// correctly. The cassette format mirrors what the `record_provider_cassette`
// example produces against live APIs, so the same tests can later run
// against real (sanitised) recordings without code changes.
//
// See `tests/common/mock_server.rs` for the wiremock wiring.

#[tokio::test]
async fn outlook_client_list_messages_against_cassette_mock() {
    use emailops_lib::sync::outlook::OutlookClient;
    use emailops_lib::sync::provider::EmailProvider;
    use std::path::Path;

    let mock = common::mock_server::MockProviderServer::from_cassette_path(Path::new(
        "tests/fixtures/cassettes/outlook/list_inbox_two_messages.json",
    ))
    .await;
    assert_eq!(mock.cassette().provider, "outlook");

    // base_url() ends at the host:port — the OutlookClient appends `/me/...`
    // exactly as it would against the real Graph API. The cassette's
    // `urlPath` includes `/me/...` only (no `/v1.0` prefix on the mock
    // server side), so we point the client at `<mock_base_url>` rather than
    // `<mock_base_url>/v1.0`. This keeps the cassette agnostic to whether
    // the real-world prefix changes in future Graph versions.
    let client = OutlookClient::new("dummy-token".into(), None, None, None).with_base_url(mock.base_url());

    let (refs, next_page) = client
        .list_messages(100, None, None, None, None)
        .await
        .expect("list_messages against cassette");

    assert_eq!(refs.len(), 2, "cassette has 2 message refs");
    assert_eq!(refs[0].id, "AAMkAD-msg-001");
    assert_eq!(refs[0].thread_id, "AAQkAD-thread-001");
    assert_eq!(refs[1].id, "AAMkAD-msg-002");
    assert!(next_page.is_none(), "cassette response had no @odata.nextLink");
}
