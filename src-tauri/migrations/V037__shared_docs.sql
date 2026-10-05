-- V037: shared documents and sheets, kept in sync between EmailOps installs by
-- email (services::shared_docs).
--
-- `shared_docs`: one row per document on this install. `state` is the merged
-- Yjs v1 update holding the whole document. `consented_at` is when the user
-- shared or accepted it: until then nothing about it is mailed. `dirty_since`
-- is set while changes wait to be mailed (NULL when clean).
CREATE TABLE IF NOT EXISTS shared_docs (
    id            TEXT    PRIMARY KEY,
    account_id    TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind          TEXT    NOT NULL CHECK (kind IN ('doc', 'sheet')),
    title         TEXT    NOT NULL,
    state         BLOB    NOT NULL,
    status        TEXT    NOT NULL CHECK (status IN ('invited', 'active', 'left')),
    consented_at  INTEGER,
    dirty_since   INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_shared_docs_account ON shared_docs(account_id, updated_at);

-- `shared_doc_participants`: the normalized addresses a document is shared
-- with (this account's own included). `state_vector` is what that peer is
-- known to have seen: reported in its last message, or assumed after ours
-- went out. NULL when nothing is known yet (it gets the whole document).
CREATE TABLE IF NOT EXISTS shared_doc_participants (
    doc_id        TEXT    NOT NULL REFERENCES shared_docs(id) ON DELETE CASCADE,
    address       TEXT    NOT NULL CHECK (address = lower(trim(address)) AND address <> ''),
    position      INTEGER NOT NULL,
    state_vector  BLOB,
    PRIMARY KEY (doc_id, address)
) WITHOUT ROWID;

-- `shared_doc_messages`: provider messages already applied, so a message seen
-- again (a resync, its Sent copy) is not processed twice.
CREATE TABLE IF NOT EXISTS shared_doc_messages (
    account_id    TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id    TEXT    NOT NULL,
    doc_id        TEXT    NOT NULL,
    seen_at       INTEGER NOT NULL,
    PRIMARY KEY (account_id, message_id)
) WITHOUT ROWID;
