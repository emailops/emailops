# Testing IMAP accounts

IMAP behaviour is validated at six levels. The first four run in a plain
`cargo test`. The live, CLI and UI levels need Docker.

| Level | What it proves | Where | Run |
|---|---|---|---|
| In-memory server | Every `ImapClient` operation through the real `imap` crate: archive, spam, trash, flags, message states, folders, sending, MOVE-less fallbacks, faults (refused connections, throttled or rejected logins, dropped connections, interleaved untagged responses, UIDVALIDITY changes) | `src-tauri/src/sync/imap_fake.rs` (`FakeImapServer`), `src-tauri/tests/imap_client.rs` | `cargo test --no-default-features --test imap_client` |
| Provider contract | The behaviour every `EmailProvider` shares, written once and run on `FakeEmailProvider` and on `ImapClient` over three server shapes, so the fakes cannot drift apart | `src-tauri/src/sync/provider_contract.rs`, `src-tauri/tests/provider_contract.rs` | `cargo test --no-default-features --test provider_contract` |
| Recorded conversations | Server responses nothing else produces on demand: Gmail-style hierarchies, folder names sent as literals, unusual FETCH item order, `* BYE` mid-command, login `[ALERT]` | `ReplayServer` in `imap_fake.rs`, `src-tauri/tests/fixtures/imap/*.txt`, `src-tauri/tests/imap_replay.rs` | `cargo test --no-default-features --test imap_replay` |
| Properties | Id codecs round-trip, inbox paging visits every UID once, the SEARCH window and the parsers never panic on server bytes, and any sequence of star/read changes matches a model | `src-tauri/src/sync/imap_proptests.rs`, `flag_model` in `tests/imap_client.rs` | `cargo test --no-default-features sync::imap::proptests` |
| Live server | The provider contract against a real Dovecot over STARTTLS, sending through SMTP submission to Mailpit, a 120-message mailbox, and network faults through Toxiproxy (server back within the retry window, slow link, connection reset) | `src-tauri/tests/imap_live.rs`, `scripts/imap_test/` | `make imap-live` |
| CLI | `emailops-cli` on a throwaway data dir: add account, sync, list, search, show, send, re-sync without duplicates, with a clean `--json` stdout | `scripts/imap_test/e2e_cli.py` | `make imap-e2e` (live + CLI) |
| Desktop UI | The real app on a fresh data dir: the first-run wizard's IMAP form, Test connection, the mail in the inbox, star and archive from the UI checked on the server | `.claude/skills/verify-emailops/scripts/imap_ui.mjs` | `make imap-ui` |

## The local servers

`make imap-up` starts three containers (`scripts/imap_test/compose.yml`), on 127.0.0.1 only:

- **Dovecot 2.4**: IMAP with STARTTLS on 31143 and SMTP submission on 31587. Any username logs in with the shared test password and gets an empty mailbox of its own, so each run uses a fresh user. Sent, Drafts, Junk, Trash and Archive are created with their SPECIAL-USE roles.
- **Mailpit** receives what Dovecot submission relays. Its API on 8025 shows what the app sent.
- **Toxiproxy** sits in front of Dovecot IMAP on 21143, with its API on 8474.

TLS uses a test CA that `scripts/imap_test.sh certs` writes under `.imap-test/certs/` (gitignored). A debug build trusts it when `EMAILOPS_IMAP_TEST_CA` names it. A release build ignores the variable. `scripts/imap_test.sh env` prints every variable the tests read. `make imap-down` stops the containers and drops their mail.

The containers listen on IPv4 only. `localhost` resolves to `::1` first, and the client then falls through to `127.0.0.1`. The env uses `127.0.0.1` directly to skip that refused attempt.

## Adding a case

- A new provider behaviour goes in `provider_contract.rs` as a check, then into the `contract_tests!` list in `tests/provider_contract.rs` and the `live_contract!` list in `tests/imap_live.rs`.
- A server quirk that a capability or a fault can express goes in `tests/imap_client.rs`, with `FakeImapServer::with_capabilities`, `with_roles_only_on_request` or `fault(Fault::…)`.
- A quirk seen on a real server goes in a new transcript under `tests/fixtures/imap/`. Paraphrase it into synthetic data first: no real names, addresses or subjects.
