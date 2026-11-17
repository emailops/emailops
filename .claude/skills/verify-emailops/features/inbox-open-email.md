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
- `inbox.unread` the header's **All / Unread** toggle lists only conversations with an unread message (flat views: unread messages), in every mailbox view and sidebar filter; hidden during a free-text search.
- `inbox.columns` the folder sidebar and (split view) the email list resize from the line on their right edge: drag, arrows / Home / End, double-click for the default; a column stops growing when the reading pane reaches 320 px; when the right dock opens or the window narrows, a widened column gives its extra width back (list first, down to its default) and gets it back when the dock closes; a drag saves once, on release; the widths survive a relaunch.

## How to get to it (user POV)

- Sidebar → Views → **Inbox** (default view after launch).
- Clicking an account in the sidebar switches the list to that account; **All accounts** merges them.
- A row's ⋮ menu also offers "Open in new tab" and "Chat about this thread".
- The list header's **All | Unread** segmented control (`role="radiogroup"`, aria `Show all or only unread emails`).

## Driving it with verify.sh

Preconditions: baseline; account `demo-acct-work` selected (default). `R=$(readlink src-tauri/reports/verify/current)`.

- List renders → `$V wd find 'button=Inbox'` prints one match and `$V wd find '//div[@role="button"][contains(., "Nadia Brunner")]'` prints the first demo row (*How do I add a new email account in EmailOps?*).
- Open → `$V wd shot "$R/inbox-open-before.png"`, `$V wd click '//div[@role="button"][contains(., "Nadia Brunner")]'`, `$V wd shot "$R/inbox-open-after.png"` → `$V wd find 'h1*=How do I add a new email account'` prints the subject and `$V wd exists 'button=Back'` prints `present`.
- Close → `$V wd click 'button=Back'`, `$V wd shot "$R/inbox-close-after.png"` → `$V wd exists 'h1*=How do I add a new email account'` prints `absent` and `$V wd exists 'h2*=Inbox'` prints `present`.
- Categories → `$V wd click 'button=Promotions'` → the row set in `$V wd find 'div[role="button"]'` differs from the Primary one; `$V wd click 'button=Primary'` restores it.
- Remote images → search `Harborlight`, open *Harborlight Weekly: shipping notes* → the page says *Remote images were blocked to protect your privacy.* with **Show images** and **Always trust this sender**; `$V wd js 'document.querySelector("iframe[title=\\"Email content\\"]").getAttribute("srcdoc").match(/<img[^>]*>/)[0]'` prints the `<img>` without its `src`, and the frame's CSP allows only `img-src data: blob: cid:`. Do not click **Show images** (it asks the sender's server). Confirmed live on 30/09/2026.

