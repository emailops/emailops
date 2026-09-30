-- Index every child column that references emails(id).
--
-- SQLite enforces a foreign key on a parent delete by looking the child rows
-- up by the referencing column. None of these columns led an index (the two
-- Lens tables and chat_message_sources only have it as the second or a
-- non-key column), so each deleted email full-scanned all five tables.
-- Deleting an account with a large mailbox stalled for minutes holding the
-- write lock.
CREATE INDEX IF NOT EXISTS idx_drafts_email ON drafts(email_id);
CREATE INDEX IF NOT EXISTS idx_chat_sources_email ON chat_message_sources(email_id);
CREATE INDEX IF NOT EXISTS idx_memory_facts_source_email ON memory_facts(source_email_id);
CREATE INDEX IF NOT EXISTS idx_lens_rows_email ON lens_rows(email_id);
CREATE INDEX IF NOT EXISTS idx_lens_exclusions_email ON lens_exclusions(email_id);
