# Search emails

Typing in the search box above the list and pressing Enter filters the inbox to matching
threads across subject, sender and body, with `from:`, `to:`, `subject:`, `is:unread`,
`after:`, `before:`, `tag:` and `id:` (exact email ids, repeatable; used by the chat "Show in email list" button) operators. The ✕ inside the box clears the search and
restores the full list.

## Sub-features

- `search.results` matching rows replace the list; the query stays in the box.
- `search.empty` a query with no hits shows "No emails match your search" and no rows.
- `search.clear` the ✕ inside the box empties it and restores the unfiltered inbox.
- `search.autocomplete` typing `from:` offers sender suggestions (ArrowDown/Enter to pick).
- `search.exclusions` mail in the trash is never a search result; trashed mail and mail the junk detector or the user marked as junk is never a retrieval candidate (keyword or vector), so the chat cannot cite it. The app's own search keeps junk-marked mail reachable on purpose.
- `search.operators` `from:`, `to:`, `subject:`, `is:unread`, `after:`, `before:`, `tag:` and `id:` narrow the results the same way in every entry.
- `search.tips` the operators are listed next to the box (Search Tips).
- `search.accountChip` in an All-accounts search each hit names its account with a chip.

## How to get to it (user POV)

Four entry points, the columns of `## Parity`:

- **Inbox search box (one account)** — the box in the inbox header (placeholder `Search… (from:, to:, subject:)`); Enter filters the list, ✕ clears. Rendered only in the full-width layout, which the demo uses.
- **Inbox search box (All accounts)** — the same box while **All accounts** is selected: results span every enabled account.
- **Search overlay (⌘K)** — the sidebar **Search emails… ⌘K** button or ⌘K opens a separate modal (`SearchBar`) with hits as you type and the Search Tips; Enter applies the query to the list. In the split layout (the app default) this is the only search entry.
- **CLI search** — `emailops-cli search "<query>" [--limit --offset --trace --json]`, one account.

## Parity

| Capability | Inbox search box (one account) | Inbox search box (All accounts) | Search overlay (⌘K) | CLI search |
|---|---|---|---|---|
| search.results | e2e:Búsqueda/consulta Ollama | gap: untested — no step searches with All accounts selected; the fan-out is only proven in the service | gap: untested — Enter applying the overlay query to the list has no test; SearchBar.accountChip only renders the dropdown | gap: untested — only clap parsing is tested; no dispatch test runs a query |
| search.empty | e2e:Búsqueda/sin resultados | gap: untested — the empty state is not driven in unified mode | gap: untested — the overlay's own empty text has no test | gap: untested — an empty result is never asserted |
| search.clear | e2e:Búsqueda/limpiar | vitest:src/components/Inbox/InboxSearchBox.clear.test.tsx::labels the ✕ with the clear-search string and clears on click | gap: missing — the overlay ✕ only closes the overlay; in the split layout an applied search shows no query and has no clear control | n/a: a one-shot command keeps no query to clear |
| search.autocomplete | gap: untested — the from:/to: suggestions in InboxSearchBox have no test or step | gap: untested — not driven in unified mode, where suggestions come from the first enabled account only | gap: untested — the overlay's suggestions have no test | n/a: the query is a shell argument; there is no input to suggest into |
| search.exclusions | integration:retrieval_leaves_out_trashed_and_junk_marked_messages | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| search.operators | rust:src-tauri/src/services/search.rs::search_by_ids_still_applies_other_operators | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| search.tips | gap: missing — the inline box has no tips; its placeholder names only from:, to:, subject: | gap: missing — same box, no tips | gap: untested — the Search Tips grid has no test | gap: missing — `search --help` says only "Search query." |
| search.accountChip | n/a: one account's results need no chip | gap: untested — EmailRow.accountChip proves the row draws a chip when asked; nothing proves Inbox asks for it during a unified search | vitest:src/components/Search/SearchBar.accountChip.test.tsx::names each hit’s account in the unified view | n/a: CLI search is scoped to one account |

## Driving it with verify.sh

Preconditions: baseline; inbox of `demo-acct-work` visible. `R=$(readlink src-tauri/reports/verify/current)`. Box selector: `input[placeholder^="Search…"]`; its clear button: `form:has(input[placeholder^="Search…"]) button`.

