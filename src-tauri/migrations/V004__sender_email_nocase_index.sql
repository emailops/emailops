-- Case-insensitive sender-address index for `from:` search.
--
-- The `from:` filter lowercases the user's needle and does a B-tree range scan
-- on `sender_email`. The existing idx_emails_sender_email uses BINARY collation,
-- so a stored mixed-case address (e.g. "EMEA_Invoicing@email.apple.com") sorts
-- before the lowercased needle and never falls inside the range — the search
-- returned 0 results. This NOCASE index lets the range scan match regardless of
-- case while staying index-backed. The original BINARY index is left in place
-- for exact-match callers (trusted senders, contact self-match).
CREATE INDEX IF NOT EXISTS idx_emails_sender_email_nocase
    ON emails(account_id, sender_email COLLATE NOCASE);
