# Contacts and Dashboard

Contacts groups the people and companies the account exchanges mail with. The Dashboard shows
per-account counters, storage and the background queues (sync, AI work) with their progress.

## Sub-features

- `contacts.list` one entry per address, grouped by company, with detail.
- `dashboard.accounts` per-account counts, storage, last sync.
- `dashboard.queues` running and queued background tasks; AI tasks stop cooperatively when asked.

## How to get to it (user POV)

- Sidebar → Other Views → **Contacts**, **Dashboard**.

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
