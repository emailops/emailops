# Unsubscribe and block sender

A message with `List-Unsubscribe` offers **Unsubscribe**: the dialog says exactly what will
happen — an RFC 8058 one-click POST to the sender's host, an email from this account, or the
sender's page in the browser — before anything is sent. **Block sender** (row ⋮ menu) stores a
per-account block, marks the sender's mail junk and files it in the provider's Spam; new mail
from them is filed on arrival. Settings → Junk lists blocked senders with Unblock.

## Sub-features

- `sender.unsubscribe` header parsing, the three methods, the confirmation dialog, "Unsubscribed" banner.
- `sender.block` the Block dialog (with "also move existing messages").
- `sender.banner` the blocked / unsubscribed banner in the thread, with Unblock.
- `sender.unblock` Unblock from the banner or Settings → Junk, optionally bringing their mail back and forgetting the junk mark.
- `sender.arrivals` the sync hook that files a blocked sender's new mail as spam.

## How to get to it (user POV)

Four entry points, the columns of `## Parity`:

- **Inbox row ⋮** — **Block sender**.
- **Open thread** — **Unsubscribe** next to the sender, and the blocked / unsubscribed banner.
- **Tag Board** — a card's ⋮ (same menu) and the board's reading pane.
- **Settings → Junk** — Gear → **Junk** → *Blocked senders*.
- Sync files a blocked sender's new mail with no UI entry point.

## Parity

| Capability | Inbox row ⋮ | Open thread | Tag Board | Settings → Junk |
|---|---|---|---|---|
| sender.unsubscribe | gap: missing — the ⋮ menu has no Unsubscribe; only the open message offers it | e2e:Remitentes/Unsubscribe: diálogo y Cancel | gap: untested — the card ⋮ has none; the board pane's Unsubscribe button is never driven | n/a: Settings → Junk manages blocks; an unsubscribe is a one-off request from a message |
| sender.block | e2e:Remitentes/bloquear remitente | gap: missing — the thread toolbar has no Block sender; it is offered only after a confirmed unsubscribe | gap: untested — card ⋮ → Block sender is never driven on the board | n/a: Settings → Junk lists and lifts blocks; a block starts from a message |
| sender.banner | n/a: the banner belongs to the open message, not the row | e2e:Remitentes/aviso de remitente bloqueado en el hilo | gap: untested — the board pane renders the same banner, never checked there | n/a: Settings shows no message |
| sender.unblock | gap: missing — the ⋮ menu offers Block sender even for a blocked sender, never Unblock | vitest:src/components/EmailView/SenderControls.test.tsx::tells a blocked sender apart and offers Unblock | gap: untested — reachable only through the board pane's banner, never driven | e2e:Remitentes/Ajustes → Junk: lista y desbloqueo |
| sender.arrivals | integration:sync_files_new_mail_from_a_blocked_sender_as_spam | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |

## Driving it with verify.sh

Preconditions: baseline. *Harborlight Weekly: shipping notes* carries `List-Unsubscribe` (https + mailto, `.example` hosts)
and `List-Unsubscribe-Post: List-Unsubscribe=One-Click` in `email_headers`. Confirmed live on 02/10/2026.

- Unsubscribe → search `Harborlight`, open it, `[data-testid=unsubscribe-button]` → dialog *Unsubscribe from Harborlight Weekly?* naming `harborlight-weekly.example`; click **Cancel**. Never confirm: it would POST to the sender's host.
- Block → Back, ⋮ → `[data-testid=menu-block-sender]` → `[data-testid=sender-block-confirm]` → toast *Blocked news@harborlight-weekly.example · Messages that could not be moved: 1* (no credentials); `blocked_senders` has the row; the message stays in `inbox` with `email_junk.user_override = 'junk'`; the thread shows `[data-testid=blocked-sender-banner]`.
- Unblock → Settings → **Junk** → `[data-testid=blocked-senders]` → **Unblock** → confirm → toast *Unblocked …*; `blocked_senders` empty and the junk mark gone.

## Gotchas

- With no credentials the block is stored but the existing mail cannot be moved: the toast counts it as "could not be moved" and the mail stays in the inbox, marked junk here. Unblock with "bring their mail back" forgets that mark (fixed 02/10/2026; before, it stayed junk).
- The Unsubscribe confirm button is never pressed by the verifier; the dialog text is the proof.

| Case | Test kind |
|---|---|
| header parsing, method choice, one-click POST guard, block/unblock planning, spam filing | unit (`services::unsubscribe`, `services::sender_controls`, `db::sender_controls`, `services::emails::spam`) |
| store, dialogs, banners, Settings list | vitest (`senderStore`, `SenderDialogs`, `SenderControls`, `BlockedSendersSettings`) |
| a blocked sender's new mail is filed as spam during sync | integration (`sync_files_new_mail_from_a_blocked_sender_as_spam`) |
| command arguments and payloads | contract (`src/lib/apiContract/junk.api.test.ts`, `sender_controls.rs`) |
| Unsubscribe dialog + Cancel, Block, banner, Settings list + Unblock | e2e (`Remitentes/*`) |
| eval | n/a: no model involved |
