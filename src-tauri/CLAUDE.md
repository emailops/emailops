# Backend (src-tauri/) — Rust + Tauri 2

Instructions specific to the Rust/Tauri backend. The root `CLAUDE.md` covers
project-wide architecture, security checklist, performance budgets, and
cross-cutting policies — read it first. Frontend conventions live in
`src/CLAUDE.md`.

## Layout

```
src-tauri/
├── src/
│   ├── main.rs            # Tauri entry, app setup
│   ├── lib.rs             # Module exports, crate-level lints
│   ├── commands/          # Tauri command handlers (thin layer)
│   ├── services/          # Business logic (planners + executors)
│   ├── db/                # Database operations
│   ├── sync/              # Email provider sync
│   ├── ai/                # Ollama / llama.cpp integration
│   ├── evals/             # Shared eval machinery (library)
│   └── models/            # Data structures
├── examples/              # Eval harnesses & ad-hoc tools — thin wrappers (~15 lines).
│                          # Declared as Cargo `[[example]]` (not `[[bin]]`) so the
│                          # tauri-bundler does NOT enumerate them. Invoke via
│                          # `cargo run --features eval --example <name>`.
├── migrations/            # Versioned SQL migrations
├── capabilities/          # Tauri 2 capability files
├── reports/evaluations/   # All eval reports
├── Cargo.toml
├── clippy.toml            # allow-unwrap-in-tests, allow-expect-in-tests
├── .cargo/config.toml     # linker config, incremental
└── tauri.conf.json
```

## Coding Standards

### Naming Conventions
- Modules: `snake_case` (e.g., `email_sync.rs`)
- Types/Structs/Enums: `PascalCase` (e.g., `EmailAccount`)
- Functions/Variables: `snake_case` (e.g., `fetch_emails`)
- Constants: `SCREAMING_SNAKE_CASE` (e.g., `MAX_SYNC_BATCH`)

### Type Safety
- Prefer enums over string literals for any value with a fixed set of variants (e.g., provider names like `"gmail"` / `"outlook"`, account types, sync states).

### Error Handling
```rust
// Define custom errors with thiserror
#[derive(Debug, thiserror::Error)]
pub enum EmailError {
    #[error("Authentication failed: {0}")]
    AuthError(String),
    #[error("Database error: {0}")]
    DbError(#[from] rusqlite::Error),
    #[error("Sync failed: {0}")]
    SyncError(String),
}

// Use Result type alias for cleaner signatures
pub type Result<T> = std::result::Result<T, EmailError>;
```

- Use `thiserror` for custom error types.
- Propagate with `?`; handle at the command boundary.
- Return user-friendly error messages to the frontend.
- Log detailed errors locally for debugging.
- NEVER discard or ignore errors — they must always be logged and handled, and surfaced to users when relevant.

### No `.unwrap()` / `.expect(...)` in production code

The crate root in `src-tauri/src/lib.rs` denies `clippy::unwrap_used` and
`clippy::expect_used` for the entire crate. Tests are exempted via
`clippy.toml` (`allow-unwrap-in-tests = true` / `allow-expect-in-tests = true`).

When generating Rust code, **never** reach for `.unwrap()` or `.expect(...)`
on a `Result` or `Option` in production paths. The default move is one of:

- **Propagate with `?`** when the enclosing function already returns a
  `Result`. This is correct ~95% of the time.
- **Map and propagate** when the error type doesn't match:
  `result.map_err(|e| AppError::SyncError(e.to_string()))?`.
- **Pattern-match** (`match` / `if let`) when each branch needs different
  behavior.
- **Recover from poisoned mutexes** with
  `lock().unwrap_or_else(PoisonError::into_inner)` instead of `.expect("…")` —
  a previous panic on another thread should not cascade through every
  subsequent caller.

The lint may only be opted out site-by-site, with a justification comment.
The legitimate exceptional cases (and the form to use) are:

```rust
// 1. Hard-coded literal that cannot fail by construction
#[allow(clippy::unwrap_used)]
let re = Regex::new(r"\b\d{4}\b").unwrap();  // syntax checked at build time

// 2. Mutex poisoning where there is no meaningful recovery
#[allow(clippy::expect_used)]
let mut guard = m.lock().expect("cancel-flags mutex poisoned");

// 3. Startup-time fail-fast in `pub fn run()` — the app cannot continue
//    without a DB / data dir, so panic with a descriptive message
#[allow(clippy::expect_used)]
pub fn run() {
    let db = Database::new(dir).expect("Failed to initialize database");
    // …
}
```

