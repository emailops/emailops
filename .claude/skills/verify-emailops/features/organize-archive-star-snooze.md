# Archive, star, snooze and multi-select

A conversation can be starred, marked read/unread, archived, deleted or snoozed from its row
(star toggle, ⋮ menu), from the open thread, from the keyboard, or for many rows at once
through the checkboxes and the bulk toolbar. Archive and delete are provider-first and wait
out a 6-second undo window ("Archived 2 conversations · Undo") before anything reaches the
provider; a refusal rolls the rows back with a visible error. Snooze hides the thread until a
chosen time (Snoozed view); Unsnooze brings it back at once.

## Sub-features

- `organize.star` the row's star toggle and the Starred view (stars stay local and are pushed to the provider later, `star_push_pending_since`).
- `organize.archive` Archive / Move to Inbox; IMAP accounts archive into their Archive folder, so the Archive view is offered for Gmail/Outlook and All accounts only.
- `organize.select` row checkboxes, the bulk toolbar (Archive, Snooze, Delete, Mark as read/unread, Star) and Clear selection.
- `organize.undo` the 6 s undo window on archive and delete, and the error + rollback when the provider refuses.
- `organize.advance` after archive, delete, snooze, block or Confirm junk of the open conversation the next one opens (Settings → Appearance → *After archiving or deleting*: next / previous / back to the list; pref `ui.after_thread_leave`). Mark as unread and bulk actions never advance; a slow action never replaces a conversation opened meanwhile.
- `organize.snooze` ⋮ → Snooze presets (later today, tomorrow, this weekend, next week, custom), the Snoozed view with its badge, Unsnooze.

## How to get to it (user POV)

- Inbox row: star at the right, checkbox at the left, ⋮ menu (Archive, Snooze, Mark as unread, Star…).
- Sidebar → Views → **Starred**, **Snoozed**; **Archive** with All accounts or a Gmail/Outlook account.
- Keyboard: `e` archive, `s` star, `b` snooze, `x` select (see the shortcuts feature).

## Driving it with verify.sh

Preconditions: baseline. The demo DB has one starred thread (*Corrected Larkspur Freight renewal quote*) and one
archived message (*Studio key handover confirmed*) in `demo-acct-work` (`insert_verification_fixtures`). Confirmed live on 02/10/2026.

- Star → `$V wd js '[...document.querySelectorAll("div[role=button]")].find(r => r.innerText.includes("Nadia Brunner")).querySelector("[data-testid=star-toggle]").click()'` → its `aria-pressed` turns `true` and `sqlite3 .emailops-demo-data/emailops.db "select is_starred from emails where sender='Nadia Brunner'"` prints 1. Click again to undo.
- Starred → `$V wd click 'button*=Starred'` → `h2` starts with *Starred* and lists both threads.
- Archive view → `$V wd click 'button=All accounts'`, `$V wd click 'button=Archive'` → *Studio key handover confirmed*.
- Multi-select → click `[data-testid=row-select]` in two rows → `[data-testid=bulk-toolbar]` says *2 selected*; its **Archive** removes them and the toast offers **Undo**, which brings them back.
- Archive without credentials → ⋮ → **Archive** on one row and wait ~7 s → toast *Could not archive 1 conversation: … Authentication required …*, the row returns, `emails.mailbox` stays `inbox`.
- Snooze → ⋮ → `[data-testid=row-snooze]` → `[data-testid^=snooze-preset-]` → the row leaves, `thread_snoozes` gains a row; Snoozed lists it with `[data-testid=snooze-badge]`; ⋮ → `[data-testid=row-unsnooze]` removes it.

## Gotchas

- The demo accounts have no credentials: archive/delete are provider-first, so after the undo window they fail and roll back. That error + rollback is the expected result, not a failure of the step.
- Stars and read state are local-first: the row changes at once and `star_push_pending_since` records the pending push, which never succeeds on the demo.
- The "later today" snooze preset disappears late in the day; pick the first preset rather than a fixed one.
- Toasts stack above the Output bar; close them (`[data-testid=toast-stack] button[aria-label=Close]`) between steps so a stale one is not read as the new result.

| Case | Test kind |
|---|---|
| thread action planning, provider-first archive/delete, local-only accounts, star widening | unit (`services::emails::thread_actions`, `db::emails::mailbox_state`) |
| snooze planner, wake pass, snooze rows | unit (`services::emails::snooze`, `db::emails::snoozes`) |
| selection, bulk planning, undo queue, snooze presets, star/selection rows, toasts | vitest (`selectionStore`, `bulkActions`, `pendingActions`, `snooze`, `BulkToolbar`, `SnoozePicker`, `EmailRow.star/.selection`, `emailStore.threadActions/.snooze`, `ToastHost`) |
| archive ↔ inbox through the command entry point; snooze → wake | integration (`thread_action_archive_and_move_to_inbox_round_trip`, `snooze_hides_a_thread_until_the_wake_pass_brings_it_back`) |
| auto-advance planner, executor and race guard, setting, junk confirm race, block | vitest (`autoAdvance`, `autoAdvanceStore`, `AutoAdvanceSetting`, `JunkBanner.autoAdvance`, `EmailView.shortcuts`, `SenderDialogs`) |
| command arguments | contract (`src/lib/apiContract/inbox.api.test.ts`, `emails.rs`) |
| star, Starred, Archive, select, bulk archive + Undo, archive refused, snooze, Snoozed, Unsnooze | e2e (`Organizar/*`, `Vistas/Starred`, `Vistas/Snoozed`) |
| eval | n/a: no model involved |
