-- V038: EO Docs — personal folders, version history and full-text search for
-- shared documents (V037).
--
-- `shared_doc_folders`: this install's own folders, nestable (`parent_id`).
-- Never mailed: each person organizes their documents their own way, and
-- sharing stays per document.
CREATE TABLE IF NOT EXISTS shared_doc_folders (
    id          TEXT    PRIMARY KEY,
    account_id  TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    parent_id   TEXT    REFERENCES shared_doc_folders(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL CHECK (name <> ''),
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_shared_doc_folders_parent ON shared_doc_folders(account_id, parent_id);

-- The folder a document sits in here; NULL is the top level. A deleted
-- folder's documents move back to the top level.
ALTER TABLE shared_docs ADD COLUMN folder_id TEXT REFERENCES shared_doc_folders(id) ON DELETE SET NULL;

-- `shared_doc_versions`: what a document looked like after each change, for
-- the history panel. `author` is who made it (this account for local edits,
-- the sender for a change that arrived by email); `state` is the whole
-- document then (Yjs v1). Local edits made in one sitting share one row.
CREATE TABLE IF NOT EXISTS shared_doc_versions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id      TEXT    NOT NULL REFERENCES shared_docs(id) ON DELETE CASCADE,
    author      TEXT    NOT NULL,
    origin      TEXT    NOT NULL CHECK (origin IN ('local', 'remote')),
    created_at  INTEGER NOT NULL,
    state       BLOB    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_shared_doc_versions_doc ON shared_doc_versions(doc_id, id);

-- Full-text index over each document's title and text (a sheet's cell
-- values), refreshed whenever its content changes.
CREATE VIRTUAL TABLE IF NOT EXISTS shared_docs_fts USING fts5(
    doc_id UNINDEXED,
    title,
    body,
    tokenize='unicode61 remove_diacritics 2'
);
