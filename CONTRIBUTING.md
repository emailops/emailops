# Contributing to EmailOps

Thanks for your interest in contributing! This guide covers the development setup and conventions.

## Where to start

New contributors should read these in order:

1. **[README.md](README.md)** — what EmailOps does, the privacy model, and how to run the app.
2. **This file (CONTRIBUTING.md)** — dev setup, coding conventions, PR workflow.
3. **[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)** — community expectations.
4. **[SECURITY.md](SECURITY.md)** — security model and how to report vulnerabilities.
5. **[ROADMAP.md](ROADMAP.md)** — what's planned next and where help is welcome.

> **Note on `CLAUDE.md`:** these are dense reference guides for **AI coding agents**
> (Claude Code, Cursor, etc.) working on the repo — they exhaustively document architectural
> patterns, lessons learned, and per-module conventions. They are *not* the human contributor
> onboarding entry point. Browse them only if you're curious about the AI-agent workflow or
> looking for deep-dive context on a specific subsystem.

A good first PR is usually a bug fix, a doc improvement, or a test that closes a gap — see
issues tagged `good first issue` if any exist, or open an issue describing what you want to
work on before starting a larger change.

## Development Setup

### Prerequisites

- Node.js (LTS) + npm
- Rust toolchain (stable) + Cargo
- [Tauri system dependencies](https://tauri.app/start/prerequisites/)
- CMake + a C++ toolchain (for the embedded `llama-cpp-2` build)
- [Ollama](https://ollama.com/) — **optional**, only needed if you want to test the Ollama provider path; the default AI provider is the embedded llama.cpp runtime

### Getting Started

```bash
# Clone and install (installs npm deps + git hooks via lefthook)
git clone https://github.com/emailops/emailops.git
cd emailops
make install
# Or, without make:
#   npm install
#   npx lefthook install

# Set up OAuth credentials
cp .env.example .env.local
cp src-tauri/.env.example src-tauri/.env.local
# Edit both .env.local files with your Gmail OAuth credentials

# Run the app
npm run tauri dev
```

### Git hooks

`make install` (or `npx lefthook install`) installs [lefthook](https://github.com/evilmartians/lefthook) hooks that run on `pre-commit` (biome, tsc, clippy, fmt, package-lock sync check) and `pre-push` (`cargo test`). If you skipped them and want to add them later, run `make hooks`.

### Checking Your Changes

Run the same aggregate check used by this repo's local workflow:

```bash
make check
```

`make check` runs lint + typecheck + clippy + Rust tests + frontend tests.

## Project Structure

```
emailops/
├── src-tauri/src/           # Rust backend
│   ├── commands/            # Tauri command handlers (thin layer)
│   ├── services/            # Business logic
│   ├── db/                  # Database operations
│   ├── ai/                  # AI provider abstraction (llama.cpp, Ollama, OpenRouter)
│   ├── sync/                # Email provider sync (Gmail, Outlook/Graph, IMAP)
│   └── models/              # Data structures
├── src/                     # React frontend
│   ├── components/          # React components
│   ├── stores/              # Zustand state stores
│   ├── hooks/               # Custom React hooks
│   ├── lib/                 # API bindings, utilities
│   └── types/               # TypeScript types
└── src-tauri/examples/      # AI eval harnesses (Rust, run via `cargo run --features eval --example ...`)
```

## Coding Conventions

### Rust
- Commands are thin wrappers — business logic lives in `services/`
- Use `thiserror` for error types, propagate with `?`
- Use parameterized SQL queries, never string interpolation
- Naming: `snake_case` for modules/functions, `PascalCase` for types

### TypeScript/React
- Components: `PascalCase` files, function components
- Hooks: `camelCase` with `use` prefix
- State: Zustand stores, keep them focused and small
- Tauri calls: centralized in `src/lib/api.ts`

### Commits
- Format: `type: short description`
- Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`
- Keep under 72 characters

### Pull Requests
- One feature or fix per PR
- Include a description of what and why
- Ensure `cargo check` and `npx tsc --noEmit` pass

## Eval Harnesses

AI evaluation harnesses live in `src-tauri/examples/*_eval.rs` (declared as
Cargo `[[example]]` entries, not `[[bin]]`, so the tauri-bundler does not try
to copy them into the packaged .app). All require the `eval` feature:

```bash
cargo run --features eval --example chat_eval
cargo run --features eval --example lens_extract_eval
cargo run --features eval --example draft_eval -- --n 5
```

Eval result files (JSON, HTML reports under `src-tauri/reports/`) are gitignored
— they may contain real email data.

## Releasing

### Cutting a release

Releases are orchestrated by [release Claude skill](.claude/skills/release/SKILL.md).

Invoke it from Claude Code with `/release <patch|minor|major|X.Y.Z>`. It walks
the full pipeline: bumps the version across `package.json`,
`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and `Cargo.lock`; updates
`CHANGELOG.md`; runs `make check`; builds the signed + notarized universal DMG
via `make build-mac && make verify-mac`; then commits and tags. Pushing and the
GitHub release are gated behind explicit confirmation, and the skill always
stops to ask when anything is ambiguous.