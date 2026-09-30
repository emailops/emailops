-- V029: read-state changes that still have to reach the provider.
--
-- Marking a message read is local-first: the row flips immediately and the
-- push to the account (Gmail label, IMAP \Seen, Graph isRead) is best-effort.
-- A push that failed — offline, a 5xx, expired credentials — used to be lost:
-- the message stayed unread in every other client, forever.
--
-- `read_push_pending_since` is NULL for a row whose read state the provider
-- already has, and the unix time the change was made for one it still owes.
-- The sync retries those, and its server-to-local refresh leaves them alone
-- (a pending local change wins over the server's older state).
ALTER TABLE emails ADD COLUMN read_push_pending_since INTEGER;

-- Partial index: the pending set is normally empty, so the per-sync retry
-- lookup costs nothing on a large mailbox.
CREATE INDEX IF NOT EXISTS idx_emails_read_push_pending
    ON emails(account_id, read_push_pending_since)
    WHERE read_push_pending_since IS NOT NULL;
