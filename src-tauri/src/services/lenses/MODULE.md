# services/lenses

## What this module owns

AI-extracted, schema-typed tabular views over a user's mailbox — "Lenses".

- **scope.rs** — pure planner: `LensScope → Vec<email_id>` SQL evaluation. No I/O beyond a DB read. All tests in this file are unit tests against an in-memory DB.
- **extractor.rs** — thin executor: takes a `Lens` + list of email IDs → calls AI provider → writes `lens_rows` to DB
- **runner.rs** — orchestrates a full lens run: scope evaluation → batch extraction → status updates → abort handling
- **templates.rs** — built-in lens templates (invoice tracker, contact summary, etc.)
- **mod.rs** — public surface

## Dependencies

- `db/lenses.rs` — all SQL (lens CRUD, row upserts, run status)
- `ai/provider.rs` — `AIProvider` trait for extraction calls
- `services/clock` — for `now_secs()` in scope date-range filtering (via Clock seam)
- `services/logger` — log seam for run progress events

## Public surface

- `scope::evaluate(db, scope) -> Result<Vec<String>>`
- `scope::evaluate_with_limit(db, scope, limit) -> Result<Vec<String>>`
- `runner::run(db, lens_id, run_id, ai, abort) -> Result<()>`
- `templates::list_templates() -> Vec<LensTemplate>`

## What should NOT live here

- SQL schema — that is `db/lenses.rs` and the migration in `db/schema.rs`
- Frontend Lens component state — that is `src/stores/lensStore.ts`
- Classification tags — those are `services/classification`; lens extraction is separate
