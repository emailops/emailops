# Tag Board

A grid of blocks, one per (account, classified tag) for the chosen tag type, each
listing the threads whose newest tagged message carries that tag. The board shows the
15 best-ranked blocks (engagement score, computed by the backend), a range filter, a tag
search, hide/restore per block, two block widths and drag-to-reorder.

## Sub-features

- `tagboard.blocks` one block per (account, tag) with threads in the current slice, ranked, capped at 15.
- `tagboard.rows` each block lists its threads, eight per page, "Show more" until exhausted.
- `tagboard.range` All time / Today / Yesterday / Last 7 days / Custom limit the slice by the newest tagged message.
- `tagboard.search` "Search tags…" narrows blocks by tag substring and bypasses the ranking cap.
- `tagboard.junk` "Hide junk messages" also drops graymail from the counts.
- `tagboard.hide` "Block options → Hide this tag" removes a block; "Show N hidden tags" restores; persists.
- `tagboard.open` a card opens the thread in the middle pane.
- `tagboard.density` Narrow / Wide blocks change the columns per row.
- `tagboard.reorder` drag a block header to reorder; the order is remembered.
- `tagboard.groupBy` Company / Intent / Topic / Priority picks the tag type the board groups by; the choice is remembered.
- `tagboard.categories` Gmail/Outlook category chips narrow the board; IMAP-only scopes hide the row.
- `tagboard.inbox` a block title or "Open in inbox" applies the tag as the inbox filter and switches to the list.
- `tagboard.pin` Pin keeps a tag at the top of the sidebar list; persists.
- `tagboard.cardActions` every card carries the inbox row's ⋮ actions (filter sender, hide from smart filters, block sender, read/unread, star, snooze, archive, attachment rule, classification rule, open in tab, chat about thread).
- `tagboard.scope` the selected account scopes the board; All accounts shows one block per account.

## How to get to it (user POV)

Two entry points to the same ranked tags, the columns of `## Parity`:

- **Tag Board view** — Sidebar → Views → **Tag Board** (only shown when AI is enabled): blocks per (account, tag) with a toolbar for group by, search, density, range, categories and hide junk.
- **Sidebar tag filters** — the Smart Filters section of the sidebar (Companies / priority / intent / topic), at most 10 per type; a click filters the inbox list, hover gives Pin and ✕ Hide.

## Parity

| Capability | Tag Board view | Sidebar tag filters |
|---|---|---|
| tagboard.blocks | e2e:Tag Board/bloques | gap: untested — sidebar tag groups come from the same stats, 10 per type; only the store helper is tested, no SmartFilters/Sidebar test |
| tagboard.rows | gap: untested — Show more paging is proven only by the oracle and the lib helper; Tag Board/bloques only counts rows | gap: untested — a tag click lists its threads; no test clicks a sidebar tag |
| tagboard.range | e2e:Tag Board/rango Today | gap: missing — the tag-filtered inbox has no date window and the sidebar counts use no window |
| tagboard.search | e2e:Tag Board/buscar tag | gap: missing — no tag search in the sidebar and each type is cut to 10, so a capped tag is unreachable there |
| tagboard.junk | gap: untested — the Hide junk checkbox is proven only by the oracle and a backend stats test | gap: missing — hide-junk is bypassed while a smart filter is active and the sidebar counts ignore it |
| tagboard.hide | gap: untested — Hide this tag / Show N hidden tags have no component test or sweep step | gap: missing — the sidebar ✕ hides a tag but nothing restores it: restoreFilter is never wired |
| tagboard.open | e2e:Tag Board/abrir hilo desde un bloque | gap: untested — opening a thread from the tag-filtered list; Inbox/abrir hilo runs with no filter |
| tagboard.density | gap: untested — Narrow/Wide is checked only by the tagboard oracle | n/a: the sidebar is a single list, there are no blocks to widen |
| tagboard.reorder | gap: untested — drag-to-reorder is not automated anywhere | gap: missing — the sidebar order is the ranking with pinned first; the board's saved order is not applied and there is no drag |
| tagboard.groupBy | gap: untested — the group-by switch is never switched; the toolbar step only checks layout | n/a: the sidebar lists all four tag types at once, each under its own heading |
| tagboard.categories | gap: untested — category chips only appear for Gmail/Outlook scopes and no test selects one | gap: untested — inbox category tabs narrow the tag-filtered list; no test |
| tagboard.inbox | gap: untested — block title / ⋮ Open in inbox has no test | gap: untested — this is the sidebar tag click itself; no Sidebar/SmartFilters test |
| tagboard.pin | gap: missing — the block menu offers only Hide this tag and Open in inbox; pinned tags get no place on the board | gap: untested — Pin/Unpin on hover; only the store test puts pinned filters first |
| tagboard.cardActions | gap: untested — cards carry EmailActionsMenu; TagEmailCard.testids only checks data attributes | gap: untested — filtered inbox rows carry the same ⋮ menu; Inbox/menú ⋮ de una fila runs with no tag filter |
| tagboard.scope | gap: untested — per-account blocks and All accounts are proven only by the oracle and a backend stats test | gap: untested — under All accounts the sidebar merges a value across accounts and sums its counts; only store tests |

