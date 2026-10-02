-- V031: snooze — hide a conversation from the Inbox until a chosen time.
--
-- Local state only: no provider exposes a portable snooze (Gmail's is not in
-- its API, Graph and IMAP have none), so the record lives here and snooze
-- works the same on every account type. Keyed by conversation, because
-- `thread_id` is what survives an IMAP or Graph move (both re-key messages).
--
-- A row is in one of two states:
--   * snoozed (`woke_at IS NULL`): the thread is hidden from the Inbox list and
--     its count, and listed in the Snoozed view, until `snoozed_until`.
--   * woken   (`woke_at` set): the wake-up ticker brought it back. The Inbox
--     sorts the thread by `woke_at` instead of its latest message's date, so a
--     thread snoozed weeks ago comes back at the top (as Gmail does) without
--     rewriting the message's real timestamp.
-- A new inbound message in the thread deletes the row in either state (the
-- thread then sorts by that message), as do unsnooze, archive and delete.
CREATE TABLE IF NOT EXISTS thread_snoozes (
    account_id    TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    thread_id     TEXT    NOT NULL,
    snoozed_until INTEGER NOT NULL CHECK (snoozed_until > 0),
    created_at    INTEGER NOT NULL,
    woke_at       INTEGER CHECK (woke_at IS NULL OR woke_at > 0),
    PRIMARY KEY (account_id, thread_id)
) WITHOUT ROWID;

-- The wake-up ticker reads the due snoozes every 30 s; the woken rows are
-- not part of that set.
CREATE INDEX IF NOT EXISTS idx_thread_snoozes_due
    ON thread_snoozes(snoozed_until)
    WHERE woke_at IS NULL;
