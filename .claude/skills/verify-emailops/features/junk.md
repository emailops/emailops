# Junk mail

A local detector scores each message on three axes (spam, phishing, graymail). A flagged
message shows a chip in the list, is left out of tag views, counts, search candidates and chat
retrieval, and can be corrected by the user ("junk" / "not junk"); the correction outranks any
later score.

## Sub-features

- `junk.chip` a flagged message shows its kind in the row and in the thread.
- `junk.feedback` marking as junk writes the chip at once and hides the message from filtered views; "not junk" removes both and is permanent.
- `junk.exclusion` flagged mail is left out of tag counts, the Tag Board, keyword and vector retrieval.
- `junk.settings` Settings → **Junk**: detector on/off, phishing axis, what to do with flagged mail.
- `junk.dim` a flagged row is faded in the list (not while selected) so the eye skips it.
- `junk.hideToggle` "Hide junk messages" above the inbox list and in the Tag Board toolbar hides flagged rows; it is the same preference as Settings → Junk.
- `junk.backfill` Settings → Junk shows per-account counts and scores existing mail per account with its own button.

## How to get to it (user POV)

Six entry points, the columns of `## Parity`:

- **Inbox list (one account)** — a flagged row shows its junk chip; **Hide junk messages** sits above the list when flagged mail is loaded.
- **All accounts** — the same list in unified mode.
- **Search overlay (⌘K)** — hits include flagged mail, because the app's search keeps it reachable.
- **Open thread** — a flagged message shows a banner with its kind, top reasons, **Is junk** and **Not junk**. This banner is the only junk / not-junk control; the row ⋮ menu has none.
- **Tag Board** — spam/phishing never reaches the board; flagged graymail stays unless the toolbar's **Hide junk messages** is checked.
- **Settings → Junk** — gear → **Junk**: detector, phishing axis, dim/hide, per-account counts and backfill.

## Parity

| Capability | Inbox list (one account) | All accounts | Search overlay (⌘K) | Open thread | Tag Board | Settings → Junk |
|---|---|---|---|---|---|---|
| junk.chip | e2e:Junk/chip en la bandeja | gap: untested — same row and chips, not driven with All accounts | gap: missing — overlay hits show no junk marking although the search keeps flagged mail | gap: untested — JunkBanner names the kind; no component test, and no step opens the flagged message | gap: missing — flagged graymail stays on the board unless Hide junk is on, and cards show no junk chip | n/a: settings show no message |
| junk.feedback | gap: missing — the row ⋮ menu has no junk / not-junk item | gap: missing — same menu in unified mode | n/a: overlay hits have no per-row actions; picking one opens the thread, where the banner applies | gap: untested — Is junk / Not junk have no component test or step; the banner renders only for already-flagged mail, so missed junk cannot be marked anywhere | gap: missing — the card ⋮ is the same menu with no junk item | n/a: settings act on the detector, not on a message |
| junk.exclusion | integration:junk_feedback_shows_the_chip_and_hides_the_message_from_company_views | n/a: backend, same path for every entry point | n/a: the app's own search keeps flagged mail reachable by design | n/a: an opened message is shown whatever its verdict | gap: untested — only the shared SQL helper and the tagboard oracle; no test runs the board query with a junk-marked row | n/a: settings list no mail |
| junk.settings | n/a: the detector switch and phishing axis live only in Settings → Junk; the list's own control is junk.hideToggle | n/a: same as one account | n/a: the overlay has no junk controls by design | n/a: the banner corrects one message; detector settings live in Settings | n/a: the board's own control is junk.hideToggle | vitest:src/components/Settings/JunkSettings.test.tsx::publishes the chosen action to the store so the inbox stops showing junk |
| junk.dim | gap: missing — only compact (full-width) rows fade; the split-layout row, the app default, never applies the opacity | gap: missing — same split/compact asymmetry in unified mode | gap: missing — overlay hits carry no junk marking at all | n/a: fading is dropped for the open message by design | gap: missing — cards never fade, so flagged graymail looks like clean mail | n/a: settings choose dim vs hide but show no rows |
| junk.hideToggle | gap: untested — the Hide junk messages checkbox has no component test or step | gap: untested — not driven in unified mode | n/a: a search deliberately bypasses the hide filter | n/a: an opened message is shown whatever the list hides | gap: untested — the toolbar checkbox has no test or step | vitest:src/components/Settings/JunkSettings.test.tsx::shows the action the store already holds, not a hardcoded default |
| junk.backfill | n/a: backfill is an account-level action offered only in Settings → Junk | n/a: same as one account | n/a: backfill is an account-level action offered only in Settings → Junk | n/a: backfill is an account-level action offered only in Settings → Junk | n/a: backfill is an account-level action offered only in Settings → Junk | vitest:src/components/Settings/JunkSettings.test.tsx::gives each account its own backfill button |

## Driving it with verify.sh

Preconditions: baseline. The demo DB holds one message the owner marked as junk: *Tessellate Hosting: payment failed, plan suspended* (`insert_verification_fixtures`). Confirmed live on 30/09/2026.

- Chip → `$V wd find '//div[@role="button"][contains(., "Tessellate Hosting Billing")]'` prints the row with its `Phishing` chip; the genuine *Your Tessellate Hosting plan renews in March* row has none.
- Excluded from tag views → the Tag Board oracle (`tagboard_check.mjs`) counts threads with the same rule as `db::exclude_junk_sql`, the user's own mark included.

## Gotchas

- The inbox list still shows flagged mail (with its chip) unless the "hide" action is chosen in Settings → Junk; exclusion applies to tag views, counts and retrieval.
- `Vistas/Spam` in the sweep is the provider's Spam folder (Inbox feature), not the detector.

| Case | Test kind |
|---|---|
| signals, scoring, verdicts, lookalike domains, training | unit (`services::junk::*`, `db::emails::junk*`) |
| exclusion rule | unit (`db::exclude_junk_sql_tests`) |
| feedback → chip → filtered list and count, and back | integration (`junk_feedback_shows_the_chip_and_hides_the_message_from_company_views`) |
| feedback → retrieval candidates | integration (`retrieval_leaves_out_trashed_and_junk_marked_messages`, under Search) |
| junk command arguments | contract (`src/lib/apiContract/junk.api.test.ts`) |
| chip in the list | e2e (`Junk/chip en la bandeja`) |
| detector quality | eval (`make eval-junk`, 47 synthetic cases + false-positive gates) |
| chat never cites a junk-marked lookalike | eval (`retrieval_excludes_junk_lookalike`, under Search) |