Rules for the opt-out:

- Always co-locate `#[allow(...)]` with **one** call site or function,
  never an entire module or the crate root.
- Always pair it with a **one-line comment** explaining why the panic is
  acceptable. "Infallible by construction", "no recovery possible", and
  "fail-fast at startup" are the three accepted patterns; anything else
  means the code should be returning a `Result` instead.
- Prefer `.expect("descriptive message")` over `.unwrap()` when you do
  panic — the message lands in the backtrace and helps debugging.

If you find yourself wanting to reach for `.unwrap()` to "silence the
compiler", you are almost certainly looking at an error path that needs a
real handler. Add the variant to `AppError`, use `?`, and move on.

### Tauri Commands
```rust
// Commands should be async, return Result, and be thin wrappers
#[tauri::command]
pub async fn get_emails(
    state: State<'_, AppState>,
    context_id: Option<String>,
) -> Result<Vec<Email>, String> {
    services::email::get_emails(&state.db, context_id)
        .await
        .map_err(|e| e.to_string())
}
```

- Tauri commands are thin wrappers that delegate to service modules. Business logic lives in services, not commands.
- Only `commands/` performs the final `.map_err(|e| e.to_string())` at the Tauri boundary — `services/` and `db/` keep their typed errors.

### Data Structures
```rust
// Use serde for serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Email {
    pub id: String,
    pub account_id: String,
    pub subject: String,
    pub sender: String,
    pub body: String,
    pub timestamp: i64,
    pub is_read: bool,
}
```

## Database Conventions

### Table Naming
- Tables: `snake_case`, plural (e.g., `accounts`, `emails`)
- Foreign keys: `{referenced_table_singular}_id` (e.g., `account_id`)

### Enum columns
SQLite has no native enum type; store them as `TEXT`. For **new** tables or columns,
add a `CHECK` constraint to enforce the allowed set at the DB level:
```sql
status TEXT NOT NULL DEFAULT 'open'
    CHECK (status IN ('open', 'done', 'snoozed', 'dismissed')),
```
Existing `TEXT` columns without `CHECK` (e.g. `triage_status`, `category`, `mailbox`)
are not worth rebuilding tables for — the application layer validates them instead.
Do **not** add constraints to existing tables without a migration that also rebuilds
any indexes that reference the column (SQLite requires a full table-rebuild to add
a `CHECK` constraint after the fact).

### Migrations

