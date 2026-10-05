//! Executor tests: two (or three) installs, each with its own database and
//! fake provider, exchanging the messages their providers "sent".

use std::sync::Arc;

use base64::Engine;
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, Transact};

use super::crdt::test_support::{text_of, Peer};
use super::*;
use crate::sync::provider::{EmailCategory, FakeEmailProvider, FakeMailboxOp};

const NOW: i64 = 1_800_000_000;

struct Install {
    db: Arc<Database>,
    account: Account,
    provider: FakeEmailProvider,
    /// Messages of other installs already delivered here, per sender.
    delivered: std::sync::Mutex<std::collections::HashMap<String, usize>>,
}

fn install(address: &str) -> Install {
    let db = Database::new_for_testing().unwrap();
    db.connection()
        .execute(
            "INSERT INTO accounts (id, provider, email, name, created_at) VALUES (?1, 'gmail', ?2, '', 0)",
            rusqlite::params![format!("acc-{address}"), address],
        )
        .unwrap();
    db.set_preference(SHARED_DOCS_ENABLED_PREF, "true").unwrap();
    let account = db.get_account(&format!("acc-{address}")).unwrap().unwrap();
    Install {
        db: Arc::new(db),
        account,
        provider: FakeEmailProvider::new(address, ""),
        delivered: Default::default(),
    }
}

/// Deliver to `to` every message `from` sent it since the last delivery,
/// dropping the ones whose index is in `lose`. Returns how many arrived.
async fn deliver_except(from: &Install, to: &Install, lose: &[usize]) -> usize {
    let sent = from.provider.sent();
    let start = {
        let mut delivered = to.delivered.lock().unwrap();
        let start = *delivered.get(&from.account.email).unwrap_or(&0);
        delivered.insert(from.account.email.clone(), sent.len());
        start
    };
    let mut arrived = 0;
    for (i, msg) in sent.iter().enumerate().skip(start) {
        // An install of the sender's own sees its messages through Sent.
        let own = from.account.email == to.account.email;
        if lose.contains(&i) || !(own || msg.to_emails.contains(&to.account.email)) {
            continue;
        }
        let email = Email {
            id: format!("{}-{i}", from.account.email),
            account_id: to.account.id.clone(),
            thread_id: "t-doc".into(),
            message_id: None,
            references: None,
            subject: msg.subject.clone(),
            sender: String::new(),
            sender_email: from.account.email.clone(),
            recipients: msg.to_emails.clone(),
            cc: vec![],
            body: msg.body.text.clone(),
            snippet: String::new(),
            timestamp: NOW + i as i64,
            is_read: false,
            triage_status: None,
            category: "primary".into(),
            mailbox: if own { "sent" } else { "inbox" }.into(),
            is_sent: own,
            is_starred: false,
            headers: None,
        };
        let infos: Vec<AttachmentInfo> = msg
            .attachments
            .iter()
            .map(|a| AttachmentInfo {
                attachment_id: "att-1".into(),
                filename: a.filename.clone(),
                mime_type: a.mime_type.clone(),
                size: a.data.len() as i64,
                inline_data: Some(a.data.clone()),
            })
            .collect();
        to.db.insert_emails_batch(std::slice::from_ref(&email)).unwrap();
        to.provider
            .add_message(email.clone(), EmailCategory::Primary, infos.clone());
        ingest_arrivals(&to.db, &to.account, &to.provider, &[(email, infos)], NOW).await;
        arrived += 1;
    }
    arrived
}

async fn deliver(from: &Install, to: &Install) -> usize {
    deliver_except(from, to, &[]).await
}

/// An editor open on `doc_id` in `inst`: loads the stored state.
fn open_editor(inst: &Install, doc_id: &str, client: u64) -> Peer {
    let peer = Peer::new(client);
    let state = state(&inst.db, &inst.account.id, doc_id).unwrap();
    peer.apply(&b64().decode(state).unwrap());
    peer
}

/// Type `text` at the end in the editor and save it as the editor would.
fn type_at_end(inst: &Install, doc_id: &str, editor: &Peer, text: &str) {
    let at = editor.text().chars().count() as u32;
    let update = editor.insert(at, text);
    apply_local_update(&inst.db, &inst.account.id, doc_id, &b64().encode(update), NOW).unwrap();
}

fn stored_text(inst: &Install, doc_id: &str) -> String {
    text_of(&inst.db.shared_doc_state(doc_id).unwrap().unwrap())
}

