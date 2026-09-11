# Search emails

Typing in the search box above the list filters the inbox to matching threads across
subject, sender and body, with `from:`, `to:`, `subject:`, `is:unread`, `after:`, `before:`
and `tag:` operators. Clearing the search restores the full list.

## Sub-features

- `search.open` the sidebar "Search emails… ⌘K" button focuses the search box.
- `search.results` matching rows replace the list; the header shows `Searching: "<query>"`.
- `search.empty` a query with no hits shows "No emails to show".
- `search.clear` the header's Clear search returns to the unfiltered inbox.

## How to get to it (user POV)

- Sidebar → **Search emails…**, or click the box above the inbox list (placeholder
  `Search… (from:, to:, subject:)`).
- Operators are listed in the Search Tips popover under the box.

## Driving it with verify.sh

Preconditions: baseline; inbox of `demo-acct-work` visible. Selector for the box: `input[placeholder^="Search…"]` (confirmed present on 11/09/2026; the sidebar button is `button*=Search emails`).

- Query → `$V wd type 'input[placeholder^="Search…"]' 'Ollama'`, `$V wd keys Enter`, `$V wd shot "$R/search-results.png"` → `$V wd find '//div[@role="button"][contains(., "Ollama")]'` prints the single expected row (Kwame Boateng, *Can EmailOps use my own Ollama models?*) and `$V wd exists '//div[@role="button"][contains(., "Nadia Brunner")]'` prints `absent`.
- Same result headless (cross-check) → `make cli-demo ARGS="search 'Ollama' --json"` returns one hit with that subject.
- Empty → type `zzzz-no-such-term`, Enter → `$V wd find '*=No emails to show'` prints a match.
- Clear → `$V wd click 'aria/Clear search'` (or clear the box and press Enter) → the Nadia Brunner row is back.

## Gotchas

- Search is FTS over the local DB; a term must exist in the 79 demo emails. `Ollama`, `Proton Bridge`, `Fly.io` are known to match; `refresh tokens` does not.
- Results are scoped to the selected account; on "All accounts" the count can differ.
- `wd type` clears the field before typing; use `wd keys` for Enter.
