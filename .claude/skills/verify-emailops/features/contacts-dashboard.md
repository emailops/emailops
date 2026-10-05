# Contacts and Dashboard

Contacts groups the people and companies the account exchanges mail with. The Dashboard shows
per-account counters, storage and the background queues (sync, AI work) with their progress.

## Sub-features

- `contacts.list` one entry per address, grouped by company, with detail.
- `dashboard.accounts` per-account counts, storage, last sync.
- `dashboard.queues` running and queued background tasks with progress.
- `dashboard.stop` AI tasks stop cooperatively when asked.
- `contacts.actions` from a contact, compose to them or see their emails in the inbox.

## How to get to it (user POV)

Four entry points, the columns of `## Parity`:

- **Contacts view** — Sidebar → Other Views → **Contacts** (companies, contact rows, detail panel).
- **Dashboard view** — Sidebar → Other Views → **Dashboard** (account panels, storage, queues).
- **Logs status bar** — the bottom bar shows running background work; its model selector stops AI work before a model change.
- **Chat** — the `search_contacts` tool resolves people from the stored contacts.

## Parity

| Capability | Contacts view | Dashboard view | Logs status bar | Chat |
|---|---|---|---|---|
| contacts.list | e2e:Vistas/Contacts | n/a: the Dashboard shows accounts, not people | n/a: the status bar shows running work only | rust:src-tauri/src/services/chat/tools/mod.rs::search_contacts_resolves_name_plus_domain |
| contacts.actions | gap: untested — Compose to and View emails from have no test; ContactsView has no test at all | n/a: the Dashboard has no per-contact actions | n/a: the status bar has no per-contact actions | n/a: drafting to a person from the chat is Compose's generate_email_draft |
| dashboard.accounts | n/a: Contacts lists people, not account counters | vitest:src/components/Dashboard/AccountPanel.name.test.tsx::shows the account name | n/a: the status bar shows running work, not counters | n/a: counters and storage are a report view; the chat answers about mail content |
| dashboard.queues | n/a: Contacts shows no background work | vitest:src/components/Dashboard/QueuePanel.keys.test.tsx::renders merged per-account sync queues without a duplicate-key error | vitest:src/components/LogPanel/BackgroundActivityStatus.test.tsx::shows a backfill with its progress | n/a: the chat shows its own turn progress, not the queues |
| dashboard.stop | n/a: Contacts shows no background work | gap: missing — QueuePanel lists running and queued tasks but has no stop control | vitest:src/components/LogPanel/ModelSelector.test.tsx::changes the chat model once the work was stopped | n/a: a chat turn has its own stop, not a queue task |

## Driving it with verify.sh

Preconditions: baseline.

- `$V wd click 'button*=Contacts'` and `$V wd click 'button*=Dashboard'` render content without error text.

## Gotchas

- The queues are empty on the demo instance (no credentials, nothing to sync), so progress rows are not drivable there.

| Case | Test kind |
|---|---|
| contact aggregation, dashboard counters, storage | unit (`services::contacts`, `services::dashboard`, `services::storage_stats`) |
| queue: order, names, progress, cooperative cancellation | unit (`services::task_queue`) |
| dispatcher records tasks in order | integration (`fake_dispatcher_*`) |
| contacts and dashboard command arguments | contract (`src/lib/apiContract/contactos.api.test.ts`) |
| the two views | e2e (`Vistas/Contacts`, `Vistas/Dashboard`) |
| eval | n/a: neither view has a model reply; both are SQL aggregations and queue snapshots |
