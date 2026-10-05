-- V033: one email signature per account.
--
-- `html` is the signature as the compose editor writes it, sanitized on save
-- with the same allowlist the send path uses (`sanitize_outgoing_html`). An
-- empty string means "no signature". The two flags pick where the composer
-- inserts it: a new message, and a reply or forward (Gmail/Outlook parity).
-- The row goes with its account.
CREATE TABLE IF NOT EXISTS account_signatures (
    account_id      TEXT    PRIMARY KEY NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    html            TEXT    NOT NULL DEFAULT '',
    use_for_new     INTEGER NOT NULL DEFAULT 1 CHECK (use_for_new IN (0, 1)),
    use_for_replies INTEGER NOT NULL DEFAULT 1 CHECK (use_for_replies IN (0, 1)),
    updated_at      INTEGER NOT NULL
) WITHOUT ROWID;
