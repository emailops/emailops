# Accounts and sync

Adding, reordering, disabling and removing accounts (Gmail, Outlook, IMAP), and keeping the
local mailbox in step with the provider: new mail, custom folders, read state and deletions in
both directions, drafts, attachments. Removing an account also removes its per-account
preferences and credentials.

## Sub-features

- `accounts.switch` sidebar account rows and **All accounts** change the list scope.
- `accounts.add` / `accounts.remove` add-account dialogs; removal clears mail, sync state, preferences and secrets.
- `sync.incremental` new mail per mailbox, with watermarks and backfill.
- `sync.writeBack` marking read and deleting are pushed to IMAP and Outlook; a failed read push is retried on the next sync (V029 `read_push_pending_since`).
- `sync.stateRefresh` read state and removals made elsewhere reach stored mail (IMAP flags, Outlook, Gmail History API).
- `sync.uidValidity` a changed IMAP UIDVALIDITY re-keys stored mail instead of duplicating it (V027 `folder_uid_validity`).
- `sync.largeAttachments` Outlook attachments over the inline limit go through upload sessions.

## How to get to it (user POV)

- Sidebar → Accounts: a row per account, **All accounts**, **Add account**, **Sync emails**.
- Hover an account row → settings / remove.

## Driving it with verify.sh

Preconditions: baseline. The demo accounts have no credentials: every sync ends in the expected "Authentication required" banner, so only the read-only paths are drivable.

- Switch → `$V wd click 'button*=ulises@fastmail.com'` → the `h2` still says Inbox and the rows change; `$V wd click 'button=All accounts'` → at least as many rows; back with `$V wd click 'button*=ulises@emailopslabs.dev'`.

## Gotchas

- Anything that needs the provider (sync, send, write-back, state refresh) cannot be driven on the demo instance; it is proven against `FakeEmailProvider` in `tests/integration.rs` and against recorded cassettes.
- Live provider behaviour is checked by hand from a dev build on a real account, never in this run.

| Case | Test kind |
|---|---|
| provider clients, payloads, retries, OAuth, MIME, History API, upload sessions | unit (`sync::*`) |
| sync planning, watermarks, UIDVALIDITY, state refresh, write-back, account removal | unit on an in-memory DB (`services::emails::{sync,mailbox_state,state_refresh,history_refresh,uid_validity}`, `services::accounts`) |
| sync end to end against the fake provider | integration (`sync_*`, `*imap*`, `*outlook*`, `*backfill*`, `a_failed_read_push_is_delivered_by_the_next_sync`, `a_renumbered_imap_inbox_…`) |
| V026–V029 present in the test schema | contract (`db::schema_parity_tests`) |
| account and system command arguments | contract (`src/lib/apiContract/cuentas.api.test.ts`) |
| account switching | e2e (`Cuentas/*`) |
| eval | n/a: no model takes part in sync or account management |
