-- Smart-filter stats and filtered views scope to mailbox IN ('inbox','sent')
-- and count DISTINCT threads (so sidebar counts match the thread rows shown
-- when a suggestion is clicked). Extend the covering indexes with mailbox so
-- the GROUP BY stats queries and the matched_threads CTE stay index-only
-- instead of falling back to per-row table lookups.
DROP INDEX IF EXISTS idx_emails_domain_filter;
CREATE INDEX IF NOT EXISTS idx_emails_domain_filter
    ON emails(account_id, is_deleted, sender_domain, mailbox, thread_id);

DROP INDEX IF EXISTS idx_emails_sender_filter;
CREATE INDEX IF NOT EXISTS idx_emails_sender_filter
    ON emails(account_id, is_deleted, sender_email, mailbox, thread_id);
