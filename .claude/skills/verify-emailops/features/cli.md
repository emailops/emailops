# CLI and agents

`emailops-cli` is the headless front-end over the same services the app calls. With `--json`
every command prints one envelope `{ok, data, error}` and exits with a code grouped by
remediation (0 ok, 2 invalid input, 3 not found, 4 auth, 5 network/sync, 6 AI). Read commands
open the database read-only and never run migrations.

## Sub-features

- `cli.envelope` success and failure share one JSON shape; `error` is `{code, params, message}`.
- `cli.readOnly` read commands (`doctor`, `search`, `show`, `emails`, …) open the DB read-only.
- `cli.eval` `emailops-cli eval` runs the chat eval cases through the shared harness.

## How to get to it (user POV)

- `make cli-fast ARGS="… --json"` (no llama.cpp), `make cli-run`, `make cli-demo` (demo data dir).

## Driving it with verify.sh

No window: the CLI is driven as a process on the demo data dir.

- Envelope → `make cli-demo ARGS="doctor --json" | sed -n '/^{/,$p'` has exactly the keys `ok`, `data`, `error`.
- Read → `EMAILOPS_DATA_DIR=$PWD/.emailops-demo-data src-tauri/target/debug/emailops-cli search Ollama --json` → `ok: true`, one hit, exit 0, and `refinery_schema_history` unchanged.
- Failure → `… emailops-cli show no-such-id --json; echo $?` → `ok: false`, `error.code: "not_found"`, exit 3.

## Gotchas

- Through `make`, a failing command exits with make's own code (2); run the binary directly to read the CLI's exit code.
- The CLI module only compiles with `--features cli`: `cargo test` without it runs none of its tests. `make verify` runs `cargo test --features cli`.

| Case | Test kind |
|---|---|
| argument parsing, dispatch against an in-memory DB, renderers, REPL | unit (`cli::*`, run by `make verify` with `--features cli`) |
| envelope shape, error shape, read-only open that does not migrate | contract (`cli::output::tests::*envelope*`, `cli::session::tests::read_only_*`) |
| the built binary on the demo DB: `doctor`, a read (`search`), a failure (`show`) with its exit code | contract (layer `contract` of `verify_all.py`) |
| integration (`tests/integration.rs`) | n/a: `tests/integration.rs` is compiled without the `cli` feature; the dispatch tests inside the crate run every command against the in-memory DB |
| e2e | n/a: there is no window; the process-level runs above are the CLI's end to end |
| eval | n/a as a feature of its own: every chat eval case runs through `emailops-cli eval`, and is attributed to the feature the case is about |
