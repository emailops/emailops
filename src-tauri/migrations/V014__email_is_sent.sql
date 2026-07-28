-- Record that the provider filed a message under Sent, independently of the
-- single-valued `mailbox` column.
--
-- Gmail labels mail you send to yourself with both INBOX and SENT. `mailbox`
-- can only hold one value and records 'inbox' so the thread stays in the inbox
-- view, which left such messages invisible in the Sent view — especially ones
-- sent through a send-as alias, whose sender is not the account address and so
-- did not match the sender-based fallback either.
ALTER TABLE emails ADD COLUMN is_sent INTEGER NOT NULL DEFAULT 0;

-- Existing rows the sync already filed under Sent are sent mail by definition.
UPDATE emails SET is_sent = 1 WHERE mailbox = 'sent';

-- Serves the Sent view's per-account, newest-first listing. Partial so it only
-- indexes the small sent subset rather than the whole mailbox.
CREATE INDEX IF NOT EXISTS idx_emails_account_is_sent
    ON emails (account_id, timestamp DESC)
    WHERE is_sent = 1 AND is_deleted = 0;
