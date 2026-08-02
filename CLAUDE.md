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
- **Quality gates:** `make check`, `make lint`, `make fmt`, `make test`, plus `-fast` variants (`test-fast`, `lint-fast`, `clippy-fast`, `check-fast`) that skip the embedded llama.cpp feature for faster iteration
- **Release / signing:** `make bootstrap-mac`, `make build-mac`, `make verify-mac`, `make build-mac-intel`, `make verify-mac-intel`; Linux/Windows equivalents: `make bootstrap-linux`/`bootstrap-windows`, `build-linux`/`build-windows`, `verify-linux`/`verify-windows`, `dist-linux`/`dist-windows` — see "Linux / Windows Release Builds" below
- **Hooks / deps:** `make install`, `make hooks`, `make audit`, `make clean`

When you do need to run something the Makefile does not cover, prefer extending it (add a new target) over scattering one-off shell snippets across the codebase or your chat output — that way the next agent or developer can find it the same way.

**Keep the Makefile thin — it is an index, not an implementation.** A target's recipe should be a one-or-two-line invocation. Any recipe that needs multi-line shell logic (conditionals, loops, file generation, output capture) belongs in a standalone script under `scripts/`, called by a thin Makefile target (existing examples: `scripts/cli_bench.sh`, `scripts/eval_all.sh`, `scripts/fetch_bundled_models.sh`).

## Agent self-validation with `emailops-cli`

**When the user reports a bug in chat, drafts, classification, lenses, retrieval, memory, or any other AI feature, drive the fix through the `fix-ai-bug` skill** (`.claude/skills/fix-ai-bug/SKILL.md`). It codifies the loop in this section — frame → CLI repro → root-cause → fix (with confirmation gated to genuine design forks) → re-run until green → gated graduation into `private-evals/` — and reports back in a chat-style format that hides the raw `--trace` payload by default. Invoke it whenever a bug report lands, even from a screenshot or trace fragment.

**When the user wants to add a new AI feature or change an existing AI surface** (a new chat tool, a planner/route, a prompt edit, a shortcut fast-path, a classifier/extractor, or a retrieval/draft/memory tweak), **drive the work through the `build-ai-feature` skill** (`.claude/skills/build-ai-feature/SKILL.md`). It runs the build loop — frame → pick the seam → check setup/config fit (context budget + KV-prefix prompt cache) → CLI baseline → TDD the pure planner → wire the thin executor (+ frontend toggle/gating) → mandatory eval gate → gated eval-case graduation — and reports a chat-style before/after delta.

`emailops-cli` (gated behind the `cli` cargo feature) is a headless front-end over the same `services::*` entry points the Tauri commands call — no `AppHandle`, no webview. Use it to **drive real features and assert on structured output** while developing, instead of guessing whether a change works. It operates on the real data dir (SQLite WAL → read commands are safe while the app is open; run heavy write commands — `sync`/`classify`/`embed` — with the app closed).

Every command supports `--json`, which prints one **stable envelope** to stdout so you can parse a single shape regardless of success or failure (logs always go to stderr):

```jsonc
{ "ok": true,  "data": { /* result */ }, "error": null }                       // success
{ "ok": false, "data": null, "error": { "code": "not_found", "params": {…}, "message": "…" } }  // failure
```

`error` is the same `{code, params, message}` shape `AppError` serializes to at the Tauri boundary. Exit codes are grouped by remediation: `0` ok, `2` invalid input, `3` not found, `4` auth, `5` network/sync, `6` AI, `130` cancelled, `1` otherwise — so a shell/agent can branch on failure class without parsing text.

**Always drive the CLI with `--json` from an agent.** The `--json` envelope is unstyled by contract — no ANSI color, no markdown/table re-rendering, no live-preview redraw — so it never adds presentation tokens to your context. Human (pretty) output is the opposite: chat answers are re-rendered through a terminal markdown renderer (aligned tables, styled prose) and lists/traces are colorized, but **only** on an interactive TTY (`RenderStyle::Rich`). Pretty output that is piped/redirected, or run with `NO_COLOR`, degrades to plain text with zero escape codes (`RenderStyle::Plain`). The styling lives entirely in `src-tauri/src/cli/render.rs` + the pretty renderers in `cli/output.rs`; it cannot leak into the `--json` path.

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

## macOS Release Builds

Use the Makefile release targets; do not hand-roll `npm run tauri build` commands for signed builds.

