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

## How to get to it (user POV)

- Sidebar → Views → **Inbox** (default view after launch).
- Clicking an account in the sidebar switches the list to that account; **All accounts** merges them.
- A row's ⋮ menu also offers "Open in new tab" and "Chat about this thread".

## Driving it with verify.sh

Preconditions: baseline; account `demo-acct-work` selected (default). `R=$(readlink src-tauri/reports/verify/current)`.

- List renders → `$V wd find 'button=Inbox'` prints one match and `$V wd find '//div[@role="button"][contains(., "Nadia Brunner")]'` prints the first demo row (*How do I add a new email account in EmailOps?*).
- Open → `$V wd shot "$R/inbox-open-before.png"`, `$V wd click '//div[@role="button"][contains(., "Nadia Brunner")]'`, `$V wd shot "$R/inbox-open-after.png"` → `$V wd find 'h1*=How do I add a new email account'` prints the subject and `$V wd exists 'button=Back'` prints `present`.
- Close → `$V wd click 'button=Back'`, `$V wd shot "$R/inbox-close-after.png"` → `$V wd exists 'h1*=How do I add a new email account'` prints `absent` and `$V wd exists 'h2*=Inbox'` prints `present`.
- Categories → `$V wd click 'button=Promotions'` → the row set in `$V wd find 'div[role="button"]'` differs from the Primary one; `$V wd click 'button=Primary'` restores it.

Live run 11/09/2026: open and close proven on the *Nadia Brunner* and *Marisol Vega · Production bug* rows; evidence in `src-tauri/reports/verify/20260911-141357/`.

## Gotchas

- Rows are thread rows: a thread with replies shows only its latest message's subject (`Re: …`); match on sender + a subject fragment, not on the exact original subject.
- The reading pane's exit is **Back** (`button=Back`, title "Back to inbox"), not a Close button; "Close chat panel" belongs to the chat panel.
- Opening a thread did **not** flip `is_read` on an older unread message of that thread in the live run; do not use `emails.is_read` as this feature's side-effect proof without first checking what the app marks.
- The list is virtualised; a row far down needs a scroll (`$V wd js 'document.querySelector("[data-virtual-list], main").scrollBy(0, 800)'` or cua-driver `scroll`) before it is clickable.
