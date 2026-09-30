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
- `search.exclusions` mail in the trash, and mail the junk detector or the user marked as junk, is never a result or a retrieval candidate (keyword or vector), so the chat cannot cite it.

## How to get to it (user POV)

- The box above the inbox list (placeholder `Search… (from:, to:, subject:)`), or the
  sidebar **Search emails… ⌘K** button, which focuses it.
- Operators are listed in the Search Tips popover under the box.

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
- The clear ✕ has no `aria-label` or `title` (accessibility gap worth fixing in `InboxSearchBox.tsx`); `aria/Clear search` does not exist, and a loose "clear" match hits the output panel's "Clear logs".
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
