-- V027: the UIDVALIDITY each IMAP mailbox had when its mail was stored.
--
-- IMAP message ids embed a UID (`{account}::{uid}`, `{account}::SENT::{uid}`,
-- …), and a UID only identifies a message together with its mailbox's
-- UIDVALIDITY (RFC 3501 §2.3.1.1). A server that rebuilds a mailbox — a
-- migration, a restore from backup, an index repair — issues a new value and
-- renumbers the messages. Nothing recorded the old value, so nothing noticed:
-- new mail whose UID matched a stored id was dropped as "already synced", and
-- stored ids silently pointed at other messages.
--
-- One row per (account, mailbox); `mailbox` is the `emails.mailbox` value
-- ('inbox', 'sent', 'spam', 'trash', 'folder:<server path>'). A mailbox with no
-- row has simply not been checked yet — the first sync after this migration
-- records its baseline.
CREATE TABLE IF NOT EXISTS folder_uid_validity (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox TEXT NOT NULL,
    uid_validity INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, mailbox)
);
