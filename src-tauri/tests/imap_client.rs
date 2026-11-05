// `ImapClient` end to end against the in-memory `FakeImapServer`: every
// operation goes through the real `imap` crate and its wire parsing, with no
// network. Server quirks and faults are set up per test.
//
// Run: cargo test --no-default-features --test imap_client

use emailops_lib::sync::imap::{ImapClient, ImapCredentials};
use emailops_lib::sync::imap_fake::{synthetic_message, FakeImapServer, Fault};
use emailops_lib::sync::provider::{EmailBody, EmailProvider, MoveTarget, RemoteMessageState};

const ACCOUNT: &str = "acct";
const NOW: i64 = 1_793_000_000;

fn creds() -> ImapCredentials {
    ImapCredentials {
        host: "imap.example.test".to_string(),
        port: 993,
        username: "user@example.test".to_string(),
        password: "secret".to_string(),
        smtp_host: "smtp.example.test".to_string(),
        smtp_port: 465,
    }
}

fn client(server: &FakeImapServer) -> ImapClient {
    ImapClient::new(
        creds(),
        "user@example.test".to_string(),
        "Test User".to_string(),
        ACCOUNT.to_string(),
    )
    .with_transport(server.transport())
}

/// Deliver a synthetic message to INBOX; returns its app id and Message-ID.
fn deliver_inbox(server: &FakeImapServer, n: u32) -> (String, String) {
    let message_id = format!("<m{n}@example.test>");
    let raw = synthetic_message(
        &message_id,
        &format!("Subject {n}"),
        "sender@example.test",
        "user@example.test",
        NOW,
    );
    let uid = server.deliver("INBOX", &raw, &[], NOW);
    (format!("{ACCOUNT}::{uid}"), message_id)
}

// ── archive ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn archive_moves_the_message_into_the_archive_folder() {
    let server = FakeImapServer::standard();
    let (id, message_id) = deliver_inbox(&server, 1);

    let location = client(&server)
        .archive_message(&id, Some(&message_id))
        .await
        .expect("archive");

    assert_eq!(server.find(&message_id).map(|(f, _)| f), Some("Archive".to_string()));
    assert!(server.messages("INBOX").is_empty());
    assert_eq!(location.mailbox, "folder:Archive");
    let (_, new_uid) = server.find(&message_id).expect("archived");
    assert!(location.id.ends_with(&format!("::{new_uid}")), "{}", location.id);
}

#[tokio::test]
async fn archiving_a_message_already_in_the_archive_moves_nothing() {
    let server = FakeImapServer::standard();
    let message_id = "<filed@example.test>";
    let raw = synthetic_message(message_id, "Filed", "a@example.test", "b@example.test", NOW);
    let uid = server.deliver("Archive", &raw, &[], NOW);
    let client = client(&server);
    // The id the sync gives a message of the Archive folder.
    let listed = client
        .list_folder_messages("Archive", 10, None, None)
        .await
        .expect("list archive");
    let id = listed[0].id.clone();

    let location = client.archive_message(&id, Some(message_id)).await.expect("archive");

    assert_eq!(location.id, id);
    assert_eq!(server.find(message_id), Some(("Archive".to_string(), uid)));
    assert!(!server.commands().iter().any(|c| c.starts_with("UID MOVE")));
}

#[tokio::test]
async fn archive_refuses_when_the_uid_now_names_another_message() {
    let server = FakeImapServer::standard();
    let (id, _) = deliver_inbox(&server, 1);

    let result = client(&server)
        .archive_message(&id, Some("<someone-else@example.test>"))
        .await;

    assert!(result.is_err(), "{result:?}");
    assert_eq!(server.messages("INBOX").len(), 1, "nothing may move");
}

#[tokio::test]
async fn archive_without_an_archive_folder_is_a_typed_error() {
    let server = FakeImapServer::new();
    let (id, message_id) = deliver_inbox(&server, 1);

    let result = client(&server).archive_message(&id, Some(&message_id)).await;

    assert!(
        matches!(result, Err(emailops_lib::models::error::AppError::NoArchiveFolder)),
        "{result:?}"
    );
}

#[tokio::test]
async fn archive_from_a_folder_the_server_lost_is_an_error() {
    let server = FakeImapServer::standard().with_folder("Projects", &[]);
    let raw = synthetic_message("<p@example.test>", "P", "a@example.test", "b@example.test", NOW);
    server.deliver("Projects", &raw, &[], NOW);
    let client = client(&server);
    let listed = client
        .list_folder_messages("Projects", 10, None, None)
        .await
        .expect("list");
    server.fault(Fault::RefuseNext {
        verb: "SELECT".to_string(),
        reply: "NO [NONEXISTENT] Mailbox doesn't exist".to_string(),
    });
    // LIST runs first; the SELECT of the source is the one refused.
    let result = client.archive_message(&listed[0].id, Some("<p@example.test>")).await;

    assert!(result.is_err(), "{result:?}");
    assert_eq!(server.messages("Archive").len(), 0);
}

