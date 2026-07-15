# sync

## What this module owns

Provider-specific email synchronisation adapters plus the shared `EmailProvider` trait.

- **provider.rs** — `EmailProvider` trait + `FakeEmailProvider` test double. The trait defines the minimal surface all sync paths share: `list_messages`, `get_message`, `send_reply`, `send_new_email`, `download_attachment`. Sends return a best-effort `SentMessageMeta` (Gmail: provider `id`/`threadId` from the send response; IMAP: the lettre-generated RFC Message-ID; Outlook: empty — Graph returns 202 with no body) so `services/emails` can insert an optimistic local Sent row. The fake's `set_send_meta` simulates each shape.
- **gmail.rs** — Gmail REST API adapter (OAuth 2, incremental sync via `after:` filter, attachment handling, category mapping)
- **outlook.rs** — Microsoft Graph API adapter (OAuth 2, delta sync, category mapping)
- **imap.rs** — IMAP/SMTP adapter (native-TLS, IDLE for push notifications, SMTP for send)
- **oauth.rs** — shared OAuth 2 helpers (PKCE flow, token refresh, callback listener)
- **mod.rs** — re-exports

## Key design decisions

- **`EmailProvider` is `async` and `Send + Sync`.** Fakes can be constructed without a network; real impls hit the provider API.
- **Incremental sync is the norm.** Every real adapter queries with a `since_timestamp` so a second sync on an up-to-date mailbox returns 0 messages, not 50 000.
- **IMAP is blocking.** The IMAP crate uses a blocking API; calls run in `spawn_blocking`. IDLE notifications arrive via a `mpsc::Sender<bool>` so the async sync path can await them.

## Dependencies

- `services/accounts` — OAuth token retrieval / refresh
- `services/keychain` — keychain seam for token persistence
- `db/emails` — upsert helpers called after a successful fetch
- `models/` — `Email`, `EmailAttachment`, `Account` types

## What should NOT live here

- Business logic (draft generation, classification) — those are in `services/`
- DB schema — that is `db/schema.rs`
- Sync scheduling (when to sync) — that is `services/sync_scheduler`
