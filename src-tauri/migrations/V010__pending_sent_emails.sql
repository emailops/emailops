-- V010: locally inserted sent copies awaiting provider reconciliation.
-- pending_sync = 1 marks a row inserted optimistically at send time with a
-- synthetic id (Outlook / IMAP — the provider reported no canonical message
-- id). The sync reconciler deletes it when the provider's real Sent copy is
-- ingested. Gmail rows use the real provider id and are inserted with
-- pending_sync = 0 (nothing to reconcile).
ALTER TABLE emails ADD COLUMN pending_sync INTEGER NOT NULL DEFAULT 0;

-- Partial index: the pending set is tiny (usually 0-2 rows), so this keeps
-- the reconciler's per-batch lookup cheap without bloating the main table.
CREATE INDEX IF NOT EXISTS idx_emails_pending_sync
    ON emails(account_id, pending_sync) WHERE pending_sync = 1;
