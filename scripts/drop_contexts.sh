#!/usr/bin/env bash
# drop_contexts.sh — one-off cleanup script to remove the dead `contexts` feature
# from an existing EmailOps production database.
#
# Run ONCE against the prod DB, then discard. The code no longer references
# contexts, so re-running is safe but unnecessary.
#
# Usage:
#   bash scripts/drop_contexts.sh [DB_PATH]
#
# DB_PATH defaults to the macOS production location:
#   ~/Library/Application Support/com.emailops.app/emailops.db
#
# The script creates a timestamped backup before making any changes.

set -euo pipefail

DB="${1:-$HOME/Library/Application Support/com.emailops.app/emailops.db}"

if [ ! -f "$DB" ]; then
  echo "Error: database not found at: $DB"
  exit 1
fi

BACKUP="${DB%.db}_before_drop_contexts_$(date +%Y%m%d_%H%M%S).db"
echo "Backing up to: $BACKUP"
cp "$DB" "$BACKUP"
echo "Backup created."

echo "Applying schema changes..."

sqlite3 "$DB" <<'SQL'
PRAGMA foreign_keys = OFF;
BEGIN;

-- ── Rebuild emails table without context_id ───────────────────────────────
CREATE TABLE emails_v2 (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    message_id TEXT,
    subject TEXT NOT NULL,
    sender TEXT NOT NULL,
    sender_email TEXT NOT NULL,
    sender_domain TEXT NOT NULL DEFAULT '',
    recipients_json TEXT NOT NULL,
    cc_json TEXT NOT NULL DEFAULT '[]',
    snippet TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    is_read INTEGER NOT NULL DEFAULT 0,
    is_deleted INTEGER NOT NULL DEFAULT 0,
    triage_status TEXT,
    category TEXT NOT NULL DEFAULT 'primary',
    mailbox TEXT NOT NULL DEFAULT 'inbox',
    raw_json TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);

INSERT INTO emails_v2
    SELECT id, account_id, thread_id, message_id, subject, sender, sender_email,
           sender_domain, recipients_json, cc_json, snippet, timestamp,
           is_read, is_deleted, triage_status, category, mailbox, raw_json, created_at
    FROM emails;

DROP TABLE emails;
ALTER TABLE emails_v2 RENAME TO emails;

-- ── Recreate all emails indexes ───────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_emails_account_id ON emails(account_id);
CREATE INDEX IF NOT EXISTS idx_emails_thread_id ON emails(thread_id);
CREATE INDEX IF NOT EXISTS idx_emails_timestamp ON emails(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_emails_triage_status ON emails(triage_status);
CREATE INDEX IF NOT EXISTS idx_emails_category ON emails(category);
CREATE INDEX IF NOT EXISTS idx_emails_sender_email ON emails(account_id, sender_email);
CREATE INDEX IF NOT EXISTS idx_emails_account_timestamp ON emails(account_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_emails_account_read ON emails(account_id, is_read, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_emails_account_active ON emails(account_id, is_deleted, timestamp DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_emails_account_mailbox
    ON emails(account_id, mailbox, is_deleted, timestamp DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_emails_thread_latest ON emails(account_id, thread_id, timestamp DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_emails_sender_domain ON emails(account_id, sender_domain, thread_id);
CREATE INDEX IF NOT EXISTS idx_emails_domain_stats ON emails(account_id, is_deleted, sender_domain);
CREATE INDEX IF NOT EXISTS idx_emails_sender_stats ON emails(account_id, is_deleted, sender_email);
CREATE INDEX IF NOT EXISTS idx_emails_domain_filter ON emails(account_id, is_deleted, sender_domain, thread_id);
CREATE INDEX IF NOT EXISTS idx_emails_sender_filter ON emails(account_id, is_deleted, sender_email, thread_id);

-- ── Recreate the FTS delete trigger (dropped with the table) ──────────────
CREATE TRIGGER IF NOT EXISTS emails_fts_delete AFTER DELETE ON emails BEGIN
    DELETE FROM emails_fts WHERE email_id = old.id;
END;

-- ── Rebuild drafts table without context_id ───────────────────────────────
CREATE TABLE drafts_v2 (
    id TEXT PRIMARY KEY,
    email_id TEXT,
    account_id TEXT NOT NULL,
    to_addresses_json TEXT NOT NULL,
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    ai_generated INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (email_id) REFERENCES emails(id),
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);

INSERT INTO drafts_v2
    SELECT id, email_id, account_id, to_addresses_json, subject, body,
           ai_generated, status, created_at, updated_at
    FROM drafts;

DROP TABLE drafts;
ALTER TABLE drafts_v2 RENAME TO drafts;

CREATE INDEX IF NOT EXISTS idx_drafts_account ON drafts(account_id, status, updated_at DESC);

-- ── Drop the contexts table ───────────────────────────────────────────────
DROP TABLE IF EXISTS contexts;

-- ── Reset refinery history so V001 re-runs with the new checksum ─────────
-- If the table doesn't exist yet (DB predates refinery), this is a no-op.
-- If it does exist (after the first refinery startup), we drop the old V001
-- entry so refinery re-applies V001 with the updated checksum on next boot.
-- All V001 statements use IF NOT EXISTS, so re-applying is always safe.
DROP TABLE IF EXISTS refinery_schema_history;

COMMIT;
PRAGMA foreign_keys = ON;
SQL

echo "Done. contexts removed, refinery_schema_history reset."
echo "On next app startup, refinery will apply V001 fresh (all IF NOT EXISTS — safe)."
