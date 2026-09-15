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

## How to get to it (user POV)

- Sidebar → Views → **Tag Board** (only shown when AI is enabled).
- The selected account (or All accounts) scopes the board; categories behave as in the inbox.

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