- One-time setup: run `make bootstrap-mac`, then create `.env.signing` from `.env.signing.example` with the Apple signing identity, certificate, Apple ID, app-specific password, and team ID.
- Apple Silicon / standard Mac release: run `make build-mac && make verify-mac`. This builds the signed and notarized `universal-apple-darwin` bundle and includes the embedded local AI provider plus bundled embedding model.
- Intel Mac release: run `make build-mac-intel && make verify-mac-intel`. This targets `x86_64-apple-darwin`, uses `src-tauri/tauri.intel.conf.json`, disables default features to omit embedded llama.cpp, and verifies no `.gguf` model files are bundled. Intel users can still configure Ollama or OpenRouter.
- Always run the matching `verify-*` target before publishing a DMG; it checks codesigning, architectures, Gatekeeper assessment, notarization stapling, and the Intel no-bundled-model guard.
- Homebrew: after the DMGs are uploaded to the GitHub release, run `make cask` to regenerate `homebrew/Casks/emailops.rb` from the release assets (GitHub-computed sha256 digests, per-arch URLs) and copy it into the `emailops/homebrew-tap` repo. Full flow and invariants: `homebrew/README.md`. Never replace a DMG asset on an already-tagged release — the cask pins its sha256.

## Linux / Windows Release Builds

Use the Makefile release targets (`scripts/build_platform.sh`, `scripts/verify_platform.sh`, `scripts/dist_platform.sh`, `scripts/bootstrap_platform.sh` — one script per concern, dispatched by platform argument); do not hand-roll `npm run tauri build` commands.

- One-time setup: run `make bootstrap-linux` (or `bootstrap-windows`). It only **checks** prerequisites and prints the exact install command for anything missing — it never runs `sudo apt-get` itself, so it's safe to run on a machine you don't want silently modified.
- Linux: `make build-linux && make verify-linux`. Builds `.deb` + `.AppImage` for `x86_64-unknown-linux-gnu`.
- Windows: `make build-windows && make verify-windows`. Builds `.msi` + NSIS `.exe` for `x86_64-pc-windows-msvc`. The Makefile requires a POSIX shell — run these from Git Bash or MSYS2, not `cmd.exe`/PowerShell.
- Set `CARGO_FEATURES=vulkan DYNAMIC_BACKENDS=1` on either build target to ship the Vulkan GPU backend as a loadable module the binary probes for at runtime (falls back to CPU when no compatible driver is present) — this is what CI's release job uses, and is the recommended default (covers AMD/Intel/NVIDIA off the user's normal driver). A plain `make build-linux`/`build-windows` with no env vars produces a CPU-only artifact. See `docs/DECISIONS.md` ("Windows and Linux releases build with Vulkan via dynamic backends") for why dynamic backends exist instead of a statically-linked GPU build.
- Set `CARGO_FEATURES=cuda DYNAMIC_BACKENDS=1` (Windows or Linux) for an NVIDIA-only alternative — faster to build (needs the CUDA Toolkit, not the Vulkan SDK) but measured slower at runtime than Vulkan for the same model/quant on the same GPU, so it's offered alongside Vulkan, not in place of it. CI ships this as a separate `release-windows-cuda` job in `release.yml`, gated the same manual/tag-only way as everything else in that workflow. **`--jobs 1` in `scripts/build_platform.sh` is scoped to `CARGO_FEATURES=vulkan` specifically** (a PDB-race workaround for `vulkan-shaders-gen`, which a CUDA-only build never configures) — don't widen that condition back to "any Windows build" without re-reading why it's scoped, that mistake alone costs ~4h per build. See `docs/DECISIONS.md` ("Windows CUDA ships as an additional, opt-in release asset alongside Vulkan").
- `make dist-linux` / `make dist-windows` stage artifacts under `release/` with the same stable-name convention as `dist-mac` (`EmailOps-linux.AppImage`, `EmailOps-linux.deb`, `EmailOps-windows.msi`, `EmailOps-windows-setup.exe`), so the GitHub release always has a permanent latest-download URL. `dist_platform.sh` takes an optional variant suffix (`bash scripts/dist_platform.sh windows cuda` → `EmailOps-windows-cuda.msi`) for a platform that ships more than one installer of the same type.
- Neither platform is code-signed: Linux packages conventionally aren't, and Windows would need an EV/OV certificate the project doesn't hold — unsigned installers show a SmartScreen warning on first run.
- **The `DYNAMIC_BACKENDS=1` path has real, non-obvious packaging gotchas** that only surfaced the first time it was ever built outside CI's `ubuntu-latest` runner (whose image happens to ship several build-only prerequisites preinstalled, which hid these gaps): the direct shared-library dependencies of the main binary (`libggml-base`/`libggml`/`libllama`/`libllama-common`, distinct from the dlopen'd backend modules like `libggml-vulkan.so`) must be staged into the bundle and given a Linux rpath separately, and a vendored crate's build script has a stale-symlink idempotency bug that makes a second build fail unless those symlinks are cleaned first. Read the comments in `scripts/build_platform.sh` before touching this path — they explain each workaround inline. `bootstrap-linux` now also checks for `libclang`/`xdg-open`/`libfuse2` (needed by `bindgen` and by `linuxdeploy`'s AppImage bundling respectively) for the same reason.
- This path currently has no per-PR CI coverage — only the manually-triggered release workflow exercises it (the tag-push trigger is disabled). A regression here is caught only at actual release time; be extra careful with changes to `scripts/build_platform.sh` or the `dynamic-backends` Cargo feature.

