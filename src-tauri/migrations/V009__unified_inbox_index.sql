-- Global timestamp ordering for the unified (all-accounts) inbox/sent/spam views.
-- The per-account index idx_emails_account_mailbox cannot serve a multi-account
-- predicate in sorted order (SQLite would sort the whole merged set). This index
-- lets the unified list scan in ORDER BY timestamp DESC, id DESC order and
-- early-terminate at LIMIT, filtering account enablement per row.
CREATE INDEX IF NOT EXISTS idx_emails_mailbox_active
    ON emails(mailbox, is_deleted, timestamp DESC, id DESC);