## Driving it with verify.sh

The automated check is `scripts/tagboard_check.mjs` (`node …/tagboard_check.mjs "$(readlink src-tauri/reports/verify/current)"`); it needs a `launch`ed instance. It reads the board through the `data-testid="tag-column"` / `data-testid="tag-card"` hooks (`data-account-id`, `data-tag-value`, `data-thread-count`, `data-loaded`, `data-has-more`, `data-thread-id`) and never parses visible text.

Each case says which kind of test proves it. The three kinds:

| Kind | What it compares | Where it lives |
|---|---|---|
| **backend ↔ BD** | the Tauri command the view calls (`get_tag_board_stats`, invoked from the page) against an oracle SQL that re-derives the rule from the DB, with no cap | `tagboard_check.mjs`, `oracleThreads()` |
| **UI ↔ backend** | what is rendered against the ranked list the backend returned, after hidden blocks and the 15-block cap | `tagboard_check.mjs`, `compareUiToBackend()` |
| **UI ↔ BD** | rendered thread ids per block against the oracle's thread set (no thread missing, none extra, no duplicates) | `tagboard_check.mjs`, `uiThreads()` |

Plus the layers that do not need the app: the ranking score, recency decay and the 15-cap are unit-tested in Rust (`services/filters.rs`, `db/tags.rs`) and the reducer/ordering in `src/lib/tagBoard.test.ts`.

| Case | Kind | Proves |
|---|---|---|
| bloques completos · Company/Intent/Topic/Priority | backend ↔ BD, UI ↔ backend | every (account, tag) with a thread in the DB is a candidate with the right count; the UI shows exactly the ranked top 15 minus hidden |
| hilos completos · … | UI ↔ BD | after expanding every block, each lists exactly the DB's threads for that tag |
| rango Today / Yesterday / Last 7 days | backend ↔ BD, UI ↔ backend | the window is `[local midnight, …)` on the newest tagged message |
| rango Custom | same | typed dates become `[from 00:00, to + 1 day)` |
| buscar tag | same | substring, case-insensitive, no cap |
| ocultar junk | same | graymail excluded when the box is ticked (needs junk verdicts in the DB to discriminate) |
| abrir hilo | DOM ↔ BD | the H1 is the subject of the thread's newest message |
| ocultar bloque y persistencia | DOM ↔ BD | hidden block gone, the next-ranked one enters, survives reload, restore returns it |
| cambio de cuenta | backend ↔ BD, UI ↔ backend | per-account scope and All accounts (one block per account) |
| anchura de bloque | DOM | Wide gives fewer columns per row than Narrow (needs the chat panel closed) |
| reordenar (drag) | not automated | pointer gesture; check by hand or with cua-driver `drag` |

## Gotchas

- Block titles are `<account> · <tag>`; the hooks carry the raw values, use them.
- The board caps at 15 *ranked* blocks, not the 15 with most threads: a tag with many unread, unanswered notifications ranks below a small tag you reply to. The search box is how a capped tag is reached.
- Custom dates must be typed as the local calendar day; `toISOString()` shifts a local midnight to the previous UTC day.
- The ⋮ block menu only shows on hover; click it through the DOM, not through `waitForClickable`.
- With the chat panel docked the board is a single column; density has nothing to change.
- Priority tags do not exist in the demo DB, so those cases report n/a.