## Architecture Principles

### Privacy First
- All email data stored locally in SQLite
- OAuth tokens stored in OS keychain (not plain files)
- No telemetry, no cloud sync, no external API calls except to email providers or AI providers (only when user chooses to use remote LLMs)
- **NEVER include real names, email addresses, subjects, or other personal data from the developer's mailbox in git-tracked files** — not in test fixtures, not in code comments, not in regression test cases, not in commit messages, not in PR descriptions. When a real-world failure surfaces a personal-data string, paraphrase it into a synthetic equivalent that preserves the technical shape (length, multibyte boundaries, regex pattern, etc.) but drops the identifying content. Real bench output goes under `src-tauri/reports/` which is gitignored — keep it there.

### Separation of Concerns
- Tauri commands are thin wrappers that delegate to service modules
- Business logic lives in service modules, not in commands
- Frontend handles UI state only, backend handles data state

### Error Handling
- Use `thiserror` for custom error types in Rust
- Propagate errors with `?` operator, handle at command boundary
- Return user-friendly error messages to frontend
- Log detailed errors locally for debugging
- NEVER discard or ignore errors, errors should be always logged and handled. If relevant they should be notified to users.

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
- NEVER commit not push changes automatically. Developer will do
- NEVER include claude or other agent as author or co-author in commits
- After implementing a feature, bug fix, or any moderate change, run the full pre-commit hook suite locally (`npx lefthook run pre-commit` or, if some files are unstaged, the equivalent commands directly: `npx biome check src/`, `npx tsc --noEmit`, `cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --tests -- -D warnings` (must match CI's exact flags — see `lefthook.yml`'s clippy comment), `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`, plus the `no-invoke-outside-api` grep check). Fix every reported issue before handing the change back to the developer — do not rely on the developer to discover lint/type/format failures.
- For changes that affect Rust behavior, also run `cargo test --manifest-path src-tauri/Cargo.toml` (matches the pre-push hook) before declaring the work done.

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
- **Use the trait seams.** When a test would require mocking `reqwest`, `rusqlite`, the OS clock, the keychain, or `AppHandle`, route through the existing trait fakes (`MailProvider`, `AiProvider`, `Clock`, `Keychain`, `Logger`) instead of reaching for heavy mocks. If a seam is missing, add it as the first step.
- **Pure planners first.** Extract pure decision functions and unit-test them exhaustively before wiring the thin executor. Integration tests cover the executor against fakes.
- **Prefer `test-driven-development` skill.** When implementing any logic, fixing any bug, or changing any behavior, invoke the `test-driven-development` skill to drive the work.

Language-specific testing guidance: `src-tauri/CLAUDE.md` (Rust) and `src/CLAUDE.md` (TypeScript).

## Security Checklist

- [ ] OAuth tokens stored in OS keychain via `keyring` crate
- [ ] No secrets in code or config files
- [ ] Validate all input from frontend before processing
- [ ] Sanitize email content before rendering (XSS prevention)
- [ ] Use HTTPS for all external API calls
- [ ] No logging of sensitive data (tokens, email content)

## Performance Guidelines

- Paginate email lists (50 items per page)
- Use virtual scrolling for long lists
- Batch AI operations (triage multiple emails at once)
- Index frequently queried columns in SQLite
- Cache thread summaries in database
