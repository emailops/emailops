# Unsubscribe and block sender

A message with `List-Unsubscribe` offers **Unsubscribe**: the dialog says exactly what will
happen — an RFC 8058 one-click POST to the sender's host, an email from this account, or the
sender's page in the browser — before anything is sent. **Block sender** (row ⋮ menu) stores a
per-account block, marks the sender's mail junk and files it in the provider's Spam; new mail
from them is filed on arrival. Settings → Junk lists blocked senders with Unblock.

## Sub-features

- `sender.unsubscribe` header parsing, the three methods, the confirmation dialog, "Unsubscribed" banner.
- `sender.block` the Block dialog (with "also move existing messages"), the blocked banner in the thread.
- `sender.unblock` Unblock from the banner or Settings → Junk, optionally bringing their mail back and forgetting the junk mark.
- `sender.arrivals` the sync hook that files a blocked sender's new mail as spam.

## How to get to it (user POV)

- Open a newsletter: **Unsubscribe** next to the sender. Row ⋮ → **Block sender**.
- Gear → **Junk** → *Blocked senders*.

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
