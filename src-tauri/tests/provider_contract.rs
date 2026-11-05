// The shared `EmailProvider` contract (`sync::provider_contract`), run against
// every in-process implementation: `FakeEmailProvider`, and `ImapClient` over
// the in-memory IMAP server in three server shapes. `tests/imap_live.rs` runs
// the same checks against a real server.
//
// Run: cargo test --no-default-features --test provider_contract

use emailops_lib::sync::imap_fake::FakeImapServer;
use emailops_lib::sync::provider_contract::{self as contract, ContractMailbox, FakeImapMailbox, FakeProviderMailbox};

/// One test per (implementation, check).
macro_rules! contract_tests {
    ($module:ident, $mailbox:expr) => {
        mod $module {
            use super::*;

            async fn run<F, Fut>(check: F)
            where
                F: FnOnce(&'static dyn ContractMailbox) -> Fut,
                Fut: std::future::Future<Output = Result<(), String>>,
            {
                let mailbox: &'static dyn ContractMailbox = Box::leak(Box::new($mailbox));
                if let Err(e) = check(mailbox).await {
                    panic!("{e}");
                }
            }

            #[tokio::test]
            async fn a_delivered_message_is_listed_in_the_inbox() {
                run(contract::a_delivered_message_is_listed_in_the_inbox).await;
            }
            #[tokio::test]
            async fn a_fetched_message_carries_its_headers() {
                run(contract::a_fetched_message_carries_its_headers).await;
            }
            #[tokio::test]
            async fn the_star_round_trips() {
                run(contract::the_star_round_trips).await;
            }
            #[tokio::test]
            async fn the_read_state_round_trips() {
                run(contract::the_read_state_round_trips).await;
            }
            #[tokio::test]
            async fn archive_leaves_the_inbox_and_can_be_undone() {
                run(contract::archive_leaves_the_inbox_and_can_be_undone).await;
            }
            #[tokio::test]
            async fn spam_leaves_the_inbox() {
                run(contract::spam_leaves_the_inbox).await;
            }
            #[tokio::test]
            async fn trash_leaves_the_inbox() {
                run(contract::trash_leaves_the_inbox).await;
            }
            #[tokio::test]
            async fn a_message_in_place_is_located_under_its_id() {
                run(contract::a_message_in_place_is_located_under_its_id).await;
            }
        }
    };
}

contract_tests!(fake_provider, FakeProviderMailbox::new());
contract_tests!(imap_modern_server, FakeImapMailbox::new(FakeImapServer::standard()));
contract_tests!(
    imap_without_move_or_uidplus,
    FakeImapMailbox::new(FakeImapServer::standard().with_capabilities(&["IMAP4rev1", "SPECIAL-USE"]))
);
contract_tests!(
    imap_roles_only_on_request,
    FakeImapMailbox::new(FakeImapServer::standard().with_roles_only_on_request())
);
