# services/emails

## What this module owns

All business logic for email read/write operations that cross the provider boundary or require multi-step coordination:

- **sync.rs** — full-account sync (fetch from provider → upsert to DB → trigger embeddings/classification)
- **drafts.rs** — AI draft generation (prompt assembly, Ollama call, DB write)
- **send.rs** — send reply / new email through the provider's SMTP or API; after the provider accepts, inserts an optimistic local Sent row (via `optimistic.rs`) so the message shows in the thread/Sent views immediately, and for Gmail spawns a background authoritative `get_message` refresh
- **optimistic.rs** — pure planner for the optimistic Sent row: provider-keyed permanent row when the send returned a canonical id (Gmail), synthetic `local-sent-<uuid>` row with `pending_sync = 1` otherwise (Outlook/IMAP)
- **reconcile.rs** — pure planner + thin executor that matches `pending_sync = 1` rows against provider-ingested Sent copies (exact RFC Message-ID for IMAP, conservative subject/recipients/time heuristic for Outlook), deletes the synthetic row, and moves the incoming row into the local thread when thread ids diverged. Hooked after both `insert_emails_batch` sites in sync.rs; a 24h stale sweep at sync start un-flags rows that never matched
- **mailbox_state.rs** — read/unread and delete, plus their provider write-back. Read state is local-first with a best-effort push (reading mail must work offline); delete is provider-first (Gmail `messages.trash`) so a refused push leaves the message visible instead of diverging silently. Gated on `provider_supports_mailbox_writes` — Gmail only today, IMAP/Outlook stay local
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
- `services/events` — UI event seam (`sync-progress`); `None` AppHandle still emits via the seam

## Public surface

- `sync_account(db, account_id, app_data_dir, app: Option<AppHandle>, ai_background, abort_flags, sync_locks) -> Result<()>` — `app=None` (CLI) routes progress through the events seam and skips AppHandle-bound follow-ups (AI tasks, attachments)
- `generate_draft(db, email_id, account_id, app, ai_queue) -> Result<String>`
- `send_reply(db, email_id, body, from_account_id, to, cc, app) -> Result<String>` (returns sending account id for a post-send sync)
- `send_new_email(db, account_id, to, cc, subject, body, attachments, app) -> Result<String>` (returns sending account id for a post-send sync)
- `redownload_email(db, account_id, email_id) -> Result<()>`
- `mark_as_read(db, email_id, app) -> Result<()>` / `delete_email(db, email_id, app) -> Result<()>` — command entry points that resolve the provider themselves; the `*_with_provider` variants take an injected provider (`None` = local-only) and are what tests drive

## What should NOT live here

- SQL queries — those go in `db/emails/`
- Provider authentication / token refresh — that is `services/accounts`
- Scheduling decisions (when to sync) — that is `services/sync_scheduler`
- Classification / tagging — that is `services/classification`