- Query → `$V wd shot "$R/search-before.png"`, `$V wd type 'input[placeholder^="Search…"]' 'Ollama'`, `$V wd keys Enter`, `$V wd shot "$R/search-results.png"` → `$V wd find '//div[@role="button"][contains(., "Ollama")]'` prints one row (Kwame Boateng, *Can EmailOps use my own Ollama models?*) and `$V wd exists '//div[@role="button"][contains(., "Nadia Brunner")]'` prints `absent`.
- Same result headless (cross-check) → `make cli-demo ARGS="search 'Ollama' --json" | sed -n '/^{/,$p'` returns one hit with that subject.
- Empty → `$V wd type 'input[placeholder^="Search…"]' 'zzzz-no-such-term'`, `$V wd keys Enter`, `$V wd shot "$R/search-empty.png"` → `$V wd js 'document.querySelectorAll("div[role=button]").length'` prints `0` and the page text contains *No emails match your search*.
- Clear → `$V wd click 'form:has(input[placeholder^="Search…"]) button'`, `$V wd shot "$R/search-clear-after.png"` → the Nadia Brunner row is `present` again and the box value is empty.
- Trashed mail stays out → `$V wd type 'input[placeholder^="Search…"]' 'Larkspur'`, `$V wd keys Enter` → exactly one row, *Corrected Larkspur Freight renewal quote*; the trashed *Larkspur Freight renewal quote* (same sender, `is_deleted = 1`, still in `emails_fts`) is not listed. Confirmed live on 30/09/2026.

Live run 11/09/2026: the first four steps proven; evidence in `src-tauri/reports/verify/20260911-145214/` (`search-before/results/empty/clear-after.png`).

## Gotchas

- Enter must go through `$V wd keys Enter`: the embedded server dispatches synthetic key events, which do not submit a `<form>` on their own, so the helper submits the active field's form the way a real Enter does.
- The clear ✕ carries `aria-label`/`title` from `inbox:header.clearSearch`; a loose "clear" text match still hits the output panel's "Clear logs", so address it by that label.
- The `inbox:header.search` ("Searching: …") string is not rendered in this layout; the query in the box is the visible state.
- Search is FTS over the local DB; `Ollama`, `Proton Bridge`, `Fly.io` match the 79 demo emails, `refresh tokens` does not. Results are scoped to the selected account.
- The verification fixtures (`insert_verification_fixtures` in `scripts/generate_demo_db.py`) add two pairs to the work account: Larkspur Freight (a trashed quote and its correction) and Tessellate Hosting (the real renewal notice and a lookalike marked as junk). Add them to an existing demo DB with `uv run scripts/generate_demo_db.py --append --demo-db .emailops-demo-data/emailops.db`, then `make cli-demo ARGS="embed --account demo-acct-work --json"`, then `touch .emailops-demo-data/emailops.db` (a DB older than the generator is rebuilt from scratch on the next launch).

| Case | Test kind |
|---|---|
| query parsing, operators, FTS and vector queries, chunking, exclusion of trashed and junk rows | unit (`db::emails::search`, `db::embeddings`, `services::search`, `services::embeddings`, `services::retrieval`) |
| an Embeddings run stopped mid-way and resumed | unit on a real queue (`services::embeddings::tests::a_cancelled_run_stops_at_the_next_email_and_a_later_run_embeds_the_rest`) |
| keyword + vector candidates leave out trashed and junk-marked mail, and take back an un-junked one | integration (`retrieval_leaves_out_trashed_and_junk_marked_messages`) |
| search and Embeddings command arguments | contract (`src/lib/apiContract/busqueda.api.test.ts`) |
| search box, account chip | vitest (`InboxSearchBox`, `searchQuery`, `Search/*`) |
| query, empty, clear, trashed mail left out | e2e (`Búsqueda/*`) |
| the chat does not cite a trashed email or a junk-marked lookalike | eval (`retrieval_*` in `src-tauri/evals/chat/cases/retrieval_exclusions.yaml`) |
| agent search quality | eval on the production mailbox (`agent_search`), outside `make verify` |
