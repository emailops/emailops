# EmailOps - AI-Native Email Client

## Project Overview

EmailOps is a privacy-first, AI-native email client. AI runs locally by default via an embedded llama.cpp runtime; users can optionally route to a local Ollama instance or to remote providers (OpenRouter). The app supports multiple email accounts with context-based organization.

## Tech Stack

| Layer | Technology |
|-------|------------|
| Desktop Framework | Tauri 2.x |
| Backend | Rust |
| Frontend | React + TypeScript |
| Styling | Tailwind CSS |
| State Management | Zustand |
| Local Database | SQLite (via rusqlite) |
| Vector Search | sqlite-vec |
| Local AI | Embedded llama.cpp (default); optional Ollama (HTTP API) |
| Remote AI (opt-in) | OpenRouter |
| Email Providers | Gmail API, Microsoft Graph API, IMAP/SMTP |

## Sub-area Instructions

- **Backend (Rust + Tauri):** `src-tauri/CLAUDE.md` — Rust coding standards, database conventions, production guardrails, testability practices, build hygiene, backend lessons learned.
- **Frontend (React + TypeScript):** `src/CLAUDE.md` — component structure, Tauri API call centralization, Zustand patterns, frontend robustness, frontend lessons learned.

## Local Workflow — Check the Makefile First

Common development operations live in the root `Makefile`. **Before reaching for an ad-hoc `cargo …`, `npm …`, `python …`, or `sqlite3 …` command, run `grep -E '^[a-z][a-z0-9_-]*:' Makefile` (or skim the file) to see if there is already a target for what you need.** Targets are the canonical, tested way to invoke local tooling for this repo. Examples (non-exhaustive):

- **App run:** `make dev` (repo-local data dir) / `make dev-fresh` (throwaway data dir) / `make dev-trace` (tracing feature enabled)
- **Demo data:** `make demo-db` / `make demo-embed` / `make demo` (run app against demo DB) — plus `-es` variants for Spanish demo data
- **Quality gates:** `make gates SET=commit|push|rust|frontend|all` (one summary line per gate, full output in files — `scripts/gates.sh`, which the lefthook pre-commit clippy/rustfmt/typecheck and pre-push hooks also call), `make check`, `make lint`, `make fmt`, `make test`, plus `-fast` variants (`test-fast`, `lint-fast`, `clippy-fast`, `check-fast`) that skip the embedded llama.cpp feature for faster iteration
- **Release / signing:** `make bootstrap-mac`, `make build-mac`, `make verify-mac`; Linux/Windows equivalents: `make bootstrap-linux`/`bootstrap-windows`, `build-linux`/`build-windows`, `verify-linux`/`verify-windows`, `dist-linux`/`dist-windows` — see "Release Builds" below
- **Hooks / deps:** `make install`, `make hooks`, `make audit`, `make clean`
- **Disk:** `make disk-report` (read-only) / `make disk-clean` (dry run; `ARGS="--apply"` frees orphaned and 3-day-idle worktree build dirs, `--release` also the per-arch release builds) — `/Volumes/Build` shares free space with the data volume

When you do need to run something the Makefile does not cover, prefer extending it (add a new target) over scattering one-off shell snippets across the codebase or your chat output — that way the next agent or developer can find it the same way.

**Keep the Makefile thin — it is an index, not an implementation.** A target's recipe should be a one-or-two-line invocation. Any recipe that needs multi-line shell logic (conditionals, loops, file generation, output capture) belongs in a standalone script under `scripts/`, called by a thin Makefile target (existing examples: `scripts/cli_bench.sh`, `scripts/eval_all.sh`, `scripts/fetch_bundled_models.sh`).

## Agent self-validation with `emailops-cli`

**Mandatory skill routing.** A screenshot, trace fragment or pasted answer showing wrong chat/AI behaviour means the first tool call of the turn is `Skill fix-ai-bug`, before any grep, CLI run or edit. A request to add or change an AI surface means the first tool call is `Skill build-ai-feature`. Do not reproduce the loop by hand: the skills carry the repro, gate and report format the developer expects.

- **`fix-ai-bug`** (`.claude/skills/fix-ai-bug/SKILL.md`): frame → CLI repro → root cause → fix → re-run until green → graduate the repro into an eval case. Graduation goes to the public `src-tauri/evals/` suite on the demo persona by default; `private-evals/` only when the case cannot be reproduced on the demo DB, and only after asking.
- **`build-ai-feature`** (`.claude/skills/build-ai-feature/SKILL.md`): pick the seam → check context-budget and KV-prefix cache fit → CLI baseline → TDD the pure planner → thin executor (+ frontend toggle) → mandatory eval gate.

