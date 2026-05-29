# services/memory

## What this module owns

Extraction and lifecycle management of two kinds of persistent knowledge from emails:

1. **Facts** — durable knowledge about people, projects, and preferences (e.g. "Alice is the CFO at Acme")
2. **Tasks** — actionable items the user committed to or was asked for (e.g. "Send contract by Friday")

Files:

- **extractor.rs** — calls AI to extract facts/tasks from a single email body; pure extraction logic
- **consolidation.rs** — periodically merges duplicate facts and promotes high-confidence ones
- **embeddings.rs** — generates vector embeddings for facts so they can be retrieved in chat context
- **config.rs** — per-account enable/disable + extraction model preferences
- **header.rs** — shared prompt header injected into chat turns for relevant facts
- **mod.rs** — public surface + backfill orchestration

## Dependencies

- `db/memory.rs` — all SQL for facts, tasks, pending extractions
- `ai/provider.rs` — `AIProvider` for extraction and consolidation calls
- `services/clock` — for `now_secs()` in scheduled consolidation
- `services/logger` — log seam for backfill progress events

## Public surface

- `extract_facts_for_email(db, email_id, account_id, ai) -> Result<Vec<MemoryFact>>`
- `extract_tasks_for_email(db, email_id, account_id, ai) -> Result<Vec<PendingTask>>`
- `run_consolidation(db, account_id, ai) -> Result<usize>`
- `build_memory_header(db, account_id, query) -> Result<String>`

## What should NOT live here

- Chat conversation management — that is `services/chat`
- Task list UI (frontend) — that is `src/components/Memory/`
- Email sync — that is `services/emails`
