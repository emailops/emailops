# Inbox: open and close a thread

The inbox lists the selected account's threads newest first, one row per thread with
sender, subject snippet, tags and date. Clicking a row replaces the list with the thread
(subject as H1, messages below, Reply / Reply All / AI Draft toolbar); **Back** returns to
the list.

## Sub-features

- `inbox.list` rows render sender + subject for the selected account.
- `inbox.categories` the Primary/Social/Updates/Forums/Promotions tabs filter the list.
- `inbox.open` clicking a row shows the thread with its subject as the page H1.
- `inbox.close` Back hides the thread and shows the list again.
- `inbox.rowMenu` a row's ⋮ menu offers Chat about this thread, Open in new tab, sender filter, hide from smart filters, block sender (with confirmation), read/unread, star, snooze, archive, rules, copy id, redownload, move (IMAP) and delete thread.
- `inbox.toolbar` an open thread shows Reply, Reply All, Forward and AI Draft plus archive, snooze, read/unread, star, search-in-thread and delete; tooltips name the shortcut key.
- `inbox.openInTab` "Open in new tab" (row ⋮ or the thread header) opens the thread in its own tab next to the main one; the tab's ✕ closes it.
- `inbox.remoteImages` remote images are blocked with "Show images" / "Always trust this sender", and the frame's CSP allows only `data: blob: cid:`.

## How to get to it (user POV)

Five ways to reach a thread, the columns of `## Parity`; every opener renders the same `ReadingPane` → `EmailView`:

- **Inbox (one account)** — Sidebar → Views → **Inbox** with one account selected (default view after launch).
- **All accounts** — Sidebar → Accounts → **All accounts**: one list, each row with an account-colour stripe.
- **Tag Board** — Sidebar → **Tag Board**; clicking a card opens the thread in a reading pane beside the board.
- **Search overlay (⌘K)** — sidebar **Search emails…** or ⌘K; clicking a hit opens that thread in the inbox.
- **Chat source** — a source/citation pill or `email://` link in a chat answer opens the thread as a tab in the reading pane.

## Parity

| Capability | Inbox (one account) | All accounts | Tag Board | Search overlay (⌘K) | Chat source |
|---|---|---|---|---|---|
| inbox.list | e2e:Inbox/lista inicial | e2e:Cuentas/All accounts | n/a: the board lists tag blocks, not the inbox list (tagboard.blocks / tagboard.rows) | n/a: the overlay lists search hits (search.results), not the inbox | n/a: a pill names one email; there is no list |
| inbox.categories | gap: untested — e2e:Inbox/pestañas de categoría always SKIPs on the IMAP demo accounts and no Inbox component test clicks a tab | gap: untested — unified mode shows the tabs when any enabled account is not IMAP; no test or step | gap: untested — the toolbar's category chips have no test or step | n/a: the overlay searches every category by design | n/a: a pill opens one thread; there is no list to filter |
| inbox.open | e2e:Inbox/abrir hilo | gap: untested — no step opens a row while All accounts is selected | e2e:Tag Board/abrir hilo desde un bloque | gap: untested — clicking a hit has no test or step | gap: untested — the pills open a thread tab; MarkdownContent tests render pills but never click one |
| inbox.close | e2e:Inbox/volver con Back | gap: untested — Back is never driven in unified mode | gap: untested — the board's pane closes with ✕ Close; the sweep clicks it without asserting the pane closed | gap: untested — Back after opening from the overlay is not driven | gap: untested — closing the thread tab has no test |
| inbox.rowMenu | e2e:Inbox/menú ⋮ de una fila | gap: untested — the menu is not opened in unified mode | gap: untested — cards render the same EmailActionsMenu; TagEmailCard.testids only checks data attributes | n/a: overlay hits are a picker with no per-row actions; picking one lands in the list, whose rows have the menu | n/a: a pill is a link to one email, not a row |
| inbox.toolbar | e2e:Inbox/menú del hilo | gap: untested — not driven in unified mode | gap: untested — same EmailView via ReadingPane, never checked there | gap: untested — not checked after opening from the overlay | gap: untested — not checked in a thread tab |
| inbox.openInTab | gap: untested — the sweep only sees the Open in new tab label in the ⋮ menu; nothing opens or closes the tab | gap: untested — not driven in unified mode | gap: missing — the board's reading pane gets no onOpenInTab, so its header has no Open in new tab; only the card ⋮ offers it | gap: untested — the overlay opens into the inbox pane, whose header has the button | n/a: the pill already opens the thread in its own tab |
| inbox.remoteImages | e2e:Inbox/imágenes remotas bloqueadas | gap: untested — not driven in unified mode | gap: untested — not driven from a Tag Board card | gap: untested — not driven from an overlay hit | gap: untested — not driven in a thread tab |

