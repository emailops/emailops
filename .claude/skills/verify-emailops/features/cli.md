# CLI and agents

`emailops-cli` is the headless front-end over the same services the app calls. With `--json`
every command prints one envelope `{ok, data, error}` and exits with a code grouped by
remediation (0 ok, 2 invalid input, 3 not found, 4 auth, 5 network/sync, 6 AI). Read commands
open the database read-only and never run migrations.

## Sub-features

- `cli.envelope` success and failure share one JSON shape; `error` is `{code, params, message}`.
- `cli.readOnly` read commands (`doctor`, `search`, `show`, `emails`, …) open the DB read-only.
- `cli.eval` `emailops-cli eval` runs the chat eval cases through the shared harness.
- `cli.exitCodes` failures exit with a code grouped by remediation (2 invalid input, 3 not found, 4 auth, 5 network/sync, 6 AI, 130 cancelled).
- `cli.accounts` `accounts` lists accounts; `accounts add gmail|outlook|imap` connects one.
- `cli.emails` `emails` lists recent mail, with `--limit/--offset/--mailbox/--category`.
- `cli.show` `show <id>` prints one email's headers and readable body.
- `cli.search` `search <query>` returns full-text hits, with paging and `--trace`.
- `cli.chat` `chat <question…>` streams answers with sources.
- `cli.sync` `sync [account]` downloads new mail.
- `cli.classify` `classify [--all|--id]` classifies new, all or one email.
- `cli.junk` `junk` scores mail, explains a verdict, trains, and manages the private golden set.
- `cli.embed` `embed [--batch N]` generates missing embeddings.
- `cli.doctor` `doctor` reports DB, accounts and AI readiness without loading a model.
- `cli.stats` `stats` prints the per-account dashboard counts.
- `cli.skills` `skills` lists the skills folder and load errors.
- `cli.compose` `compose` saves a draft (pushed to the provider when supported) or `--send`s it, with attachments.
- `cli.drafts` `drafts` lists saved drafts.
- `cli.draft` `draft <id>` shows a draft; `--delete` removes it locally and at the provider.
- `cli.attachmentSuggestions` `attachment-suggestions` previews, lists, refreshes, dismisses, accepts and restores suggested attachment rules.
- `cli.calendar` `calendar [--days N|--next] [--sync]` lists upcoming events for one account.
- `cli.translate` `translate <id> [--to LANG|--detect-only]` detects and translates an email.
- `cli.config` `config get|set|unset|list` manages CLI-local preferences (default account).
- `cli.repl` bare `emailops-cli` opens a REPL: `/command`s, multi-turn `/chat`, `/account` and `/model` (both saved), `/new`, and a pasted id opens that email.
- `cli.promptOverride` `--prompt id=file` / `--system-prompt file` override a prompt template for this run only.

## How to get to it (user POV)

- `make cli-fast ARGS="… --json"` (no llama.cpp), `make cli-run`, `make cli-demo` (demo data dir).
- One column in `## Parity`: **emailops-cli --json**. The bare-`emailops-cli` REPL re-parses each `/command` into the same `Command` and dispatch, so it is the row `cli.repl`, not a column.

## Parity

| Capability | emailops-cli --json |
|---|---|
| cli.envelope | rust:src-tauri/src/cli/output.rs::ok_envelope_wraps_data_with_null_error |
| cli.readOnly | rust:src-tauri/src/cli/session.rs::read_only_open_does_not_migrate_the_db |
| cli.eval | gap: untested — the eval module's tests need the eval feature and verify_all runs cargo test --features cli only |
| cli.exitCodes | rust:src-tauri/src/cli/output.rs::exit_code_groups_errors_by_remediation |
| cli.accounts | rust:src-tauri/src/cli/commands.rs::dispatch_accounts_succeeds_against_seeded_db |
| cli.emails | gap: untested — only the no-account error is tested; no listing is checked |
| cli.show | gap: untested — only the not-found path is tested; the verify_all contract run cannot be cited in a cell |
| cli.search | gap: untested — no dispatch test; only the verify_all contract run, which cannot be cited, and renderer tests |
| cli.chat | gap: untested — run_chat has no test; evals use the shared harness, not this command |
| cli.sync | gap: untested — only the unknown-account error is tested |
| cli.classify | gap: untested — clap parsing only |
| cli.junk | gap: untested — the junk branch and the golden set have no test |
| cli.embed | gap: untested — clap parsing only |
| cli.doctor | rust:src-tauri/src/cli/doctor.rs::report_is_ok_with_an_enabled_account |
| cli.stats | gap: untested — clap parsing only |
| cli.skills | gap: untested — the dispatch test runs with no data dir and asserts Ok only |
| cli.compose | gap: untested — run_compose has no test |
| cli.drafts | gap: untested — the drafts branch has no test |
| cli.draft | gap: untested — show and --delete have no test |
| cli.attachmentSuggestions | rust:src-tauri/src/cli/commands.rs::dispatch_accept_marks_a_persisted_suggestion_accepted |
| cli.calendar | gap: untested — the calendar branch has no test |
| cli.translate | gap: untested — the translate branch has no test |
| cli.config | rust:src-tauri/src/cli/config.rs::set_default_account_persists_canonical_id |
| cli.repl | rust:src-tauri/src/cli/repl.rs::switch_account_persists_choice_as_default |
| cli.promptOverride | rust:src-tauri/src/cli/mod.rs::build_prompt_overrides_reads_files_and_maps_system_shorthand |

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