- Unread → `$V wd click 'button=Unread'` → `$V wd js '[...document.querySelectorAll("[role=radio]")].map(b=>b.textContent+":"+b.getAttribute("aria-checked")).join(",")'` prints `"All:false,Unread:true"`; `$V wd js 'document.querySelectorAll("div[role=button]").length'` prints the unread thread count of the view (6 in `demo-acct-work`'s inbox on a fresh demo DB: Nadia Brunner, Marisol Vega, Juan Ramírez, two BorgBase, Hetzner) and the list ends in *No more emails*. Open the Nadia Brunner row and go **Back**: the row is still listed, now read. `$V wd click 'button=All'` restores the full list.
- Unread on a filter → type `tag:priority=urgent` in the list search + Enter → the toggle is `present` and Unread/All list 1/2 rows; type a plain word (`invoice`) instead → `$V wd exists '[role="radiogroup"]'` prints `absent`.

### `inbox.columns`

Preconditions: baseline. The demo DB ships `inbox_layout = full-width`: switch to **Split view** in Settings → Appearance first and put **Full-width list** back at the end. The handles are `[role=separator]` labelled *Resize the folder list* and *Resize the email list* (the third one, *Resize chat panel*, is the right dock's). Measure with `getBoundingClientRect().width` of `aside.theme-chrome`, the list handle's `previousElementSibling` and `[data-reading-pane]`.

1. Keyboard → focus a handle and dispatch `keydown` `End` / `ArrowLeft` (`$V wd js`). With the right dock open on the 1200 px demo window the reading pane is 180 px, already under 320: End leaves the width alone, ArrowLeft takes 16 px off and saves it.
2. Drag → close the dock (`Close chat panel`), then in three `$V wd js` calls (React needs a turn between them): `pointerdown` on the handle (`new PointerEvent(…, { clientX, button: 0, pointerId: 1, isPrimary: true })`), a series of `pointermove` on `window`, `pointerup` on `window`. Between the moves and the release, `sqlite3 .emailops-demo-data/emailops.db "SELECT value FROM user_preferences WHERE key='layout.list_width'"` still prints the old width and `[data-testid=column-resize-drag-capture]` exists; after the release the row holds the new width and the layer is gone. `__TAURI_INTERNALS__.invoke` is read-only, so the database read is the way to see that a drag writes once.
3. Floor → drag or End until the column stops: `[data-reading-pane]` is exactly 320 px wide; a sidebar drag with the pane at 320 does not move; with room left the sidebar stops at its 420 px maximum.
4. Double-click → `pointerdown` + `pointerup` without moving (no capture layer appears), then a `dblclick` on the handle: the widths go back to 256 / 384.
5. `$V cleanup`, `$V launch` → the handles' `aria-valuenow` show the saved widths.
6. Dock reopened → with the dock closed, End on both handles (sidebar first); then open the dock (`Open chat` in the list header or the rail): the list's `aria-valuenow` drops to 384 before the sidebar's moves, neither goes below its default (256 / 384), the reading pane keeps `max(320, what the defaults leave)` px and the list is not clipped (its right edge ≤ the pane's left edge). The `layout.*_width` rows keep the widened values; closing the dock brings them back.

Live run 17/11/2026 (T-0064, PR #134 takeover): steps 1–5 passed (pane 180 → End no-op; drag 368 → 584 with the DB at 368 mid-drag; pane 320 at every stop; sidebar capped at 420; double-click 256 / 384; 420 / 420 after relaunch); evidence in `src-tauri/reports/verify/20261117-084349/`. Not covered by the floor in that run: reopening the dock (or narrowing the window) after widening the columns left the pane at 0 px and clipped the list's right edge by about 20 px; `fitColumns` (step 6) fixed it afterwards. The wheel handoff over a wide email has no live check: cua-driver's scroll sends keys, not wheel events; the bridge tests in `EmailHtmlFrame.zoom.test.ts` cover it.

Live run 14/10/2026 (T-0014): Unread proven on the work inbox (6 rows), the unified inbox (4 rows under the Primary tab), a `tag:` filter (1 vs 2) and Sent (0 vs 12); opening a thread kept it listed; evidence in `src-tauri/reports/verify/20261014-155030/`.

Live run 11/09/2026: open and close proven on the *Nadia Brunner* and *Marisol Vega · Production bug* rows; evidence in `src-tauri/reports/verify/20260911-141357/`.

## Gotchas

- Rows are thread rows: a thread with replies shows only its latest message's subject (`Re: …`); match on sender + a subject fragment, not on the exact original subject.
- The reading pane's exit is **Back** (`button=Back`, title "Back to inbox"), not a Close button; "Close chat panel" belongs to the chat panel.
- Opening a thread did **not** flip `is_read` on an older unread message of that thread in the live run; do not use `emails.is_read` as this feature's side-effect proof without first checking what the app marks.
- Off-Space window (`document.visibilityState` is `hidden`): the toggle's `transition-colors` never finishes, so a screenshot can show the old option highlighted while `aria-checked` is already right. Read `aria-checked`, not the colour.
- An empty Unread list says *No unread emails* only when no sync is running: *Syncing emails...* wins, and on the demo instance the credential-less accounts can keep a sync up for a while (Sent stayed on it in the 14/10 run).
- The list is virtualised; a row far down needs a scroll (`$V wd js 'document.querySelector("[data-virtual-list], main").scrollBy(0, 800)'` or cua-driver `scroll`) before it is clickable.

| Case | Test kind |
|---|---|
| list queries, thread order, soft delete, mailbox state, redownload | unit (`db::emails::*`, `services::emails::*`) |
| HTML sanitising, remote content blocking (images, background images, media) | unit + vitest (`util::html`, `emailFormatting`, `EmailHtmlFrame.*`, `EmailPreviewById.*`) |
| stale responses, auto-select, navigation | vitest (`emailStore.*`, `Inbox.*`) |
| unread filter: list + count + filters SQL, thread-level read dot, toggle, refetch | unit (`db::emails::crud` / `search` / `mailbox_state` `*unread*`, `services::emails::list_tests`) + vitest (`emailStore.unreadOnly`, `useEmails`, `Inbox.unreadToggle`) |
| lists, threads, read state, deletes against the in-memory DB | integration (`*email*`, `*thread*`, `*mailbox*`, `mark_as_read_*`) |
| email command arguments | contract (`src/lib/apiContract/inbox.api.test.ts`) |
| list, open, back, row menu, remote images blocked, Sent / Spam / Deleted / Attachments views | e2e (`Inbox/*`, `Vistas/*`) |
| column widths: limits, reading-pane floor, fit to a shrinking space, drag saved once, keyboard, double-click | vitest (`src/lib/columnResize`, `ColumnResizeHandle`, `useColumnSpace`) |
| eval | n/a: reading mail has no model reply of its own; questions about an open thread are chat evals (`thread_*`) |
