# services/retrieval

## What this module owns

Hybrid retrieval combining FTS5 keyword search and sqlite-vec vector similarity search, fused with Reciprocal Rank Fusion (RRF).

- **mod.rs** — `retrieve(db, query, account_id, top_k) -> Result<Vec<RetrievedEmail>>` — the single entry point for all search-backed features (chat, agent search, filter suggestions)
- **rrf.rs** — pure Reciprocal Rank Fusion: `rrf(vector_hits, fts_hits, k) -> Vec<RankedEmail>`. No I/O — unit-testable with lists of email IDs.
- **dedup.rs** — thread-level deduplication: collapses multiple emails from the same thread into one representative result

## Dependencies

- `db/emails/search.rs` — FTS5 query execution
- `db/embeddings.rs` — nearest-neighbour vector queries via sqlite-vec
- `services/embeddings` — on-demand embedding for the query string

## Public surface

- `retrieve(db, query, account_id, k, embedding_model) -> Result<Vec<RetrievedEmail>>`
- `rrf::fuse(vector: &[RankedId], fts: &[RankedId], k: usize) -> Vec<RankedId>` (pure)

## What should NOT live here

- Chat message assembly — that is `services/chat`
- Embedding generation for emails — that is `services/embeddings`
- Filter suggestion logic — that is `services/filters`
