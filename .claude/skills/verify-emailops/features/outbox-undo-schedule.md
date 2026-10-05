# Undo send and scheduled send

Every composer sends through a local outbox. With undo send on (Settings → Appearance, 10 s by
default) Send closes the composer and shows "Sending… · Undo" for the window; Undo takes the
message back and reopens it. The chevron next to Send offers Schedule send (tomorrow morning /
afternoon, Monday morning, a custom time) with a note that EmailOps must be open then. The
**Scheduled** view lists waiting and failed messages with Send now / Retry, Edit and Delete.

## Sub-features

- `outbox.undo` queue for the undo window, Undo reopens the composer, a lost race says "already being sent".
- `outbox.schedule` the Send split button's presets and custom date, stored as `origin = 'scheduled'`.
- `outbox.view` the Scheduled view: rows, Send now / Retry, Edit, Delete (with Undo).
- `outbox.failure` a send that fails after the window shows a toast with View and a "Not sent" row.
- `outbox.setting` Settings → Appearance → Undo send (Off, 5–30 s).

## How to get to it (user POV)

Six entry points, the columns of `## Parity`:

- **Compose modal** — Sidebar → **Compose**.
- **Compose tab** — open-in-tab, Drafts → Continue editing, or what Undo / Scheduled → Edit reopens.
- **Reply**, **Reply all**, **Forward** — the buttons of an open thread; the thread hands the message to the outbox.
- **CLI compose --send** — `emailops-cli compose … --send`.
- In each GUI composer: **Send**, or the chevron → **Schedule send**. Sidebar → Views → **Scheduled** lists what is waiting.

## Parity

| Capability | Compose modal | Compose tab | Reply | Reply all | Forward | CLI compose --send |
|---|---|---|---|---|---|---|
| outbox.undo | e2e:Envío/enviar y deshacer | vitest:src/components/EmailView/ComposeTabView.pendingRecipient.test.tsx::queues the message for the undo window, hands over its draft, and closes | gap: untested — the thread queues replies but no test sends a reply through it | gap: untested — same path, no reply-all send test | gap: untested — no test queues a forward | n/a: --send is a one-shot immediate send; there is no window to undo in |
| outbox.schedule | gap: untested — the modal's chevron is never driven; Envío/programar envío runs on the tab that Undo reopened | e2e:Envío/programar envío | gap: untested — the reply composer's Schedule send has no test | gap: untested — same, no reply-all test | gap: untested — same, no forward test | gap: missing — compose has no schedule flag; it sends at once |
| outbox.view | e2e:Envío/vista Scheduled y borrar | n/a: the Scheduled view lists outbox rows the same way whichever composer queued them | n/a: the Scheduled view lists outbox rows the same way whichever composer queued them | n/a: the Scheduled view lists outbox rows the same way whichever composer queued them | n/a: the Scheduled view lists outbox rows the same way whichever composer queued them | gap: missing — no CLI command lists, retries or deletes outbox rows |
| outbox.failure | e2e:Compose/enviar sin credenciales | n/a: the dispatcher and the outbox-updated toast are the same whichever composer queued the message | n/a: the dispatcher and the outbox-updated toast are the same whichever composer queued the message | n/a: the dispatcher and the outbox-updated toast are the same whichever composer queued the message | n/a: the dispatcher and the outbox-updated toast are the same whichever composer queued the message | n/a: --send fails synchronously (error envelope and exit code); nothing is queued to fail later |
| outbox.setting | gap: untested — with Undo send Off the modal's direct send is never driven | vitest:src/components/EmailView/ComposeTabView.pendingRecipient.test.tsx::sends to the pending address | gap: untested — the thread's direct send for replies has no test | gap: untested — same path, no reply-all test | gap: untested — same path, no forward test | n/a: the CLI never reads the undo delay; --send is an explicit immediate send |

## Driving it with verify.sh

Preconditions: baseline; undo send at its default (10 s). Confirmed live on 02/10/2026.

- Undo → Compose, fill To/Subject/body, `$V wd click '[data-testid=compose-send]'` → toast *Sending… · Undo*; `select origin,status from outbox` shows `undo|scheduled`; click **Undo** → the composer reopens (as a tab, `compose-tab-send`) and the row is `cancelled`.
- Schedule → `[data-testid$=-send-schedule]` → `[data-testid*=-send-preset-]` → toast *Scheduled for …*; Scheduled lists the row (*Sends …*); `[data-testid=scheduled-delete]` → toast *Scheduled message deleted · Undo*, row `cancelled`.
- Failure → Send and wait ~12 s → toast *A message could not be sent: Authentication required …*; the row is `failed` and Scheduled shows *Not sent*.

## Gotchas

- The demo has no credentials, so a message left to go out always fails after the window; never use a real recipient.
- Undo reopens the message in a compose **tab**, not the modal: its Send is `compose-tab-send`.
- Finished rows (`sent`/`cancelled`) stay in `outbox` for a while before pruning; match the newest row by subject.

| Case | Test kind |
|---|---|
| send-at planning, queue, cancel race, dispatch, interrupted recovery, pruning | unit (`services::outbox`, `db::outbox`) |
| store send/undo/schedule, presets, Scheduled view, split button, setting | vitest (`outboxStore`, `outbox`, `ScheduledView`, `SendSplitButton`, `UndoSendSetting`) |
| queue → Undo, queue → dispatcher sends once due | integration (`outbox_undo_send_cancels_or_sends_once_the_window_closes`) |
| command arguments and payloads | contract (`src/lib/apiContract/compose.api.test.ts`, `outbox.rs`) |
| Send + Undo, Schedule, Scheduled view + Delete, failure without credentials | e2e (`Envío/*`, `Compose/enviar sin credenciales`, `Vistas/Scheduled`) |
| eval | n/a: no model involved |
