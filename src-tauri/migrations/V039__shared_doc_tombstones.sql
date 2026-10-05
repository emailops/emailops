-- V039: documents deleted on this install (services::shared_docs::delete).
--
-- A deleted document's row, history and search entry are removed; only its id
-- is remembered here, so later mail about it from other participants is
-- ignored instead of arriving as a new invitation.
CREATE TABLE IF NOT EXISTS shared_doc_tombstones (
    account_id  TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    doc_id      TEXT    NOT NULL,
    deleted_at  INTEGER NOT NULL,
    PRIMARY KEY (account_id, doc_id)
);