/// Alice creates "Plan", writes in it and shares it with Bob, who accepts.
async fn shared_pair() -> (Install, Install, String) {
    let alice = install("alice@example.com");
    let bob = install("bob@example.org");
    let doc = create(&alice.db, &alice.account, DocKind::Doc, "Plan", NOW).unwrap();
    let editor = open_editor(&alice, &doc.id, 1);
    type_at_end(&alice, &doc.id, &editor, "Hello");
    share(
        &alice.db,
        &alice.account,
        &alice.provider,
        &doc.id,
        &["Bob@Example.org".into()],
        None,
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(deliver(&alice, &bob).await, 1);
    accept(&bob.db, &bob.account.id, &doc.id, NOW).unwrap();
    (alice, bob, doc.id)
}

#[tokio::test]
async fn an_invitation_carries_the_whole_document_and_waits_for_acceptance() {
    let alice = install("alice@example.com");
    let bob = install("bob@example.org");
    let doc = create(&alice.db, &alice.account, DocKind::Sheet, "Budget", NOW).unwrap();
    type_at_end(&alice, &doc.id, &open_editor(&alice, &doc.id, 1), "draft");

    share(
        &alice.db,
        &alice.account,
        &alice.provider,
        &doc.id,
        &["bob@example.org".into()],
        Some("<table><tr><td>draft</td></tr></table><script>x()</script>"),
        NOW,
    )
    .await
    .unwrap();
    deliver(&alice, &bob).await;

    let sent = &alice.provider.sent()[0];
    assert_eq!(sent.to_emails, vec!["bob@example.org"]);
    assert!(!sent.body.append_footer);
    let html = sent.body.html.clone().unwrap();
    assert!(html.contains("<td>draft</td>") && !html.contains("script"), "{html}");
    let received = bob.db.get_shared_doc(&doc.id).unwrap().unwrap();
    assert_eq!(
        (
            received.status,
            received.kind,
            received.title.as_str(),
            received.consented_at
        ),
        (DocStatus::Invited, DocKind::Sheet, "Budget", None)
    );
    assert_eq!(received.participants, vec!["alice@example.com", "bob@example.org"]);
    assert_eq!(stored_text(&bob, &doc.id), "draft");
    assert!(
        apply_local_update(
            &bob.db,
            &bob.account.id,
            &doc.id,
            &b64().encode(crdt::empty_state()),
            NOW
        )
        .is_err(),
        "an invitation is read-only until accepted"
    );
    assert!(
        bob.provider.mailbox_ops().is_empty(),
        "the invitation stays in the inbox"
    );
}

#[tokio::test]
async fn concurrent_edits_on_both_sides_converge() {
    let (alice, bob, doc) = shared_pair().await;
    let alice_editor = open_editor(&alice, &doc, 1);
    let bob_editor = open_editor(&bob, &doc, 2);

    type_at_end(&alice, &doc, &alice_editor, " from Alice");
    type_at_end(&bob, &doc, &bob_editor, " from Bob");
    assert!(flush(&alice.db, &alice.account, &alice.provider, &doc).await.unwrap());
    assert!(flush(&bob.db, &bob.account, &bob.provider, &doc).await.unwrap());
    deliver(&alice, &bob).await;
    deliver(&bob, &alice).await;

    let text = stored_text(&alice, &doc);
    assert_eq!(text, stored_text(&bob, &doc));
    assert!(
        text.starts_with("Hello") && text.contains(" from Alice") && text.contains(" from Bob"),
        "{text}"
    );
}

#[tokio::test]
async fn a_lost_message_is_made_good_by_the_next_exchange() {
    let (alice, bob, doc) = shared_pair().await;
    let editor = open_editor(&alice, &doc, 1);
    type_at_end(&alice, &doc, &editor, " one");
    flush(&alice.db, &alice.account, &alice.provider, &doc).await.unwrap();
    type_at_end(&alice, &doc, &editor, " two");
    flush(&alice.db, &alice.account, &alice.provider, &doc).await.unwrap();

    // The " one" message (index 1, after the invitation) never arrives.
    deliver_except(&alice, &bob, &[1]).await;
    assert_eq!(stored_text(&bob, &doc), "Hello", "\" two\" waits on the lost change");
    assert!(
        bob.db.get_shared_doc(&doc).unwrap().unwrap().dirty_since.is_some(),
        "Bob asks for a catch-up"
    );

    assert!(flush(&bob.db, &bob.account, &bob.provider, &doc).await.unwrap());
    deliver(&bob, &alice).await;
    assert!(flush(&alice.db, &alice.account, &alice.provider, &doc).await.unwrap());
    deliver(&alice, &bob).await;

    assert_eq!(stored_text(&bob, &doc), "Hello one two");
}

#[tokio::test]
async fn a_message_seen_twice_is_applied_once_and_updates_are_archived() {
    let (alice, bob, doc) = shared_pair().await;
    type_at_end(&alice, &doc, &open_editor(&alice, &doc, 1), "!");
    flush(&alice.db, &alice.account, &alice.provider, &doc).await.unwrap();
    deliver(&alice, &bob).await;
    bob.delivered.lock().unwrap().insert(alice.account.email.clone(), 1);
    deliver(&alice, &bob).await;

    assert_eq!(stored_text(&bob, &doc), "Hello!");
    let ops = bob.provider.mailbox_ops();
    assert!(
        ops.iter().any(|op| matches!(op, FakeMailboxOp::Archive { .. })),
        "{ops:?}"
    );
}

#[tokio::test]
async fn nothing_is_mailed_before_the_user_shares_or_while_nobody_else_is_in_it() {
    let alice = install("alice@example.com");
    let doc = create(&alice.db, &alice.account, DocKind::Doc, "Private", NOW).unwrap();
    type_at_end(&alice, &doc.id, &open_editor(&alice, &doc.id, 1), "secret");

    assert!(!flush(&alice.db, &alice.account, &alice.provider, &doc.id)
        .await
        .unwrap());
    assert!(alice.provider.sent().is_empty());
    assert!(share(
        &alice.db,
        &alice.account,
        &alice.provider,
        &doc.id,
        &["Alice@example.com".into()],
        None,
        NOW
    )
    .await
    .is_err());
    assert!(alice.provider.sent().is_empty());
}

#[tokio::test]
async fn a_forged_participant_is_ignored() {
    let (alice, bob, doc) = shared_pair().await;
    let eve = install("eve@example.net");
    let forged = Envelope {
        doc_id: doc.clone(),
        kind: DocKind::Doc,
        title: "Plan".into(),
        participants: vec![
            "alice@example.com".into(),
            "bob@example.org".into(),
            "eve@example.net".into(),
        ],
        state_vector: crdt::empty_state_vector(),
        update: Peer::new(66).insert(0, "pwned "),
    };
    let data = b64().encode(envelope::encode(&forged).unwrap());
    eve.provider
        .send_new_email(
            "eve@example.net",
            None,
            &["bob@example.org".into()],
            &[],
            "x",
            &EmailBody::plain("x"),
            &[EmailAttachment {
                filename: envelope::file_name(&doc),
                mime_type: envelope::ENVELOPE_MIME.into(),
                data,
                content_id: None,
                is_inline: false,
            }],
        )
        .await
        .unwrap();
    deliver(&eve, &bob).await;

    assert_eq!(stored_text(&bob, &doc), "Hello");
    drop(alice);
}

#[tokio::test]
async fn a_left_document_ignores_later_changes() {
    let (alice, bob, doc) = shared_pair().await;
    leave(&bob.db, &bob.account.id, &doc, NOW).unwrap();
    type_at_end(&alice, &doc, &open_editor(&alice, &doc, 1), " more");
    flush(&alice.db, &alice.account, &alice.provider, &doc).await.unwrap();
    deliver(&alice, &bob).await;

    assert_eq!(stored_text(&bob, &doc), "Hello");
    assert!(!flush(&bob.db, &bob.account, &bob.provider, &doc).await.unwrap());
}

#[tokio::test]
async fn a_failed_send_keeps_the_changes_pending() {
    let (alice, _bob, doc) = shared_pair().await;
    type_at_end(&alice, &doc, &open_editor(&alice, &doc, 1), "!");
    alice.provider.fail_sends(Some("server down"));

    assert!(flush(&alice.db, &alice.account, &alice.provider, &doc).await.is_err());
    assert!(alice.db.get_shared_doc(&doc).unwrap().unwrap().dirty_since.is_some());
}

#[tokio::test]
async fn an_editor_catches_up_with_a_diff_from_its_state_vector() {
    let (alice, bob, doc) = shared_pair().await;
    let bob_editor = open_editor(&bob, &doc, 2);
    type_at_end(&alice, &doc, &open_editor(&alice, &doc, 1), " world");
    flush(&alice.db, &alice.account, &alice.provider, &doc).await.unwrap();
    deliver(&alice, &bob).await;

    let sv = b64().encode(bob_editor.doc.transact().state_vector().encode_v1());
    bob_editor.apply(
        &b64()
            .decode(diff_since(&bob.db, &bob.account.id, &doc, &sv).unwrap())
            .unwrap(),
    );
    assert_eq!(bob_editor.text(), "Hello world");
    assert!(
        state(&bob.db, "acc-someone-else", &doc).is_err(),
        "another account cannot read it"
    );
}

#[tokio::test]
async fn nothing_is_ingested_while_the_feature_is_off() {
    let alice = install("alice@example.com");
    let bob = install("bob@example.org");
    bob.db.set_preference(SHARED_DOCS_ENABLED_PREF, "false").unwrap();
    let doc = create(&alice.db, &alice.account, DocKind::Doc, "Plan", NOW).unwrap();
    share(
        &alice.db,
        &alice.account,
        &alice.provider,
        &doc.id,
        &["bob@example.org".into()],
        None,
        NOW,
    )
    .await
    .unwrap();
    deliver(&alice, &bob).await;
    assert!(bob.db.get_shared_doc(&doc.id).unwrap().is_none());
}

struct Unreachable;

#[async_trait::async_trait]
impl OutboxProviders for Unreachable {
    async fn provider_for(&self, _account: &Account) -> Result<Box<dyn EmailProvider>> {
        Err(AppError::SyncError("offline".into()))
    }
}

#[tokio::test]
async fn due_documents_are_flushed_after_the_pause_and_failures_retried() {
    let (alice, _bob, doc) = shared_pair().await;
    type_at_end(&alice, &doc, &open_editor(&alice, &doc, 1), "!");

    assert_eq!(flush_due(&alice.db, NOW + 5, &Unreachable).await, 0, "still typing");
    assert_eq!(
        flush_due(&alice.db, NOW + planner::FLUSH_DEBOUNCE_SECS, &Unreachable).await,
        0,
        "the provider could not be built"
    );
    assert!(
        alice.db.get_shared_doc(&doc).unwrap().unwrap().dirty_since.is_some(),
        "kept for the next pass"
    );
}

#[tokio::test]
async fn the_owners_other_install_picks_the_document_up_from_sent_as_its_own() {
    let (alice, bob, doc) = shared_pair().await;
    let alice_laptop = install("alice@example.com");
    deliver(&alice, &alice_laptop).await;

    let there = alice_laptop.db.get_shared_doc(&doc).unwrap().unwrap();
    assert_eq!(there.status, DocStatus::Active);
    assert!(there.consented_at.is_some(), "shared by the same user elsewhere");
    assert_eq!(stored_text(&alice_laptop, &doc), "Hello");
    assert!(alice_laptop.provider.mailbox_ops().is_empty(), "Sent is left alone");
    drop(bob);
}

// ── Folders, history, search ──────────────────────────────────────────────

/// Save a paragraph typed in the doc editor of `inst`.
fn write_paragraph(inst: &Install, doc_id: &str, client: u64, text: &str, now: i64) {
    let editor = Peer::new(client);
    editor.apply(
        &b64()
            .decode(state(&inst.db, &inst.account.id, doc_id).unwrap())
            .unwrap(),
    );
    let update = editor.paragraph(text);
    apply_local_update(&inst.db, &inst.account.id, doc_id, &b64().encode(update), now).unwrap();
}

#[tokio::test]
async fn search_finds_documents_by_title_and_by_what_was_written_in_them() {
    let alice = install("alice@example.com");
    let plan = create(&alice.db, &alice.account, DocKind::Doc, "Lisbon plan", NOW).unwrap();
    let notes = create(&alice.db, &alice.account, DocKind::Doc, "Notes", NOW).unwrap();
    write_paragraph(&alice, &notes.id, 1, "Book the ferry to Cacilhas", NOW);

    let found = |q: &str| -> Vec<String> {
        search(&alice.db, &alice.account.id, q)
            .unwrap()
            .into_iter()
            .map(|d| d.id)
            .collect()
    };
    assert_eq!(found("lisb"), vec![plan.id.clone()]);
    assert_eq!(found("ferry"), vec![notes.id.clone()]);
    assert!(found("   ").is_empty());
    assert!(search(&alice.db, "acc-other", "ferry").unwrap().is_empty());
}

#[tokio::test]
async fn a_change_that_arrives_by_email_is_searchable_and_in_the_history() {
    let alice = install("alice@example.com");
    let bob = install("bob@example.org");
    let fresh = create(&alice.db, &alice.account, DocKind::Doc, "Minutes", NOW).unwrap();
    share(
        &alice.db,
        &alice.account,
        &alice.provider,
        &fresh.id,
        &["bob@example.org".into()],
        None,
        NOW,
    )
    .await
    .unwrap();
    write_paragraph(&alice, &fresh.id, 1, "Decision: travel on Tuesday", NOW + 10);
    flush(&alice.db, &alice.account, &alice.provider, &fresh.id)
        .await
        .unwrap();
    deliver(&alice, &bob).await;

    let hits: Vec<String> = search(&bob.db, &bob.account.id, "tuesday")
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(hits, vec![fresh.id.clone()]);
    let history = versions(&bob.db, &bob.account.id, &fresh.id).unwrap();
    assert!(history
        .iter()
        .all(|v| v.author == "alice@example.com" && v.origin == "remote"));
    let newest = &history[0];
    let then = b64()
        .decode(version_state(&bob.db, &bob.account.id, &fresh.id, newest.id).unwrap())
        .unwrap();
    assert_eq!(
        crdt::plain_text(&then, DocKind::Doc).unwrap().trim(),
        "Decision: travel on Tuesday"
    );
    assert!(version_state(&alice.db, &alice.account.id, &fresh.id, newest.id + 999).is_err());
}

#[tokio::test]
async fn local_edits_are_versions_by_this_account() {
    let alice = install("alice@example.com");
    let doc = create(&alice.db, &alice.account, DocKind::Doc, "Draft", NOW).unwrap();
    write_paragraph(&alice, &doc.id, 1, "First idea", NOW);
    write_paragraph(&alice, &doc.id, 2, "Second idea", NOW + 1_000);

    let history = versions(&alice.db, &alice.account.id, &doc.id).unwrap();
    assert_eq!(history.len(), 2);
    assert!(history
        .iter()
        .all(|v| v.author == "alice@example.com" && v.origin == "local"));
    let first = b64()
        .decode(version_state(&alice.db, &alice.account.id, &doc.id, history[1].id).unwrap())
        .unwrap();
    assert_eq!(crdt::plain_text(&first, DocKind::Doc).unwrap().trim(), "First idea");
}

#[tokio::test]
async fn folders_are_per_account_and_deleting_one_keeps_its_documents() {
    let alice = install("alice@example.com");
    let doc = create(&alice.db, &alice.account, DocKind::Sheet, "Budget", NOW).unwrap();
    let trips = create_folder(&alice.db, &alice.account.id, "  Trips ", None, NOW).unwrap();
    let lisbon = create_folder(&alice.db, &alice.account.id, "Lisbon", Some(&trips.id), NOW).unwrap();
    assert_eq!(trips.name, "Trips");

    let moved = move_doc(&alice.db, &alice.account.id, &doc.id, Some(&lisbon.id)).unwrap();
    assert_eq!(moved.folder_id.as_deref(), Some(lisbon.id.as_str()));
    assert!(create_folder(&alice.db, &alice.account.id, "\n", None, NOW).is_err());
    assert!(move_doc(&alice.db, "acc-other", &doc.id, None).is_err());
    assert!(create_folder(&alice.db, "acc-other", "X", Some(&trips.id), NOW).is_err());

    delete_folder(&alice.db, &alice.account.id, &lisbon.id).unwrap();
    let after = alice.db.get_shared_doc(&doc.id).unwrap().unwrap();
    assert_eq!(after.folder_id.as_deref(), Some(trips.id.as_str()));
    assert_eq!(
        rename_folder(&alice.db, &alice.account.id, &trips.id, "Travel")
            .unwrap()
            .name,
        "Travel"
    );
}

#[tokio::test]
async fn documents_from_before_the_index_existed_are_found_too() {
    let alice = install("alice@example.com");
    let doc = create(&alice.db, &alice.account, DocKind::Doc, "Old notes", NOW).unwrap();
    write_paragraph(&alice, &doc.id, 1, "harbour visit", NOW);
    // As after the V037 upgrade: the document exists, its index entry does not.
    alice
        .db
        .connection()
        .execute("DELETE FROM shared_docs_fts", [])
        .unwrap();

    let found: Vec<String> = search(&alice.db, &alice.account.id, "harbour")
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(found, vec![doc.id]);
}