// ── move without MOVE / UIDPLUS ─────────────────────────────────────────────

#[tokio::test]
async fn without_move_the_message_is_copied_and_only_it_is_expunged() {
    let server = FakeImapServer::standard().with_capabilities(&["IMAP4rev1", "UIDPLUS", "SPECIAL-USE"]);
    let (id, message_id) = deliver_inbox(&server, 1);
    // Another message the user flagged \Deleted in another client: UID
    // EXPUNGE must leave it alone.
    let other = synthetic_message("<other@example.test>", "O", "a@example.test", "b@example.test", NOW);
    server.deliver("INBOX", &other, &["\\Deleted"], NOW);

    client(&server)
        .archive_message(&id, Some(&message_id))
        .await
        .expect("archive");

    assert_eq!(server.find(&message_id).map(|(f, _)| f), Some("Archive".to_string()));
    let inbox = server.messages("INBOX");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].message_id().as_deref(), Some("<other@example.test>"));
    assert!(server.commands().iter().any(|c| c.starts_with("UID EXPUNGE")));
}

#[tokio::test]
async fn without_move_or_uidplus_a_plain_expunge_finishes_the_move() {
    let server = FakeImapServer::standard().with_capabilities(&["IMAP4rev1", "SPECIAL-USE"]);
    let (id, message_id) = deliver_inbox(&server, 1);

    client(&server)
        .archive_message(&id, Some(&message_id))
        .await
        .expect("archive");

    assert!(server.messages("INBOX").is_empty());
    assert_eq!(server.find(&message_id).map(|(f, _)| f), Some("Archive".to_string()));
    assert!(server.commands().iter().any(|c| c == "EXPUNGE"));
}

// ── spam ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn move_to_spam_files_the_message_in_junk_and_rekeys_it() {
    let server = FakeImapServer::standard();
    let (id, message_id) = deliver_inbox(&server, 1);

    let location = client(&server)
        .move_to_spam(&id, Some(&message_id))
        .await
        .expect("spam");

    let (folder, uid) = server.find(&message_id).expect("in junk");
    assert_eq!(folder, "Junk");
    assert_eq!(location.mailbox, "spam");
    assert_eq!(location.id, format!("{ACCOUNT}::SPAM::{uid}"));
}

#[tokio::test]
async fn a_message_already_in_spam_stays_put() {
    let server = FakeImapServer::standard();
    let raw = synthetic_message("<s@example.test>", "S", "a@example.test", "b@example.test", NOW);
    let uid = server.deliver("Junk", &raw, &[], NOW);
    let id = format!("{ACCOUNT}::SPAM::{uid}");

    let location = client(&server)
        .move_to_spam(&id, Some("<s@example.test>"))
        .await
        .expect("spam");

    assert_eq!(location.id, id);
    assert!(server.commands().is_empty() || !server.commands().iter().any(|c| c.starts_with("UID MOVE")));
}

#[tokio::test]
async fn spam_refuses_when_the_uid_now_names_another_message() {
    let server = FakeImapServer::standard();
    let (id, _) = deliver_inbox(&server, 1);

    let result = client(&server).move_to_spam(&id, Some("<changed@example.test>")).await;

    assert!(result.is_err());
    assert!(server.messages("Junk").is_empty());
}

#[tokio::test]
async fn spam_without_a_junk_folder_is_a_typed_error() {
    let server = FakeImapServer::new();
    let (id, message_id) = deliver_inbox(&server, 1);

    let result = client(&server).move_to_spam(&id, Some(&message_id)).await;

    assert!(
        matches!(result, Err(emailops_lib::models::error::AppError::NoSpamFolder)),
        "{result:?}"
    );
}

// ── trash ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn trash_moves_to_trash_and_never_destroys_mail() {
    let server = FakeImapServer::standard();
    let (id, message_id) = deliver_inbox(&server, 1);

    client(&server)
        .trash_message(&id, Some(&message_id))
        .await
        .expect("trash");

    assert_eq!(server.find(&message_id).map(|(f, _)| f), Some("Trash".to_string()));
}

// ── flags ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn starring_sets_and_clears_flagged_on_the_server() {
    let server = FakeImapServer::standard();
    let (id, _) = deliver_inbox(&server, 1);
    let client = client(&server);

    client.set_starred(&id, true).await.expect("star");
    assert!(server.messages("INBOX")[0].has_flag("\\Flagged"));

    client.set_starred(&id, false).await.expect("unstar");
    assert!(!server.messages("INBOX")[0].has_flag("\\Flagged"));
}

