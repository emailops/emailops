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

- Any composer (new message modal, compose tab, inline reply): **Send**, or the chevron → **Schedule send**.
- Sidebar → Views → **Scheduled**.

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