## Driving it with verify.sh

Preconditions: baseline; account `demo-acct-work` selected (default). `R=$(readlink src-tauri/reports/verify/current)`.

- List renders → `$V wd find 'button=Inbox'` prints one match and `$V wd find '//div[@role="button"][contains(., "Nadia Brunner")]'` prints the first demo row (*How do I add a new email account in EmailOps?*).
- Open → `$V wd shot "$R/inbox-open-before.png"`, `$V wd click '//div[@role="button"][contains(., "Nadia Brunner")]'`, `$V wd shot "$R/inbox-open-after.png"` → `$V wd find 'h1*=How do I add a new email account'` prints the subject and `$V wd exists 'button=Back'` prints `present`.
- Close → `$V wd click 'button=Back'`, `$V wd shot "$R/inbox-close-after.png"` → `$V wd exists 'h1*=How do I add a new email account'` prints `absent` and `$V wd exists 'h2*=Inbox'` prints `present`.
- Categories → `$V wd click 'button=Promotions'` → the row set in `$V wd find 'div[role="button"]'` differs from the Primary one; `$V wd click 'button=Primary'` restores it.
- Remote images → search `Harborlight`, open *Harborlight Weekly: shipping notes* → the page says *Remote images were blocked to protect your privacy.* with **Show images** and **Always trust this sender**; `$V wd js 'document.querySelector("iframe[title=\\"Email content\\"]").getAttribute("srcdoc").match(/<img[^>]*>/)[0]'` prints the `<img>` without its `src`, and the frame's CSP allows only `img-src data: blob: cid:`. Do not click **Show images** (it asks the sender's server). Confirmed live on 30/09/2026.

Live run 11/09/2026: open and close proven on the *Nadia Brunner* and *Marisol Vega · Production bug* rows; evidence in `src-tauri/reports/verify/20260911-141357/`.

## Gotchas

- Rows are thread rows: a thread with replies shows only its latest message's subject (`Re: …`); match on sender + a subject fragment, not on the exact original subject.
- The reading pane's exit is **Back** (`button=Back`, title "Back to inbox"), not a Close button; "Close chat panel" belongs to the chat panel.
- Opening a thread did **not** flip `is_read` on an older unread message of that thread in the live run; do not use `emails.is_read` as this feature's side-effect proof without first checking what the app marks.
- The list is virtualised; a row far down needs a scroll (`$V wd js 'document.querySelector("[data-virtual-list], main").scrollBy(0, 800)'` or cua-driver `scroll`) before it is clickable.

| Case | Test kind |
|---|---|
| list queries, thread order, soft delete, mailbox state, redownload | unit (`db::emails::*`, `services::emails::*`) |
| HTML sanitising, remote content blocking (images, background images, media) | unit + vitest (`util::html`, `emailFormatting`, `EmailHtmlFrame.*`, `EmailPreviewById.*`) |
| stale responses, auto-select, navigation | vitest (`emailStore.*`, `Inbox.*`) |
| lists, threads, read state, deletes against the in-memory DB | integration (`*email*`, `*thread*`, `*mailbox*`, `mark_as_read_*`) |
| email command arguments | contract (`src/lib/apiContract/inbox.api.test.ts`) |
| list, open, back, row menu, remote images blocked, Sent / Spam / Deleted / Attachments views | e2e (`Inbox/*`, `Vistas/*`) |
| eval | n/a: reading mail has no model reply of its own; questions about an open thread are chat evals (`thread_*`) |