#[tokio::test]
async fn read_state_follows_seen() {
    let server = FakeImapServer::standard();
    let (id, _) = deliver_inbox(&server, 1);
    let client = client(&server);

    client.set_read_state(&id, true).await.expect("read");
    assert!(server.messages("INBOX")[0].has_flag("\\Seen"));
    client.set_read_state(&id, false).await.expect("unread");
    assert!(!server.messages("INBOX")[0].has_flag("\\Seen"));
}

#[tokio::test]
async fn fetching_a_message_never_marks_it_read() {
    let server = FakeImapServer::standard();
    let (id, _) = deliver_inbox(&server, 1);

    let (email, _, _) = client(&server).get_message(&id).await.expect("get");

    assert!(!email.is_read);
    assert!(!server.messages("INBOX")[0].has_flag("\\Seen"));
}

#[tokio::test]
async fn message_states_report_flags_and_missing_messages() {
    let server = FakeImapServer::standard();
    let (kept, _) = deliver_inbox(&server, 1);
    let (gone, gone_message_id) = deliver_inbox(&server, 2);
    let client = client(&server);
    client.set_starred(&kept, true).await.expect("star");
    // Moved away in another client.
    client
        .trash_message(&gone, Some(&gone_message_id))
        .await
        .expect("trash");

    let states = client
        .fetch_message_states(&[kept.clone(), gone.clone()])
        .await
        .expect("states")
        .expect("supported");

    assert_eq!(
        states.get(&kept),
        Some(&RemoteMessageState::Present {
            is_read: false,
            is_starred: Some(true)
        })
    );
    assert_eq!(states.get(&gone), Some(&RemoteMessageState::Missing));
}

#[tokio::test]
async fn message_states_skip_a_folder_whose_fetch_was_refused() {
    let server = FakeImapServer::standard();
    let (id, _) = deliver_inbox(&server, 1);
    server.fault(Fault::RefuseNext {
        verb: "UID FETCH".to_string(),
        reply: "NO [SERVERBUG] Internal error".to_string(),
    });

    let states = client(&server)
        .fetch_message_states(std::slice::from_ref(&id))
        .await
        .expect("states")
        .expect("supported");

    assert!(!states.contains_key(&id), "a refused fetch says nothing: {states:?}");
}

// ── folders ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn folders_are_created_renamed_and_deleted_on_the_server() {
    let server = FakeImapServer::standard();
    let client = client(&server);

    client.create_folder("Clients").await.expect("create");
    client.rename_folder("Clients", "Customers").await.expect("rename");
    assert!(server.folder_names().contains(&"Customers".to_string()));
    client.delete_folder("Customers").await.expect("delete");
    assert!(!server.folder_names().contains(&"Customers".to_string()));
}

#[tokio::test]
async fn roles_sent_only_on_request_are_still_detected() {
    let server = FakeImapServer::new()
        .with_capabilities(&["IMAP4rev1", "MOVE", "SPECIAL-USE"])
        .with_folder("Correo no deseado", &["\\Junk"])
        .with_roles_only_on_request();

    let folders = client(&server).list_folders().await.expect("list");

    let junk = folders
        .iter()
        .find(|f| f.raw_name == "Correo no deseado")
        .expect("listed");
    assert!(
        junk.attributes.iter().any(|a| a.eq_ignore_ascii_case("\\Junk")),
        "{junk:?}"
    );
}

#[tokio::test]
async fn move_to_a_custom_folder_returns_its_new_id() {
    let server = FakeImapServer::standard().with_folder("Projects", &[]);
    let (id, message_id) = deliver_inbox(&server, 1);

    let moved = client(&server)
        .move_message(&id, Some(&message_id), &MoveTarget::Folder("Projects".to_string()))
        .await
        .expect("move")
        .expect("new id");

    let (folder, uid) = server.find(&message_id).expect("moved");
    assert_eq!(folder, "Projects");
    assert!(moved.id.ends_with(&format!("::{uid}")));
}

// ── listing ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn listing_returns_inbox_newest_first_then_fetches_bodies() {
    let server = FakeImapServer::standard();
    for n in 1..=3 {
        deliver_inbox(&server, n);
    }
    let client = client(&server);

    let (refs, _) = client.list_messages(50, None, None, None, None).await.expect("list");
    let ids: Vec<&str> = refs.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids[..3],
        [
            format!("{ACCOUNT}::3"),
            format!("{ACCOUNT}::2"),
            format!("{ACCOUNT}::1")
        ]
    );

    let fetched = client.batch_get_messages(&ids[..3]).await.expect("batch");
    assert_eq!(fetched.len(), 3);
    assert!(fetched.iter().all(|r| r.is_ok()));
}

#[tokio::test]
async fn operations_of_one_client_share_one_connection() {
    let server = FakeImapServer::standard();
    let (id, _) = deliver_inbox(&server, 1);
    let client = client(&server);

    client.set_starred(&id, true).await.expect("star");
    client.set_read_state(&id, true).await.expect("read");
    client.get_message(&id).await.expect("get");

    assert_eq!(server.connections(), 1);
}

