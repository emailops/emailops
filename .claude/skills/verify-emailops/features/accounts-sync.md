# Accounts and sync

Adding, reordering, disabling and removing accounts (Gmail, Outlook, IMAP), and keeping the
local mailbox in step with the provider: new mail, custom folders, read state and deletions in
both directions, drafts, attachments. Removing an account also removes its per-account
preferences and credentials.

## Sub-features

- `accounts.switch` sidebar account rows and **All accounts** change the list scope.
- `accounts.add` the add-account dialogs (Gmail, Outlook, IMAP) create an account and start its first sync.
- `accounts.remove` Account settings → Delete account removes the account with its mail, sync state, preferences and secrets.
- `sync.incremental` new mail per mailbox, with watermarks and backfill.
- `sync.writeBack` marking read and deleting are pushed to IMAP and Outlook; a failed read push is retried on the next sync (V029 `read_push_pending_since`).
- `sync.stateRefresh` read state and removals made elsewhere reach stored mail (IMAP flags, Outlook, Gmail History API).
- `sync.uidValidity` a changed IMAP UIDVALIDITY re-keys stored mail instead of duplicating it (V027 `folder_uid_validity`).
- `sync.largeAttachments` Outlook attachments over the inline limit go through upload sessions.
- `accounts.reorder` ↑ / ↓ on an account row changes the account order.
- `accounts.disable` an account can be disabled and re-enabled; a disabled account neither syncs nor appears in All accounts.
- `accounts.settings` Account settings edits the sender name and the sync range.
- `sync.manual` **Sync emails** starts a sync of the selected account (or all of them in All accounts) on demand.

## How to get to it (user POV)

Four entry points, the columns of `## Parity`:

- **Sidebar** — Accounts section: a row per account, **All accounts**, **+ Add account**, the header **Sync emails** button. Hovering a row shows ↑ / ↓ / Enable-Disable / Account settings; the settings dialog holds sender name, sync range and **Delete account**.
- **Onboarding wizard** — the first-run "Add account" step (OAuth, IMAP dialog, first sync, settings dialog).
- **Dashboard** — one panel per account; its settings button opens the same Account settings dialog.
- **CLI** — `emailops-cli accounts`, `accounts add gmail|outlook|imap`, `sync [account]`; every command takes `--account`.

## Parity

| Capability | Sidebar | Onboarding wizard | Dashboard | CLI |
|---|---|---|---|---|
| accounts.switch | e2e:Cuentas/cambiar a fastmail | n/a: the wizard runs before the mailbox UI exists | n/a: the Dashboard shows every account side by side; it has no list scope | n/a: every CLI command takes --account; there is no persistent list scope |
| accounts.add | vitest:src/components/Sidebar/AddAccountModal.test.tsx::keeps a failed sign-in contained and re-enables the button for a retry | gap: untested — StepAddAccount (OAuth start, IMAP hand-off, first sync) has no test | n/a: the Dashboard reports on configured accounts; adding is offered in the sidebar and the wizard | gap: untested — only clap parsing is tested; add_account never runs in a test |
| accounts.remove | gap: untested — the delete confirmation opens in a test but is never confirmed; nothing proves Confirm reaches removeAccount | gap: untested — the wizard's onDelete has no test | gap: untested — the panel's settings button opens the same dialog; no test clicks it | gap: missing — there is no `accounts remove` subcommand |
| sync.incremental | integration:sync_with_provider_incremental_uses_latest_timestamp | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| sync.writeBack | integration:a_failed_read_push_is_delivered_by_the_next_sync | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| sync.stateRefresh | rust:src-tauri/src/services/emails/state_refresh.rs::a_message_read_in_another_client_becomes_read_here | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| sync.uidValidity | integration:a_renumbered_imap_inbox_keeps_its_rows_and_still_receives_new_mail | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| sync.largeAttachments | rust:src-tauri/src/sync/outlook.rs::a_large_attachment_is_uploaded_in_ranges_to_a_draft_that_is_then_sent | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| accounts.reorder | gap: untested — ↑ / ↓ on a hovered row have no test or step | n/a: the wizard adds one account at a time | n/a: the Dashboard lists accounts in sidebar order and has no ordering control | n/a: order is a sidebar display preference; the CLI addresses accounts by id or email |
| accounts.disable | gap: untested — the row's Enable/Disable button and the dialog's switch have no test | gap: untested — the wizard wires the toggle into the same dialog; no test | gap: untested — reachable through the panel's settings dialog, never driven | gap: missing — no enable/disable subcommand |
| accounts.settings | vitest:src/components/Sidebar/AccountSettingsDialog.senderName.test.tsx::saves a changed sender name | gap: untested — the wizard opens the dialog after adding; no test | gap: untested — the panel's settings button is never clicked in a test | gap: missing — no command edits the sender name or sync range after `accounts add` |
| sync.manual | gap: untested — the header Sync emails button is never clicked by a test or step | gap: untested — the wizard's first sync after adding has no test | n/a: the panel shows last sync and queues; syncing is started from the sidebar header | gap: untested — only the unknown-account error path is dispatched in a test |

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
