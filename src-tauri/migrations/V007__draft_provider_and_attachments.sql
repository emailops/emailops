-- Provider draft integration + draft attachments.
--
-- `provider_draft_id` links a local draft to its counterpart in the provider's
-- Drafts folder (Gmail/Graph). NULL means local-only (never pushed, or a
-- provider that does not support drafts such as IMAP).
--
-- `cc_addresses_json` and `body_html` let a draft round-trip faithfully when it
-- is pulled back from the provider or composed in the rich editor.
ALTER TABLE drafts ADD COLUMN provider_draft_id TEXT;
ALTER TABLE drafts ADD COLUMN cc_addresses_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE drafts ADD COLUMN body_html TEXT;

-- Attachments are stored as file-path references; the bytes are read lazily at
-- provider-push / send time. Cascade so deleting a draft drops its attachments.
CREATE TABLE IF NOT EXISTS draft_attachments (
    id TEXT PRIMARY KEY,
    draft_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    FOREIGN KEY (draft_id) REFERENCES drafts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_draft_attachments_draft ON draft_attachments(draft_id);

-- Index provider-linked drafts for the pull/reconcile pass.
CREATE INDEX IF NOT EXISTS idx_drafts_provider ON drafts(account_id, provider_draft_id);
