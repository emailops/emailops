-- V030: starred (Gmail STARRED, Outlook flag, IMAP \Flagged) and its write-back.
--
-- `is_starred` is per message, like the providers keep it. A thread counts as
-- starred when any of its messages is.
--
-- Starring is local-first, like read state (V029): the row flips immediately
-- and `star_push_pending_since` records, in the same statement, that the
-- provider still has to be told. The sync retries what is pending, and its
-- server-to-local refresh leaves a pending row's star alone.
ALTER TABLE emails ADD COLUMN is_starred INTEGER NOT NULL DEFAULT 0 CHECK (is_starred IN (0, 1));
ALTER TABLE emails ADD COLUMN star_push_pending_since INTEGER;

-- The Starred view lists starred mail per account; the set is small, so a
-- partial index keeps it cheap on a large mailbox.
CREATE INDEX IF NOT EXISTS idx_emails_starred
    ON emails(account_id, timestamp)
    WHERE is_starred = 1;

-- Normally empty: the per-sync retry lookup costs nothing.
CREATE INDEX IF NOT EXISTS idx_emails_star_push_pending
    ON emails(account_id, star_push_pending_since)
    WHERE star_push_pending_since IS NOT NULL;
