-- V041: drafts gone from this install whose provider copy is still there.
--
-- Deleting or sending a draft removes its row at once, but the provider's copy
-- can only go while the provider answers. Its id is remembered here until it
-- has, so the next pull retries the delete instead of taking the copy for a
-- draft written on another device and bringing it back.
CREATE TABLE IF NOT EXISTS draft_tombstones (
    account_id         TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider_draft_id  TEXT    NOT NULL,
    deleted_at         INTEGER NOT NULL,
    PRIMARY KEY (account_id, provider_draft_id)
);
