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
- `organize.read` ⋮ / toolbar / bulk / `Shift+U`·`Shift+I` mark a conversation read or unread (local-first like stars).
- `organize.delete` Delete thread from ⋮, the toolbar, the bulk bar or `#`; provider-first behind the undo window (`organize.undo`).

## How to get to it (user POV)

Six entry points, the columns of `## Parity`:

- **Inbox row** — star at the right, checkbox at the left, ⋮ menu (Archive, Snooze, Mark as unread, Star…).
- **Open thread** — the reading pane toolbar: Archive, Snooze/Unsnooze, read/unread, Star, Delete.
- **Multi-select bar** — checkboxes, then the bulk toolbar.
- **Keyboard** — `e` archive, `s` star, `b` snooze, `x` select (see the shortcuts feature).
- **Tag Board** — a card's ⋮ menu (same menu) and the board's reading pane.
- **All accounts** — the unified inbox.
- Views: Sidebar → Views → **Starred**, **Snoozed**; **Archive** with All accounts or a Gmail/Outlook account.

## Parity

| Capability | Inbox row | Open thread | Multi-select bar | Keyboard | Tag Board | All accounts |
|---|---|---|---|---|---|---|
| organize.star | e2e:Organizar/destacar desde la fila | gap: untested — the toolbar star is never clicked; EmailView.shortcuts only sends the pane command | gap: untested — bulk Star/Unstar is never clicked; the sweep only checks the button exists | e2e:Atajos/s destaca la conversación del cursor | gap: untested — cards have no star toggle; card ⋮ → Star and the board pane's star are never driven | gap: untested — no test or step stars a row in unified mode |
| organize.archive | e2e:Organizar/archivar sin credenciales | gap: untested — the toolbar Archive is never clicked; a step only reads its title | e2e:Organizar/archivar en bloque y deshacer | vitest:src/hooks/useGlobalShortcuts.test.tsx::e archives the selected conversations and clears the selection | gap: untested — card ⋮ → Archive and the board pane's Archive are never driven | e2e:Organizar/vista Archive |
| organize.select | vitest:src/components/Inbox/EmailRow.selection.test.tsx::the checkbox toggles the row without opening it | n/a: the open thread is one conversation; selection belongs to the list | e2e:Organizar/selección múltiple | vitest:src/hooks/useGlobalShortcuts.test.tsx::x selects the cursor row and Escape clears the selection | gap: missing — cards have no checkbox and the board mounts no bulk toolbar | gap: untested — checkboxes and the bulk bar are never driven in unified mode |
| organize.undo | e2e:Organizar/archivar sin credenciales | gap: untested — no test or step archives or deletes from the toolbar and then clicks Undo | e2e:Organizar/archivar en bloque y deshacer | e2e:Atajos/e archiva y abre la siguiente | gap: untested — undo and rollback are never driven from a card or the board pane | gap: untested — undo and rollback are never driven in unified mode |
| organize.advance | gap: missing — ⋮ Archive/Delete/Snooze on the open conversation's own row skip the leave planner: the pane closes instead of advancing | vitest:src/components/EmailView/EmailView.shortcuts.test.tsx::snoozing from the toolbar opens the next conversation | n/a: bulk actions never advance, by design (this sub-feature's own rule) | e2e:Atajos/e archiva y abre la siguiente | gap: missing — the board publishes no list, so leaving always closes the board's pane | gap: untested — auto-advance is never driven in unified mode |
| organize.snooze | e2e:Organizar/posponer desde el menú | vitest:src/components/EmailView/EmailView.shortcuts.test.tsx::snoozing from the toolbar opens the next conversation | vitest:src/components/Inbox/BulkToolbar.test.tsx::snoozes the selected inbox conversations and clears the selection | vitest:src/components/EmailView/EmailView.shortcuts.test.tsx::b opens the snooze picker | gap: untested — card ⋮ → Snooze and the board pane's Snooze are never driven | gap: untested — snooze is never driven in unified mode |
| organize.read | gap: untested — ⋮ → Mark as read/unread is never clicked | gap: untested — the toolbar read/unread button is never clicked; only the pane command is tested | vitest:src/components/Inbox/BulkToolbar.test.tsx::marks read and keeps the selection | vitest:src/hooks/useGlobalShortcuts.test.tsx::Shift+U on the cursor row marks it unread; s unstars a starred row | gap: untested — never driven from a card or the board pane | gap: untested — never driven in unified mode |
| organize.delete | gap: untested — ⋮ → Delete thread is never clicked | gap: untested — the toolbar Delete is never clicked | vitest:src/components/Inbox/BulkToolbar.test.tsx::deletes the selected conversations in one call and clears the selection | vitest:src/hooks/useGlobalShortcuts.test.tsx::# deletes the cursor row in the full-width list | gap: untested — never driven from a card or the board pane | gap: untested — never driven in unified mode |

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
