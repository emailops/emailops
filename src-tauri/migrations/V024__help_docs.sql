-- "EmailOps help" corpus: the bundled user guides (docs/site/<lang>/<page>.md),
-- one row per heading section (or part of a long section), indexed for FTS
-- and vector retrieval so the chat can answer questions about the app itself.
-- Rebuilt from the binary whenever the embedded guides change
-- (`help_docs.corpus_hash` preference); embeddings are filled in lazily by
-- the active embedding provider and tagged with its model name.
-- Mirrors the memory_fact_chunks / vec_memory_facts / memory_facts_fts split.
CREATE TABLE IF NOT EXISTS help_doc_chunks (
    rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    chunk_id TEXT NOT NULL UNIQUE,
    lang TEXT NOT NULL,
    page TEXT NOT NULL,
    section_index INTEGER NOT NULL,
    part INTEGER NOT NULL DEFAULT 0,
    anchor TEXT NOT NULL DEFAULT '',
    page_title TEXT NOT NULL,
    heading TEXT NOT NULL,
    content TEXT NOT NULL,
    nav_target TEXT,
    embedding_model TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_help_doc_chunks_section ON help_doc_chunks(page, section_index, lang);
CREATE INDEX IF NOT EXISTS idx_help_doc_chunks_model ON help_doc_chunks(embedding_model);

-- Full-text index. `heading` carries "<page title> › <heading>" so a query
-- naming the feature matches even when the body never repeats the word.
CREATE VIRTUAL TABLE IF NOT EXISTS help_docs_fts USING fts5(
    chunk_rowid UNINDEXED,
    heading,
    content,
    tokenize='porter unicode61'
);

-- Vector index; rowid matches help_doc_chunks.rowid. Same dimension and
-- metric as vec_emails so one embedding provider serves both.
CREATE VIRTUAL TABLE IF NOT EXISTS vec_help_docs USING vec0(
    embedding float[768] distance_metric=cosine
);
