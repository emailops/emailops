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

## How to get to it (user POV)

- Inbox row of a flagged message; row ⋮ menu → mark as junk / not junk.
- Gear → **Junk**.

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
