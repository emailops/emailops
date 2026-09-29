-- Candidate attachment rules mined from recurring document attachments
-- (`services::attachment_suggestions`). Recomputed after each sync: pending
-- rows are replaced, while accepted / dismissed rows are kept so the same
-- candidate (same `suggestion_key`) is never proposed again.
CREATE TABLE IF NOT EXISTS attachment_rule_suggestions (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    suggestion_key TEXT NOT NULL,
    name TEXT NOT NULL,
    sender_email_pattern TEXT NOT NULL,
    filename_pattern TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]',
    email_count INTEGER NOT NULL,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    sample_filenames_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'accepted', 'dismissed')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE,
    UNIQUE (account_id, suggestion_key)
);
CREATE INDEX IF NOT EXISTS idx_attachment_rule_suggestions_status
    ON attachment_rule_suggestions(account_id, status);

-- Mining drives from the attachment metadata of one account; without this
-- index it would scan every account's attachments.
CREATE INDEX IF NOT EXISTS idx_email_attachment_meta_account
    ON email_attachment_meta(account_id, email_id);