#[tokio::test]
async fn sending_submits_over_smtp_and_files_a_seen_copy_in_sent() {
    let server = FakeImapServer::standard();
    let body = EmailBody::plain("Hello there");

    client(&server)
        .send_new_email(
            "user@example.test",
            Some("Test User"),
            &["friend@example.test".to_string()],
            &[],
            "Greetings",
            &body,
            &[],
        )
        .await
        .expect("send");

    assert_eq!(server.submitted().len(), 1);
    let sent = server.messages("Sent");
    assert_eq!(sent.len(), 1);
    assert!(sent[0].has_flag("\\Seen"));
}

// ── faults ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_refused_connection_is_retried() {
    let server = FakeImapServer::standard();
    server.fault(Fault::RefuseConnections(1));

    let folders = client(&server).list_folders().await;

    assert!(folders.is_ok(), "{folders:?}");
    assert_eq!(server.connections(), 1, "the second attempt connected");
}

#[tokio::test]
async fn a_throttled_login_is_retried() {
    let server = FakeImapServer::standard();
    server.fault(Fault::ThrottleLogins(1));

    let folders = client(&server).list_folders().await;

    assert!(folders.is_ok(), "{folders:?}");
    assert_eq!(server.connections(), 2);
}

#[tokio::test]
async fn rejected_credentials_are_not_retried() {
    let server = FakeImapServer::standard();
    server.fault(Fault::RejectLogins);

    let result = client(&server).list_folders().await;

    assert!(result.is_err());
    assert_eq!(server.connections(), 1, "asking again only risks a lockout");
}

#[tokio::test]
async fn a_connection_dropped_mid_operation_fails_it_and_the_next_reconnects() {
    let server = FakeImapServer::standard();
    let (id, _) = deliver_inbox(&server, 1);
    let client = client(&server);
    client.get_message(&id).await.expect("warm up");
    // NOOP (liveness) and SELECT get through; the STORE does not.
    server.fault(Fault::DropAfterCommands(2));

    let during = client.set_starred(&id, true).await;
    assert!(during.is_err(), "{during:?}");
    assert!(!server.messages("INBOX")[0].has_flag("\\Flagged"));

    client.set_starred(&id, true).await.expect("reconnects");
    assert!(server.messages("INBOX")[0].has_flag("\\Flagged"));
    assert_eq!(server.connections(), 2);
}

#[tokio::test]
async fn untagged_responses_from_another_client_do_not_break_operations() {
    let server = FakeImapServer::standard();
    let (id, message_id) = deliver_inbox(&server, 1);
    deliver_inbox(&server, 2);
    let client = client(&server);
    // Another client marks message 2 read while ours works.
    for verb in ["SELECT", "UID SEARCH", "UID FETCH", "UID STORE"] {
        server.fault(Fault::InterleaveNext {
            verb: verb.to_string(),
            line: "2 FETCH (FLAGS (\\Seen) UID 2)".to_string(),
        });
    }

    let (refs, _) = client.list_messages(50, None, None, None, None).await.expect("list");
    assert!(refs.iter().any(|r| r.id == id));
    let (email, _, _) = client.get_message(&id).await.expect("get");
    assert_eq!(email.message_id.as_deref(), Some(message_id.as_str()));
    client.set_read_state(&id, true).await.expect("read");
}

#[tokio::test]
async fn a_rebuilt_mailbox_reports_its_new_uid_validity() {
    let server = FakeImapServer::standard();
    deliver_inbox(&server, 1);
    let client = client(&server);
    let before = client.folder_uid_validities().await.expect("before");
    server.renumber("INBOX");

    let after = client.folder_uid_validities().await.expect("after");

    let inbox = |v: &[emailops_lib::sync::provider::FolderUidValidity]| {
        v.iter().find(|f| f.mailbox == "inbox").map(|f| f.uid_validity)
    };
    assert_ne!(inbox(&before), inbox(&after));
    assert_eq!(inbox(&after), server.uid_validity("INBOX"));
}

#[tokio::test]
async fn an_unsolicited_fetch_does_not_add_an_identity_without_a_message_id() {
    let server = FakeImapServer::standard();
    deliver_inbox(&server, 1);
    deliver_inbox(&server, 2);
    server.fault(Fault::InterleaveNext {
        verb: "UID FETCH".to_string(),
        line: "2 FETCH (FLAGS (\\Seen) UID 2)".to_string(),
    });

    let identities = client(&server)
        .list_mailbox_identities("inbox")
        .await
        .expect("identities");

    assert_eq!(identities.len(), 2, "{identities:?}");
    assert!(identities.iter().all(|i| i.message_id.is_some()), "{identities:?}");
}