`emailops-cli` (gated behind the `cli` cargo feature) is a headless front-end over the same `services::*` entry points the Tauri commands call — no `AppHandle`, no webview. Use it to **drive real features and assert on structured output** while developing, instead of guessing whether a change works. `make cli-*` runs it on `$EMAILOPS_DATA_DIR` (default: the checkout's `.emailops-data/`); `doctor --json` reports the data dir in use. Reads are safe while the app is open (SQLite WAL); run heavy writes — `sync`/`classify`/`embed` — with the app closed.

Every command supports `--json`, which prints one **stable envelope** to stdout so you can parse a single shape regardless of success or failure (logs always go to stderr):

```jsonc
{ "ok": true,  "data": { /* result */ }, "error": null }                       // success
{ "ok": false, "data": null, "error": { "code": "not_found", "params": {…}, "message": "…" } }  // failure
```

`error` is the same `{code, params, message}` shape `AppError` serializes to at the Tauri boundary. Exit codes are grouped by remediation: `0` ok, `2` invalid input, `3` not found, `4` auth, `5` network/sync, `6` AI, `130` cancelled, `1` otherwise — so a shell/agent can branch on failure class without parsing text.

**Always drive the CLI with `--json` from an agent.** The envelope is unstyled by contract (no ANSI, no markdown re-rendering); pretty output is styled only on an interactive TTY and degrades to plain text when piped or with `NO_COLOR` (`src-tauri/src/cli/render.rs`).

Recommended loop when validating a change:

```bash
make cli-fast ARGS="doctor --json"                 # confirm env is wired (DB, accounts, AI config) — no model load
make cli-fast ARGS="search 'invoice' --json"       # exercise read/search paths without booting llama.cpp
make cli-run  ARGS="chat 'what did X say?' --json --trace"  # full AI path; --trace surfaces route/retrieval/tool timings + sources under data.trace
make cli-eval ARGS="--tier smoke --json"           # re-run chat eval cases through the shared harness (needs cli,eval)
```

- `doctor` is read-only and fast — start here to verify the CLI is pointed at a usable install.
- `chat --trace` lets you inspect the routing decision, retrieval stats, and tool calls behind an answer (the same `ChatTrace` persisted on the assistant message), which is invaluable for diagnosing why a reply changed.
- `eval` reuses `case_loader` + `harness` + `metrics` (heuristics only — no judge, no HTML report, and it does **not** mutate provider preferences on the DB). It runs each case in a throwaway conversation that is deleted afterwards. It does **not** replace the `examples/*_eval.rs` harnesses mandated for AI-reply changes — it's a faster inner-loop check.

Prefer the `make cli-*` targets over ad-hoc `cargo run` invocations (see "Local Workflow"). The CLI shares the `Command` enum and dispatch with the interactive REPL (bare `emailops-cli`), so anything you can script you can also drive interactively.

## Release Builds

Build, sign and verify releases only through the Makefile targets (`build-mac`/`verify-mac`, `build-linux`/`build-windows`, `verify-*`, `dist-*`, `cask`); never hand-roll `npm run tauri build`. `docs/RELEASE.md` holds the full procedure, the per-platform packaging gotchas and the release invariants — read it before touching a release target, `scripts/build_platform.sh`, signing, or the `dynamic-backends` feature.

- `ai::gpu_plan::embedded_runtime_supported` keeps the embedded AI runtime off Intel Macs: the universal DMG's x86_64 slice carries Metal kernels it cannot run. It is load-bearing — do not weaken it.

## Architecture Principles

### Privacy First
- All email data stored locally in SQLite
- OAuth tokens stored in OS keychain (not plain files)
- No telemetry, no cloud sync, no external API calls except to email providers or AI providers (only when user chooses to use remote LLMs)
- **NEVER include real names, email addresses, subjects, or other real personal data in git-tracked files** — not in test fixtures, not in code comments, not in regression test cases, not in commit messages, not in PR descriptions. When a real-world failure surfaces a personal-data string, paraphrase it into a synthetic equivalent that preserves the technical shape (length, multibyte boundaries, regex pattern, etc.) but drops the identifying content. Real bench output goes under `src-tauri/reports/` which is gitignored — keep it there.
- **Check it with the `privacy-reviewer` subagent** (`.claude/agents/privacy-reviewer.md`) before every commit (scope `staged`), before a push or PR (scope `branch`, plus the PR title/body as `text`), and whenever a real personal-data string may have reached a tracked file. Resolve every `LEAK` with the synthetic replacement it proposes and ask the developer about every `UNSURE` before going on.
- The pre-commit hook also runs `scripts/privacy_scan.py`, a deterministic floor under the reviewer: it blocks staged mail stores (`*.db*`, `*.sqlite`, `*.eml`, `*.mbox`), app data dirs, and added lines containing an entry of the clone-local `.git/info/private-strings` (real addresses and names, never versioned).

### Separation of Concerns
- Tauri commands are thin wrappers that delegate to service modules
- Business logic lives in service modules, not in commands
- Frontend handles UI state only, backend handles data state

### Error Handling
- NEVER discard or ignore errors: log them, handle them, and surface them to users when relevant. Rust specifics (`thiserror`, `?`, typed errors up to the command boundary) live in `src-tauri/CLAUDE.md`.

## Project Structure

```
emailops/
├── src-tauri/       # Rust/Tauri backend, DB, sync, AI, migrations
├── src/             # React/TypeScript frontend
├── private-evals/   # Local-only private eval cases and runbooks
├── scripts/         # Developer and data-generation scripts
├── Makefile         # Canonical local workflow entrypoint
└── package.json     # Frontend dependencies and npm scripts
```

## Decision Log

Durable product and architecture decisions live in `docs/DECISIONS.md` (git-tracked,
append-only). **Consult it before proposing changes that touch a recorded decision, and
append an entry (same change/PR) whenever the developer makes a durable decision in a
session** — product direction, architecture choice, deliberate trade-off. Follow the
entry format documented at the top of that file. Session-level gotchas and in-flight
work state do not belong there.

## Cross-cutting Production Guardrails

### HTML / WebView Safety
- Never sanitize email HTML with regex replacements; use a dedicated HTML sanitizer with an allowlist-based policy
- Treat all provider HTML, plain-text-to-HTML conversions, inline images, and links as untrusted input
- Only allow safe URL schemes (`https`, `http`, `mailto` when explicitly intended); never open `javascript:`, `file:`, or arbitrary custom schemes from email content
- Keep the Tauri/WebView CSP enabled in production; do not set `"csp": null`

### Logging / Output Panel (shared contract)
Every user-facing operation must emit log entries so they appear in the output panel:
- **Backend** emits via `app.emit("app-log", AppLogEvent { level, source, message })` — see `src-tauri/CLAUDE.md`.
- **Frontend** uses `addLog(level, source, message)` from `useLogStore` — see `src/CLAUDE.md`.
- **Levels**: `info` (start), `success` (completion), `error` (failure), `debug` (verbose progress)
- **Sources**: `sync`, `embeddings`, `account`, `ai`, `system`

## Git Conventions
- On a `feature/*`, `fix/*` or `refactor/*` branch, commit at every green checkpoint (gates below pass) without asking. If on `main`, create the branch first. NEVER push unless the developer explicitly asks.
- NEVER include claude or other agent as author or co-author in commits
- Before committing, pushing or opening a PR, run the `privacy-reviewer` subagent (see Privacy First) alongside the gates.
- After implementing a feature, bug fix, or any moderate change, run the quality gates through the `gate-runner` subagent (`.claude/agents/gate-runner.md`, which runs `make gates`) with the `commit` set (clippy with and without `--features desktop`, rustfmt, biome, tsc — the exact CI flags live in `scripts/gates.sh`) and act on its report. Fix every reported issue before handing the change back to the developer — do not rely on the developer to discover lint/type/format failures. The `no-invoke-outside-api`, JSX-literal and docs guards run in the lefthook pre-commit hook itself.
- For changes that affect Rust behavior, also run the `gate-runner` `push` set (`cargo test`, both clippy variants, `cargo-outdated`, `vitest` — matches the pre-push hook) before declaring the work done.

### Branch Naming
- `feature/description` - New features
- `fix/description` - Bug fixes
- `refactor/description` - Code refactoring

### Commit Messages
- Format: `type: short description`
- Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`
- Keep under 72 characters
- Example: `feat: add Gmail OAuth flow`

## Testing Strategy

### Test-Driven Development (mandatory)
- **Always follow TDD.** For every new feature, bug fix, or behavior change, write the failing test first, watch it fail for the right reason, then write the minimum code to make it pass, then refactor. No exceptions — including refactors, which must be covered by tests before they begin.
- **Red → Green → Refactor.** Do not write production code without a failing test that requires it. Do not add untested branches "for safety" — if a branch matters, a test must drive it.
- **One behavior per test.** Tests describe behavior, not implementation. Prefer small, table-driven tests over large multi-assertion blocks.
- **Bug fixes start with a regression test.** Reproduce the bug as a failing test first, then fix. The test and fix land in the same PR (see "Regression test in the same PR as a bug fix" in `src-tauri/CLAUDE.md`).
- **Use the trait seams.** When a test would require mocking `reqwest`, `rusqlite`, the OS clock, the keychain, or `AppHandle`, route through the existing trait fakes (`EmailProvider`, `AIProvider`, `Clock`, `Keychain`, `Logger`) instead of reaching for heavy mocks. If a seam is missing, add it as the first step.
- **Pure planners first.** Extract pure decision functions and unit-test them exhaustively before wiring the thin executor. Integration tests cover the executor against fakes.
- **Prefer `test-driven-development` skill.** When implementing any logic, fixing any bug, or changing any behavior, invoke the `test-driven-development` skill to drive the work.

Language-specific testing guidance: `src-tauri/CLAUDE.md` (Rust) and `src/CLAUDE.md` (TypeScript).

## Security Checklist

Beyond Privacy First and HTML / WebView Safety above:
- No secrets in code or config files
- Validate all input from the frontend before processing it
- Use HTTPS for every external API call
- Never log secrets, tokens or email content

## Performance Guidelines

- Paginate email lists (50 items per page)
- Use virtual scrolling for long lists
- Batch AI operations (triage multiple emails at once)
- Index frequently queried columns in SQLite
- Cache thread summaries in database
