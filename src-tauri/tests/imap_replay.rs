// `ImapClient` against recorded IMAP conversations (`fixtures/imap/*.txt`,
// synthetic): server responses that neither the in-memory server nor a test
// Dovecot produce on demand. Each test also checks the client sent exactly the
// recorded commands.
//
// Run: cargo test --no-default-features --test imap_replay

use emailops_lib::sync::folder_plan::{plan_folders, resolve_role_folder, WellKnownFolder};
use emailops_lib::sync::imap::{ImapClient, ImapCredentials};
use emailops_lib::sync::imap_fake::ReplayServer;
use emailops_lib::sync::provider::EmailProvider;

fn replay(name: &str) -> ReplayServer {
    let path = format!("{}/tests/fixtures/imap/{name}", env!("CARGO_MANIFEST_DIR"));
    let transcript = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    ReplayServer::from_transcript(&transcript)
}

fn client(server: &ReplayServer) -> ImapClient {
    ImapClient::new(
        ImapCredentials {
            host: "imap.example.test".to_string(),
            port: 993,
            username: "user@example.test".to_string(),
            password: "secret".to_string(),
            smtp_host: "smtp.example.test".to_string(),
            smtp_port: 465,
        },
        "user@example.test".to_string(),
        "User".to_string(),
        "acct".to_string(),
    )
    .with_transport(server.transport())
}

fn assert_followed(server: &ReplayServer) {
    assert!(server.mismatches().is_empty(), "{:?}", server.mismatches());
    assert_eq!(server.unused_steps(), 0, "the client stopped before the recording did");
}

#[tokio::test]
async fn gmail_style_folders_resolve_roles_and_keep_odd_names() {
    let server = replay("gmail_style_folders.txt");

    let folders = client(&server).list_folders().await.expect("list");
    assert_followed(&server);

    assert_eq!(
        resolve_role_folder(WellKnownFolder::Sent, &folders).as_deref(),
        Some("[Gmail]/Sent Mail")
    );
    assert_eq!(
        resolve_role_folder(WellKnownFolder::Spam, &folders).as_deref(),
        Some("[Gmail]/Spam")
    );
    let custom: Vec<String> = plan_folders(&folders).custom.into_iter().map(|f| f.raw_name).collect();
    assert!(custom.contains(&"B&APw-ro".to_string()), "{custom:?}");
    assert!(custom.contains(&"Clients \"VIP\"".to_string()), "{custom:?}");
    assert!(
        !custom.iter().any(|f| f == "[Gmail]"),
        "a \\Noselect parent is not a folder: {custom:?}"
    );
}

#[tokio::test]
async fn a_fetch_with_unusual_item_order_is_parsed() {
    let server = replay("fetch_quirks.txt");

    let (email, _, _) = client(&server).get_message("acct::7").await.expect("get");
    assert_followed(&server);

    assert_eq!(email.subject, "Café mañana");
    assert_eq!(email.sender, "Équipe");
    assert!(email.is_read);
    assert!(email.is_starred);
    // 09:15 at -07:00 is 16:15 UTC.
    assert_eq!(email.timestamp, 1_793_722_500);
}

#[tokio::test]
async fn a_bye_mid_command_fails_it_and_the_next_operation_reconnects() {
    let server = replay("bye_mid_command.txt");
    let client = client(&server);

    let during = client.set_starred("acct::7", true).await;
    assert!(during.is_err(), "{during:?}");
    client.set_starred("acct::7", true).await.expect("reconnects");

    assert_followed(&server);
}

#[tokio::test]
async fn a_login_alert_reaches_the_user() {
    let server = replay("login_alert.txt");

    let result = client(&server).login_raw().await;

    let message = format!("{result:?}");
    assert!(message.contains("Application-specific password required"), "{message}");
    assert_followed(&server);
}
