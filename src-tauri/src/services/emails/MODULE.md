# services/emails

## What this module owns

All business logic for email read/write operations that cross the provider boundary or require multi-step coordination:

- **sync.rs** — full-account sync (fetch from provider → upsert to DB → trigger embeddings/classification)
- **drafts.rs** — AI draft generation (prompt assembly, Ollama call, DB write)
- **send.rs** — send reply / new email through the provider's SMTP or API
- **provider.rs** — thin wrappers that map provider-neutral `Email` to provider-specific send payloads
- **redownload.rs** — re-fetch emails whose body is empty (e.g. after a failed initial sync)
- **events.rs** — Tauri event emission helpers (`emit_sync_progress`, `emit_sync_complete`)
- **mod.rs** — re-exports the public surface

## Dependencies

- `db/emails` — all SQL queries (reads via `db.reader()`, writes via `db.connection()`)
- `sync/provider.rs` — `EmailProvider` trait for Gmail/IMAP/Fake
- `ai/provider.rs` — `AIProvider` trait for draft generation
- `services/task_queue` — heavy work (sync, draft gen) is submitted here, not awaited inline
- `services/logger` — log seam for output panel events

## Public surface

- `sync_account(db, account_id, app_data_dir, app, ai_background, abort_flags, sync_locks) -> Result<()>`
- `generate_draft(db, email_id, account_id, app, ai_queue) -> Result<String>`
- `send_reply(db, account_id, email_id, body, to, cc, attachments) -> Result<()>`
- `send_new_email(db, account_id, to, subject, body, cc, attachments) -> Result<()>`
- `redownload_email(db, account_id, email_id) -> Result<()>`

## What should NOT live here

- SQL queries — those go in `db/emails/`
- Provider authentication / token refresh — that is `services/accounts`
- Scheduling decisions (when to sync) — that is `services/sync_scheduler`
- Classification / tagging — that is `services/classification`
