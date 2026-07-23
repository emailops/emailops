-- Mail folders discovered on the provider (IMAP LIST), both well-known roles
-- (persisted detection result) and user-created custom folders. `server_path`
-- is the raw wire name (modified UTF-7) used verbatim for SELECT; it is the
-- stable identity — a server-side rename appears as delete + create.
CREATE TABLE IF NOT EXISTS folders (
    id TEXT PRIMARY KEY,                -- '{account_id}:{server_path}'
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    server_path TEXT NOT NULL,
    display_name TEXT NOT NULL,         -- decoded UTF-8 full path
    role TEXT NOT NULL DEFAULT 'custom'
        CHECK (role IN ('sent', 'spam', 'trash', 'custom')),
    delimiter TEXT,
    last_seen_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (account_id, server_path)
);

CREATE INDEX IF NOT EXISTS idx_folders_account_role ON folders(account_id, role);
