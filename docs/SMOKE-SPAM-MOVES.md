# Smoke test — Spam moves made in the provider's own client

Manual check that EmailOps follows a message the user moves **into** or **out of** Spam in
Gmail, Outlook or an IMAP webmail. About 10 minutes per account. Automated coverage stops at
the fakes: the IMAP and Graph network paths are only exercised here.

## Before you start

- Run against the dev data dir (`.emailops-data/`): `make dev`, or drive the same install
  headless with `make cli-fast ARGS="…"`. Close the app before any `sync`.
- Get the account id: `make cli-fast ARGS="accounts --json"`.
- The check runs **at most once every 7 minutes per account**. Force it between attempts:

  ```bash
  sqlite3 .emailops-data/emailops.db \
    "DELETE FROM user_preferences WHERE key LIKE 'spam_reconcile_%';"
  ```

  That also clears the "the provider no longer has this message" memory, which is what you
  want when repeating a step.

## 1. Out of Spam ("Not spam")

1. `make cli-fast ARGS="emails --mailbox spam --limit 5 --json"` — note an `id` and its subject.
2. In the provider's own client: **Not spam** (Gmail), **Not junk** (Outlook), or move the
   message to INBOX (IMAP webmail).
3. Clear the stamp (above), then `make cli-fast ARGS="sync <account-id> --json"`.
4. Expect:
   - log line `Moved 1 email(s) out of Spam to match the provider`;
   - the message gone from `emails --mailbox spam`;
   - the message listed by `emails --mailbox inbox`.

   On Gmail the id is unchanged. On IMAP and Outlook it is a **new** id — `make cli-fast
   ARGS="show <new-id> --json"` and confirm the body and subject came along, which is what
   proves the row was re-keyed instead of re-downloaded.

## 2. Into Spam (marking spam at the provider)

1. Pick an inbox message: `make cli-fast ARGS="emails --mailbox inbox --limit 5 --json"`.
2. Mark it as spam / junk in the provider's own client.
3. `make cli-fast ARGS="sync <account-id> --json"` (no stamp reset needed — this path rides
   along with the normal Spam pass).
4. Expect the message listed by `emails --mailbox spam` and gone from `emails --mailbox inbox`.
   On Gmail the log says `Moved 1 email(s) into Spam to match the provider`; on IMAP and
   Outlook it arrives as a new row and the inbox copy is hidden instead.

## 3. Other destinations (optional)

- Spam → Trash at the provider: the message should end up under `emails --mailbox trash`.
- **IMAP only:** Spam → one of your own folders: it should show under that folder
  (`folder:<path>`), not in Spam.
- **Outlook:** Spam → Archive lands in the inbox — EmailOps has no archive mailbox, same as
  Gmail's archived mail.

## Expected non-events

These are working as designed, not failures:

- Spam older than 30 days is never reconciled.
- An account with 2000+ messages in Spam even over the last 2 days logs a warning and skips
  the check entirely.
- A message deleted forever at the provider keeps its local row, and is asked about only once.
- A local row with no `Message-ID` header is left alone on IMAP and Outlook.