Schema evolution is managed by the [`refinery`](https://docs.rs/refinery) crate.
Migrations are embedded at compile time via `refinery::embed_migrations!("migrations")`
in `src/db/mod.rs` and run on every startup (no-ops for already-applied versions).
Applied versions are tracked in the `refinery_schema_history` table.

**Adding a new migration:**

1. Create `src-tauri/migrations/V{next}__short_description.sql`
   - `{next}` is the next integer after the highest existing version (e.g. `V002`, `V003`)
   - Use `IF NOT EXISTS` for `CREATE TABLE/INDEX/TRIGGER` — makes migrations safe to
     re-apply during development
   - Use `ALTER TABLE` (not `CREATE TABLE`) for adding columns to existing tables
2. Test it: `cargo test --no-default-features` — `Database::new_for_testing()` runs all
   migrations against an in-memory DB, so the `schema_parity_tests` will catch schema drift
3. Never edit or delete an applied migration file — create a new version instead
4. Never ignore migration errors; `run_migrations()` propagates them as `AppError::IoError`
   and startup aborts

**Reference:** `src-tauri/migrations/V001__init.sql` is the canonical baseline
schema. Read it to understand the full table/index/trigger structure.

### Queries
- Use parameterized queries, never string interpolation
- Wrap multi-statement operations in transactions
- Enable `PRAGMA foreign_keys = ON` on every SQLite connection before relying on foreign keys or cascades
- Account-scoped reads must require `account_id`; do not expose cross-account read paths by default
- `LIKE 'prefix%'` (no leading wildcard) is a B-tree range scan — fast on indexed columns. `LIKE '%infix%'` is always a full table scan — avoid it on large tables; route through FTS5 instead.
- `OR` in a WHERE clause prevents SQLite from combining multiple index paths; the planner falls back to a full table scan. Use `UNION` CTEs so each branch can execute its own index scan independently. Example: `(col LIKE ?a OR id IN (SELECT … FROM fts))` → split into `UNION` of two CTEs, each using its own index.

### Database Access Patterns
- All SELECT queries must use db.reader() — never db.connection() for reads
- All INSERT / UPDATE / DELETE / DDL must use db.connection() (write conn)
- Methods that do a read then a write (e.g. read a path, then delete the row) must use connection() for both to avoid TOCTOU races
- Schema migrations (ensure_* methods, ALTER TABLE, PRAGMA table_info) always use connection()
- `db.reader()` returns a connection from a pool of 4 read connections (try-lock round-robin). This prevents concurrent reads from serializing behind a single mutex. Never hold a reader connection longer than necessary — return it promptly so other queries can proceed.

### Query Performance on Large Mailboxes (47k+ emails, 6 GB+ DB)
- **Parameterized `LIKE ?` prevents index optimization.** SQLite can't know at plan time there's no leading wildcard. Convert `LIKE 'prefix%'` to explicit `>= / <` range bounds using an upper-bound helper function.
- **`NOT EXISTS` with `OR` is O(N²).** The OR in `(timestamp > ? OR (timestamp = ? AND id > ?))` prevents SQLite from using the thread-latest index efficiently. Use a scalar subquery instead: `e.id = (SELECT id FROM emails sub WHERE ... ORDER BY timestamp DESC, id DESC LIMIT 1)` — single index seek per row.
- **`GROUP BY + MAX(timestamp)` is ~250x faster than `NOT EXISTS`** for finding the latest email per thread. Use the three-step pattern: (1) find matching IDs via index, (2) get distinct thread_ids, (3) GROUP BY for latest-per-thread.
- **Covering indexes eliminate table lookups.** GROUP BY queries on `sender_domain` or `sender_email` must not touch the main table (which contains the huge `body` column). Include `is_deleted` in the index so the query is fully covered: `(account_id, is_deleted, sender_domain)`.
- **Choose query strategy based on filter selectivity:**
  - *Broad filters* (domain/sender matching many threads): scan `idx_emails_account_active` in timestamp order with LIMIT — early termination after 50 results.
  - *Selective filters* (tags matching few emails): drive from the small table (`email_tags`), GROUP BY over the small result set. Never scan all 47k emails looking for rare matches.
- **Skip `COUNT(*)` when not needed.** For infinite-scroll UIs, return `total_count = -1` instead of running a separate count query that doubles execution time.
- **SQLite parameter limit is 32,766.** Never materialize large ID sets as `IN (?1, ?2, ...)` parameters. Use CTE subqueries instead: `WHERE thread_id IN (SELECT thread_id FROM matched_threads)`.

### Batch writes for hot paths
- Sync loops and any operation that inserts/updates N rows must wrap them in a single transaction — never call db.insert_X() in a bare loop
- Use unchecked_transaction() for batches; always commit() explicitly and let ? propagate errors
- INSERT OR REPLACE / ON CONFLICT DO NOTHING inside batch transactions is safe — conflicts don't roll back the whole batch

## Production Readiness Guardrails

### OAuth / Credentials
- OAuth callback listeners must have connection/read timeouts and reject invalid or missing state
- Local OAuth listeners should continue waiting after malformed callback requests instead of failing on the first bad hit
- OAuth tokens belong in the OS keychain for production builds
- Any plaintext/dev token storage must be gated to debug-only workflows and must never be the default

### Sync / Data Integrity
- Sync paths must fail loudly on DB errors; never convert database failures into "already processed" or other success-like states
- Persist sync status transitions (`syncing`, `idle`, `error`) so the UI and logs reflect real state
- Multi-step destructive DB operations (for example account deletion) must run in a transaction
- Cleanup logic must account for related tables, foreign keys, and user data retention expectations

### Background Task Queue
- **Heavy operations must never block the UI thread.** Any operation that touches Ollama, syncs email, or does large DB scans must run in a background task queue — never directly inside a Tauri command that the frontend awaits synchronously.
- The pattern for user-triggered heavy work: return `Ok(())` immediately to the frontend, submit the task to the queue, and report progress/completion via `app-log` events. The frontend should show a loading indicator driven by those events, not by waiting on the command response.
- Use separate queues for Ollama-dependent tasks (`ai_queue`, concurrency 2) vs. fast DB-only tasks (`db_queue`, concurrency 4) so a long AI job cannot starve lightweight operations.
- Never use `let _ = sender.send(task)` — always handle the send error explicitly; log it and surface it in the output panel if the queue cannot accept the task.

### Backend Logging / Output Panel
- Long-running workflows should emit structured `app-log` events for start, progress, success, and failure via `app.emit("app-log", AppLogEvent { level, source, message })`.
- Prefer one logging path for user-visible operations; avoid mixing `println!` with UI logs for the same flow.
- Do not log secrets, OAuth tokens, or raw sensitive email bodies.

### Testing / CI
- Add a focused regression test for production-impacting bugs when practical
- `cargo test` should exercise real in-tree tests; avoid letting the backend ship with zero meaningful tests
- CI must run at least frontend build/type-check plus Rust check/tests on every PR
- Keep local data artifacts (`*.db`, caches, generated fixtures) out of Git via `.gitignore`
- Test databases (`new_for_testing()`) must include every virtual table (FTS5, vec0), trigger, and index that the methods under test rely on. Missing any of these causes tests that either crash or silently return wrong results — giving false confidence that search/filter logic is correct.

## Testability & Coding Agent Practices

These practices keep the codebase legible to both tests and AI agents. The same friction that makes a module hard to test makes it hard for an agent to reason about. Apply these by default to new code, and refactor toward them when touching old code.

### Trait seams at every external boundary
- Define a small trait at every I/O edge so the production type and a test fake are interchangeable. Inject the trait via `Arc<dyn …>` on `AppState`, never instantiate concrete clients inside services.
- Required seams:
  - `MailProvider` — Gmail / Outlook / IMAP / Fake (lives in `sync/provider.rs`)
  - `AiProvider` — local llamacpp / Ollama / OpenRouter / Claude / Fake
  - `Clock` — never call `SystemTime::now()` or `chrono::Utc::now()` directly inside services; take a `&dyn Clock`
  - `Keychain` — wrap the `keyring` crate; tests use an in-memory keychain
  - `Logger` — abstracts `app.emit("app-log", …)`; tests use a `VecLogger` that records events
- Rule of thumb: if a test would need to mock `reqwest`, `rusqlite`, the OS clock, the keychain, or `AppHandle`, the code is missing a seam.

### Pure planner + thin executor
- Split each service into a **pure planner** (input: data, output: a plan/decision/SQL/HTTP-request struct — no I/O) and a **thin executor** (does the I/O against the plan). The planner is unit-tested with table-driven cases; the executor is integration-tested with the trait fakes.
- Examples that must follow this pattern: sync scheduling (`plan_sync` → `execute_sync`), lens scope → SQL, RRF, classification, draft prompt assembly, filter suggestion.
- If a function returns `Result<T, _>` and also calls the network or the DB, look for the planner that should be extracted from it.

### `AppState` is constructible from parts
- `AppState` holds `Arc<dyn …>` for every trait above plus `Arc<Database>` and `Arc<TaskQueue>`. Provide two constructors:
  - `AppState::for_production(cfg: &Config) -> Result<Self>` — wires real implementations
  - `AppState::for_testing() -> Self` — wires fakes (in-memory DB via migrations, fake clock at a fixed instant, no-op keychain, VecLogger)
- This makes Tauri command tests possible, not just service tests.

### One source of truth for the test database
- `Database::new_for_testing()` MUST run the same migration code as production against an in-memory SQLite. Do not maintain a hand-written test schema.
- A startup-time test asserts every table, virtual table (FTS5, vec0), trigger, and index present in prod exists in the test DB. If you add schema, add the migration — the test DB updates for free.

### Typed errors all the way to the command boundary
- `services/` and `db/` return their typed error enums. Do **not** call `.map_err(|e| e.to_string())` inside services — keep the structured error so tests can match on variants.
- Only `commands/` performs the final `.map_err(|e| e.to_string())` at the Tauri boundary.

### Background queue uses a typed Task enum, not closures
- The task queue accepts an `enum Task { GenerateDraft { email_id }, ExtractLens { lens_id, email_id }, SyncAccount { account_id }, … }`, not `Box<dyn FnOnce()>`.
- Tests assert which tasks were enqueued. Agents see the full menu of background work in one place.
- Keep separate `ai_queue` (concurrency 2) and `db_queue` (concurrency 4) — see Background Task Queue guardrails.

### Evals are a library, not 16 binaries
- Shared eval machinery lives in `src-tauri/src/evals/` (report schema, judge harness, sampling, CLI flags, prod-DB connection). Each `examples/*_eval.rs` is a thin (~15 line) wrapper that configures + runs. Evals are Cargo `[[example]]` (not `[[bin]]`) so the tauri-bundler does not try to copy them into the packaged `.app`.
- All eval reports share one JSON schema: `{ run_id, eval_name, model, timestamp, total, succeeded, failed, judge_scores, per_item_results }`.
- LLM-as-judge prompts are calibrated against ~30 human labels before their scores are trusted in dashboards.

### Any change that can move AI replies must be verified by an eval
- "AI replies" = anything the chat, classifier, draft generator, memory extractor, lens extractor, or any other LLM-facing path emits to the user or downstream code. That includes: prompt edits, tool description / `prompt_summary` / parameter-schema changes, new or removed tools, new tool effects, retrieval / RAG ranking tweaks, post-processing on assistant content, and default-model bumps.
- Before declaring such a change done, run the matching `examples/*_eval.rs` (chat → `chat_eval`, draft → `draft_eval`, classification → `email_classification_eval`, lens → `lens_extract_eval`, memory → `memory_extract_eval`, task → `task_extract_eval`, invoice → `invoice_extract_eval`, agent search → `agent_search_eval`, chat shortcuts → `chat_shortcut_eval`). If no existing eval covers the new behaviour, add a focused case to the closest harness in the same PR.
- **Scope the eval run to cases related to the change.** Do not blast the full suite for an unrelated edit — that wastes minutes per run and dilutes the signal. Use `--case <id>` to target a single case, `--tier smoke` for the curated subset, or whatever case-set flag the specific harness exposes (e.g. `chat_eval -- --case kickoff_date_es`). The "all cases" sweep is reserved for default-model / shared-prompt changes that genuinely touch every path.
- Record the eval invocation and a one-line summary of the result in the PR description, alongside (or instead of) the regression-test note. A passing eval with the relevant cases is the AI-side equivalent of a passing unit test for deterministic code.

### Module READMEs for non-obvious modules
- Hairy modules (`services/emails/`, `services/lenses/`, `sync/`, `db/emails/`, `services/retrieval/`, `services/memory/`) carry a short `MODULE.md` at the directory root describing: what this module owns, what it depends on, the public surface, and what should NOT live here.
- Update the README in the same PR that changes the module's surface.

### Dev vs test data isolation
- `make dev` targets the ignored repo-local app data dir (`.emailops-data/`) via `EMAILOPS_DATA_DIR`.
- `make dev-fresh` targets a separate ignored repo-local data dir (`.emailops-data-fresh/`) — use this whenever you (or an agent) might mutate state during exploration.
- Override `EMAILOPS_DATA_DIR` privately in `.env.local` or on the command line when a workflow needs another app data directory.

### Regression test in the same PR as a bug fix
- Every production-impacting bug fix lands with a focused regression test in the same PR. No exceptions. If the test infrastructure does not yet exist to write one, that is the first PR — the fix is the second.

## Testing (Rust)
- Unit tests in same file with `#[cfg(test)]` module
- Integration tests in `tests/` directory
- Mock external APIs (Gmail, Ollama) in tests via the trait seams above — never instantiate real HTTP/DB clients in tests

## Build / Target Hygiene

Keep iterative Rust compile times fast and `src-tauri/target/` from ballooning.

- **Use the fast iteration Makefile targets** (`make test-fast`, `make clippy-fast`, `make lint-fast`, `make check-fast`) when you are NOT modifying the embedded llama.cpp paths. They pass `--no-default-features` to skip the heavy C++/cmake `llama-cpp-2` build. The non-`-fast` variants remain authoritative for CI and pre-commit hooks.
- **Compile profile is tuned in `src-tauri/Cargo.toml`**: `[profile.dev]` and `[profile.test]` use `debug = "line-tables-only"` (keeps backtraces, drops full DWARF) and dependencies build at `opt-level = 1`. Do not raise `debug` to `true` globally — use a one-off `RUSTFLAGS='-C debuginfo=2' cargo …` when actually attaching a debugger.
- **Linker config lives in `src-tauri/.cargo/config.toml`**: incremental compilation is explicit, and macOS link step strips with `-Wl,-S`. Don't add CARGO_INCREMENTAL=0 to scripts.
- **`crate-type = ["lib", "cdylib"]`** — `staticlib` was removed because it added a ~590 MB archive to every desktop relink. If iOS/Android Tauri builds ever land, restore `staticlib` only for those targets.
- **Watch for target/ bloat.** Healthy is ~5–15 GB on macOS for this project. If `du -sh src-tauri/target` exceeds ~30 GB, run `cargo clean --manifest-path src-tauri/Cargo.toml`. Common bloat sources: stale per-feature artifacts (`llamacpp` on/off, `eval` on/off), abandoned `aarch64-apple-darwin/` or `universal-apple-darwin/` mac-release dirs, and the `doc/` output. Periodically inspect with `du -sh src-tauri/target/* | sort -rh`.
- **Don't add new binaries to `src-tauri/[[bin]]` without thinking.** Each binary relinks on every change to `services/` and contributes ~50–70 MB to `target/debug/`. Prefer extending an existing eval bin or graduating eval machinery into a shared library (see "Evals are a library, not 16 binaries" above). If you must add one, gate it behind a feature so it doesn't compile by default.
- **Don't introduce wide `tokio` / `reqwest` / `serde` feature surfaces** when a narrow one will do. Audit `Cargo.toml` features when adding deps — `features = ["full"]` on tokio drags in everything; pick what you need.

## Lessons Learned

### Tauri 2 Specifics (backend / debugging)
- The app data directory is resolved from `EMAILOPS_DATA_DIR` when set, otherwise Tauri's platform default for the app identifier.
- `make dev` sets `EMAILOPS_DATA_DIR` to an ignored repo-local directory, so local runs do not depend on any external app data by default.
- **Inspecting the database while debugging.** For Makefile dev runs, the DB is under `.emailops-data/emailops.db`. Query it directly with `sqlite3`:
  ```bash
  DB=".emailops-data/emailops.db"
  sqlite3 "$DB" "SELECT id, subject, sender_email, timestamp FROM emails ORDER BY timestamp DESC LIMIT 5;"
  sqlite3 "$DB" "SELECT filename, mime_type, file_size FROM email_attachment_meta WHERE email_id = '<id>';"
  sqlite3 "$DB" ".schema emails"
  ```
  Use this when validating sync results, reproducing UI bugs, or checking that a fix actually wrote the rows you expect. The app keeps the DB open in WAL mode, so concurrent reads while the app is running are safe.
- `tokio::task::spawn_blocking` (and any Tokio future) cannot be called from the `.setup()` hook — the runtime hasn't started yet. Panics with "there is no reactor running". Use `std::thread::spawn` for any synchronous work that must happen at startup (e.g., database backup).

### Sync Performance
- Always use incremental sync: query Gmail with `after:{latest_timestamp}` instead of listing all messages and checking each against the DB. Reduces API calls from 500+ to typically 0-5 on subsequent syncs.
- Filter out already-existing emails before the download loop, not inside it. Show "Inbox up to date" immediately when nothing is new — don't make the user watch a "checking" progress bar.
- Progress messages should describe what's actually happening ("Downloading 3 new emails") not internal mechanics ("Found 500 emails to check").

### AI / Ollama Integration
- Never hardcode model names. Users have different models installed. Query `/api/tags` for available models and let the user pick. Store the preference in the DB.

### User Preferences
- Store preferences in SQLite (`user_preferences` key-value table), not localStorage. The app is a desktop app — localStorage ties to webview state which can be cleared.
- A simple `get_preference(key) -> Option<String>` / `set_preference(key, value)` on the Database struct covers most needs without per-feature tables.
