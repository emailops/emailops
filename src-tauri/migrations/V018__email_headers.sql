-- Captured RFC 5322 headers, one row per email.
--
-- Junk detection — and phishing detection in particular — is overwhelmingly a
-- header story: SPF/DKIM/DMARC results, Reply-To divergence, List-Unsubscribe
-- presence, and the receiving server's own X-Spam-* verdict. None of it was
-- stored before this migration, so the detector had nothing authoritative to
-- reason about.
--
-- Gmail (format=full) and IMAP (RFC822) already fetch these and discard them;
-- only Microsoft Graph needed its $select widened. Backfill of already-synced
-- mail runs separately and is resumable — a message with no row here is treated
-- as "unknown", never as "clean".
--
-- Typed columns rather than a key/value table (which would need a join per
-- message) or a JSON blob (which would need parsing on every read).
-- `extra_json` absorbs allowlisted long-tail headers without another migration.

CREATE TABLE IF NOT EXISTS email_headers (
    email_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,

    -- The TOPMOST Authentication-Results only, plus its authserv-id.
    -- A verdict is trustworthy only when authserv_id matches the MTA we expect
    -- for the account: an attacker can paste a forged "spf=pass; dmarc=pass"
    -- line into their own message, and our MTA prepends the real one above it.
    auth_results TEXT,
    authserv_id TEXT,

    received_spf TEXT,
    -- Comma-joined d= values from every DKIM-Signature, in header order.
    dkim_domains TEXT,

    return_path TEXT,
    reply_to TEXT,
    -- Raw From including the display name — lookalike/impersonation checks need
    -- the unparsed form.
    from_raw TEXT,
    to_raw TEXT,

    -- Bulk-mail markers. Presence is evidence of graymail AND a suppressor on
    -- the spam/phishing axes: legitimate ESP mail is the largest single source
    -- of false positives.
    list_id TEXT,
    list_unsubscribe TEXT,
    list_unsubscribe_post TEXT,
    precedence TEXT,

    x_mailer TEXT,
    content_type TEXT,

    received_count INTEGER NOT NULL DEFAULT 0,
    -- The BOTTOM-most Received: the origin hop, which a sender cannot push down
    -- the list by prepending more.
    first_received TEXT,

    -- X-Spam-* / rspamd headers from the receiving server, joined. On IMAP this
    -- carries most of the achievable spam recall for free.
    spam_headers TEXT,

    extra_json TEXT,
    captured_at INTEGER NOT NULL,

    FOREIGN KEY (email_id) REFERENCES emails(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_email_headers_account ON email_headers(account_id);
