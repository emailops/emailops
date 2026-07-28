# sync

## What this module owns

Provider-specific email synchronisation adapters plus the shared `EmailProvider` trait.

- **provider.rs** — `EmailProvider` trait + `FakeEmailProvider` test double. The trait defines the minimal surface all sync paths share: `list_messages`, `get_message`, `send_reply`, `send_new_email`, `download_attachment`, plus default-empty folder methods (`list_folders`, `list_folder_messages`) that only the IMAP adapter implements — Gmail/Outlook silently skip custom-folder sync. Folder *management* (`create_folder`, `rename_folder`, `delete_folder`, `move_message` with a `MoveTarget`) defaults to a typed "unsupported" error; only IMAP overrides it, and `services/emails/folders.rs` owns the orchestration (provider-first, then in-place local migration). Sends return a best-effort `SentMessageMeta` (Gmail: provider `id`/`threadId` from the send response; IMAP: the lettre-generated RFC Message-ID; Outlook: empty — Graph returns 202 with no body) so `services/emails` can insert an optimistic local Sent row. The fake's `set_send_meta` simulates each shape; `set_folders` simulates a server `LIST` response; folder ops mutate its state and are recorded for assertions (`folder_ops`, `set_move_result`).
- **folder_plan.rs** — pure planner for IMAP `LIST` responses: modified UTF-7 decoding *and encoding*, SPECIAL-USE (RFC 6154) role mapping with a localized name-candidate fallback (en/de/es/fr — "Gesendete Objekte", "Papierkorb", …), and classification of the remaining selectable folders as custom folders. Also owns the pure folder-management helpers: `validate_folder_name`, `compose_folder_path` (INBOX-nesting heuristic), `rename_sibling_path`. No I/O; `imap.rs` resolves Sent/Spam/Trash through it (including the sent-copy APPEND target) and `services/emails/sync.rs` persists its plan into the `folders` table.
- **draft_plan.rs** — pure planner for the draft pull pass. `plan_draft_fetches(listed, known)` splits a provider's cheap draft listing into `to_fetch` (content actually changed) and `present_ids` (everything upstream, the prune keep-list). Providers report a per-draft `change_token`; Gmail uses `draft.message.id`, which it re-mints on every save, so an unchanged id proves the content is untouched. No I/O.
- **gmail.rs** — Gmail REST API adapter (OAuth 2, incremental sync via `after:` filter, attachment handling, category mapping)
- **outlook.rs** — Microsoft Graph API adapter (OAuth 2, delta sync, category mapping)
- **imap.rs** — IMAP/SMTP adapter (native-TLS, IDLE for push notifications, SMTP for send). Message ids encode their source folder: `{account}::{uid}` (INBOX), `SENT::`/`SPAM::`/`TRASH::` sub-prefixes (canonical roles, frozen for backward compat), and `FOLDER::{b64url(server_path)}::{uid}` for custom folders (base64url shields `::` and non-ASCII in folder paths; `folder_email_id_prefix` exposes the shared prefix for rename migrations). Folder management maps to `CREATE`/`RENAME`/`DELETE`, and `move_message` uses `UID MOVE` when the capability is advertised (else `COPY` + `\Deleted` + `UID EXPUNGE`/`EXPUNGE`), then resolves the message's new UID in the target folder via `UID SEARCH HEADER Message-ID`.
- **oauth.rs** — shared OAuth 2 helpers (PKCE flow, token refresh, callback listener)
- **calendar_provider.rs** — `CalendarProvider` trait + `FakeCalendarProvider`. One method (`list_events(window)`) returning provider-expanded recurrence instances; v1 has no sync tokens (full-window fetch each cycle — see `services/calendar/sync.rs`).
- **gmail_calendar.rs** — Google Calendar v3 adapter (`events.list` with `singleEvents=true`; pure `parse_google_event`)
- **outlook_calendar.rs** — Graph calendar adapter (`/me/calendarView` with `Prefer: outlook.timezone="UTC"`; pure `parse_graph_event`)
- **mod.rs** — re-exports

## Key design decisions

- **`EmailProvider` is `async` and `Send + Sync`.** Fakes can be constructed without a network; real impls hit the provider API.
- **Incremental sync is the norm.** Every real adapter queries with a `since_timestamp` so a second sync on an up-to-date mailbox returns 0 messages, not 50 000. Drafts follow the same rule via `list_drafts(known_change_tokens)`: Gmail's listing is cheap but its per-draft read is not, so a steady-state pull is one `drafts.list` and zero `drafts.get`. Adapters must still return *every* upstream draft id in `present_ids` — it is the keep-list for `prune_provider_drafts`, so a truncated list deletes local drafts. Both adapters therefore paginate their draft listing to exhaustion (`nextPageToken` / `@odata.nextLink`), and both error out at `MAX_DRAFT_PAGES` rather than returning a truncated list. Graph returns full bodies in the listing, so Outlook reports every draft as changed and tracks no token.
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
