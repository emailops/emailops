.PHONY: dev dev-fresh dev-trace demo demo-db demo-embed demo-es demo-db-es demo-embed-es check lint fmt test test-fast lint-fast check-fast clippy-fast cli cli-run cli-fast cli-demo cli-eval cli-bench build clean install hooks eval-index eval-all bootstrap-mac build-mac verify-mac dist-mac build-mac-intel verify-mac-intel dist-mac-intel build-cli-mac verify-cli-mac dist-cli-mac fetch-bundled-models record-cassette list-cassette-accounts

# ── Bundled AI models ────────────────────────────────────────────────────────
# The Nomic embedding model ships inside the .app so first-run users don't
# have to download anything just to enable search. This target fetches it
# once into src-tauri/resources/models/ and SHA-256 verifies it. Wired as a
# prerequisite for dev / dev-fresh / build-mac so the file is always present
# when Tauri packages the bundle. The .gguf is gitignored; the canonical URL
# and SHA live in scripts/fetch_bundled_models.sh.
fetch-bundled-models:
	bash scripts/fetch_bundled_models.sh


# Load .env.local if it exists (Apple signing credentials, etc.)
-include .env.local
export

# Repo-local app data directory used by Makefile workflows.
# Override privately in `.env.local` or on the command line when you need a
# different data directory.
EMAILOPS_DATA_DIR ?= $(CURDIR)/.emailops-data

# Development (targets EMAILOPS_DATA_DIR; see src/lib.rs)
dev: fetch-bundled-models
	@echo "[dev] using app data dir: $(EMAILOPS_DATA_DIR)"
	EMAILOPS_DATA_DIR="$(EMAILOPS_DATA_DIR)" npm run tauri dev

# Throwaway dev data dir — safe for experimentation that may mutate state
# (agents trying things out, schema migrations, sync from scratch, etc.).
# Each invocation reuses the same ignored repo-local dir so successive runs share their state;
# delete it manually (`rm -rf "$EMAILOPS_DEV_FRESH_DIR"`) for a clean slate.
EMAILOPS_DEV_FRESH_DIR ?= $(CURDIR)/.emailops-data-fresh
dev-fresh: fetch-bundled-models
	@mkdir -p "$(EMAILOPS_DEV_FRESH_DIR)"
	@echo "[dev-fresh] using throwaway data dir: $(EMAILOPS_DEV_FRESH_DIR)"
	EMAILOPS_DATA_DIR="$(EMAILOPS_DEV_FRESH_DIR)" npm run tauri dev

# Development with tracing instrumentation enabled.
# Optionally set PHOENIX_HOST=http://localhost:6006 in .env if you are routing
# traces to a local collector.
dev-trace: fetch-bundled-models
	@echo "[dev-trace] using app data dir: $(EMAILOPS_DATA_DIR)"
	EMAILOPS_DATA_DIR="$(EMAILOPS_DATA_DIR)" npm run tauri dev -- -- --features tracing

# Demo data directory — isolated local data. Safe for screen recordings.
EMAILOPS_DEMO_DIR ?= $(CURDIR)/.emailops-demo-data

# Generate a fresh synthetic demo DB. Idempotent: overwrites any existing demo DB.
demo-db:
	uv run scripts/generate_demo_db.py --demo-db "$(EMAILOPS_DEMO_DIR)/emailops.db"

# Generate embeddings for every email in the demo DB so chat retrieval works.
# Calls the app's own services::embeddings::generate_embeddings via a small
# Rust bin, so demo doc vectors land in exactly the same space the running
# app will produce for query vectors. Idempotent — already-embedded emails
# are skipped by the pipeline's own pending-emails query.
demo-embed:
	cargo run --release --manifest-path src-tauri/Cargo.toml --example embed_demo_db -- \
	  --source-dir "$(EMAILOPS_DATA_DIR)" \
	  --demo-dir "$(EMAILOPS_DEMO_DIR)"

# Launch the app against the demo DB. Auto-builds the DB + embeddings if missing.
demo:
	@scripts/ensure_demo_db.sh "$(EMAILOPS_DEMO_DIR)" demo-db demo-embed
	EMAILOPS_DATA_DIR="$(EMAILOPS_DEMO_DIR)" npm run tauri dev

# ── Spanish-locale demo (same machinery, separate data dir) ──────────────────
EMAILOPS_DEMO_DIR_ES ?= $(CURDIR)/.emailops-demo-data-es

demo-db-es:
	uv run scripts/generate_demo_db.py --lang es --demo-db "$(EMAILOPS_DEMO_DIR_ES)/emailops.db"

demo-embed-es:
	cargo run --release --manifest-path src-tauri/Cargo.toml --example embed_demo_db -- \
	  --source-dir "$(EMAILOPS_DATA_DIR)" \
	  --demo-dir "$(EMAILOPS_DEMO_DIR_ES)"

demo-es:
	@scripts/ensure_demo_db.sh "$(EMAILOPS_DEMO_DIR_ES)" demo-db-es demo-embed-es
	EMAILOPS_DATA_DIR="$(EMAILOPS_DEMO_DIR_ES)" npm run tauri dev

# Full check (lint + typecheck + clippy + tests)
check:
	npm run check

# Lint only (fast)
lint:
	npx biome check src/
	cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings

# Format everything
fmt:
	npx biome check --write src/
	cargo fmt --manifest-path src-tauri/Cargo.toml

# Type check only
typecheck:
	npx tsc --noEmit

# Run tests
test:
	cargo test --manifest-path src-tauri/Cargo.toml

# ── Fast iteration variants ───────────────────────────────────────────────────
# Skip the `llamacpp` default feature (heavy C++/cmake build of llama-cpp-2).
# Use these when you're NOT touching embedded inference code. Cuts cold-cache
# build times by minutes and avoids re-linking the large native artifact on
# every change. The full `test` / `lint` / `check` targets remain authoritative
# for CI and pre-commit hooks.
test-fast:
	cargo test --manifest-path src-tauri/Cargo.toml --no-default-features

clippy-fast:
	cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features -- -D warnings

lint-fast:
	npx biome check src/
	$(MAKE) clippy-fast

check-fast:
	npm run lint && npm run typecheck && $(MAKE) test-fast && $(MAKE) clippy-fast

# ── emailops-cli (power-user / agent CLI + interactive REPL) ─────────────────
# Gated behind the `cli` cargo feature so it never compiles into default/release
# desktop builds. `cli` builds with the embedded llama.cpp provider (needed for
# local chat/classify/embed); `cli-fast` drops it (`--no-default-features`) for
# fast iteration on read/search commands or when using Ollama/OpenRouter.
#
# Usage:
#   make cli                                   # build the bin (with llama.cpp)
#   make cli-run ARGS="accounts --json"        # build + run (with llama.cpp)
#   make cli-fast ARGS="search invoice --json" # build + run (no llama.cpp)
#   make cli-fast                              # no ARGS → interactive REPL
cli:
	cargo build --manifest-path src-tauri/Cargo.toml --features cli --bin emailops-cli

cli-run:
	cargo run --manifest-path src-tauri/Cargo.toml --features cli --bin emailops-cli -- $(ARGS)

cli-fast:
	cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --features cli --bin emailops-cli -- $(ARGS)

# Drive the CLI against the synthetic demo DB (safe for screen recordings / GIFs).
# Auto-builds the demo DB + embeddings if missing, then runs with llama.cpp on so
# chat works fully offline against demo data. No ARGS → interactive REPL.
#   make cli-demo ARGS="search 'invoice' --json"
#   make cli-demo ARGS="chat 'what did Acme say about the contract?' --trace"
#   make cli-demo                                   # interactive REPL on demo data
cli-demo:
	@scripts/ensure_demo_db.sh "$(EMAILOPS_DEMO_DIR)" demo-db demo-embed
	EMAILOPS_DATA_DIR="$(EMAILOPS_DEMO_DIR)" cargo run --manifest-path src-tauri/Cargo.toml --features cli --bin emailops-cli -- $(ARGS)

# Run chat eval cases through the CLI's `eval` subcommand (heuristics only — no
# judge, no HTML report, no provider-pref mutation). Needs `cli,eval`; keeps
# llama.cpp on so local models can answer.
#   make cli-eval ARGS="--tier smoke --json"
#   make cli-eval ARGS="--case kickoff_date_es"
cli-eval:
	cargo run --manifest-path src-tauri/Cargo.toml --features cli,eval --bin emailops-cli -- eval $(ARGS)

# Multi-turn chat prefill/latency bench against the demo DB (model stays loaded
# across turns). Logic lives in scripts/cli_bench.sh; questions overridable via
# env: BENCH_Q2="..." make cli-bench
cli-bench:
	EMAILOPS_DEMO_DIR="$(EMAILOPS_DEMO_DIR)" scripts/cli_bench.sh

# ── Provider HTTP cassettes ──────────────────────────────────────────────────
# Record live Gmail / Microsoft Graph API responses for a connected account
# into a JSON cassette under src-tauri/tests/fixtures/cassettes/<provider>/.
# Cassettes are replayed by `MockProviderServer` in integration tests so the
# real GmailClient / OutlookClient parsing layer gets exercised without a
# live mailbox. The recorder is debug-only (refuses to run in release) and
# requires the explicit SCENARIO argument so it never fires by accident.
#
# Usage:
#   make list-cassette-accounts            # print {id, email, provider}
#   make record-cassette ACCOUNT=alice@hotmail.com SCENARIO=outlook_happy_path
#   make record-cassette ACCOUNT=alice@hotmail.com SCENARIO=outlook_happy_path LIMIT=10 RAW=1
#
# RAW=1 keeps real names/addresses/bodies and writes under cassettes/raw/
# (gitignored). Default sanitises everything and writes under the per-provider
# directory, which IS committed.
CASSETTE_LIMIT ?= 5
list-cassette-accounts:
	cargo run --manifest-path src-tauri/Cargo.toml --example record_provider_cassette -- --list-accounts

record-cassette:
	@if [ -z "$(SCENARIO)" ]; then echo "ERROR: SCENARIO=… is required (e.g. SCENARIO=outlook_happy_path)"; exit 1; fi
	@if [ -z "$(ACCOUNT)" ]; then echo "ERROR: ACCOUNT=<email-or-id> is required (run 'make list-cassette-accounts' for options)"; exit 1; fi
	cargo run --manifest-path src-tauri/Cargo.toml --example record_provider_cassette -- \
	  --record-scenario "$(SCENARIO)" \
	  --account "$(ACCOUNT)" \
	  --limit "$(CASSETTE_LIMIT)" \
	  $(if $(RAW),--raw,) \
	  $(if $(EMAILOPS_DATA_DIR),--data-dir "$(EMAILOPS_DATA_DIR)",)

# Build for production
build:
	npm run build

# Install dependencies + hooks
install:
	npm install
	npx lefthook install --force

# Deploy (build + install to /Applications on macOS)
deploy:
	cargo build --release --features eval --example chat_eval --manifest-path src-tauri/Cargo.toml
	npm run tauri build
	cp -r src-tauri/target/release/bundle/macos/EmailOps.app /Applications/

# Security audit
audit:
	cargo audit --file src-tauri/Cargo.lock

# Clean build artifacts
clean:
	rm -rf dist/
	cargo clean --manifest-path src-tauri/Cargo.toml

# Install git hooks
hooks:
	npx lefthook install --force

# ── macOS release: signed + notarized universal DMG ──────────────────────────
#
# One-time setup:
#   make bootstrap-mac
#   cp .env.signing.example .env.signing   # then fill in real values
#
# Then run:
#   make build-mac && make verify-mac
#
# Secrets are read from `.env.signing` (gitignored). That file is sourced
# directly by this target — NOT by Tauri — because Tauri's dotenv loader
# auto-loads `.env.local` with different quoting rules than bash and silently
# mangles passwords containing $, *, !, \, or backticks. See
# `.env.signing.example` for the required variables.
bootstrap-mac:
	rustup target add aarch64-apple-darwin x86_64-apple-darwin

build-mac: fetch-bundled-models
	@if [ ! -f .env.signing ]; then \
		echo "ERROR: .env.signing not found. Copy .env.signing.example to .env.signing and fill in your Apple signing secrets."; \
		exit 1; \
	fi
	@set -a; . ./.env.signing; set +a; \
	if [ -z "$$APPLE_SIGNING_IDENTITY" ]; then echo "ERROR: APPLE_SIGNING_IDENTITY not set in .env.signing"; exit 1; fi; \
	if [ -z "$$APPLE_ID" ]; then echo "ERROR: APPLE_ID not set in .env.signing"; exit 1; fi; \
	if [ -z "$$APPLE_PASSWORD" ]; then echo "ERROR: APPLE_PASSWORD not set in .env.signing (must be an app-specific password)"; exit 1; fi; \
	if [ -z "$$APPLE_TEAM_ID" ]; then echo "ERROR: APPLE_TEAM_ID not set in .env.signing"; exit 1; fi; \
	if [ -z "$$APPLE_CERTIFICATE" ]; then echo "ERROR: APPLE_CERTIFICATE not set in .env.signing"; exit 1; fi; \
	if [ -z "$$APPLE_CERTIFICATE_PASSWORD" ]; then echo "ERROR: APPLE_CERTIFICATE_PASSWORD not set in .env.signing"; exit 1; fi; \
	npm run tauri -- build --target universal-apple-darwin

verify-mac:
	@APP=$$(ls -d src-tauri/target/universal-apple-darwin/release/bundle/macos/*.app 2>/dev/null | head -1); \
	if [ -z "$$APP" ]; then echo "ERROR: no .app found. Run 'make build-mac' first."; exit 1; fi; \
	echo "Verifying $$APP"; \
	echo "── codesign ──"; codesign -dv --verbose=4 "$$APP" 2>&1 | sed 's/^/  /'; \
	echo "── architectures ──"; file "$$APP/Contents/MacOS/"* 2>&1 | sed 's/^/  /'; \
	echo "── spctl ──"; spctl -a -t exec -vv "$$APP" 2>&1 | sed 's/^/  /'; \
	echo "── stapler ──"; xcrun stapler validate "$$APP" 2>&1 | sed 's/^/  /'

# Copy the freshly built universal DMG to a stable, versionless name under
# release/. Tauri always embeds the version in the bundle filename
# (EmailOps_<version>_universal.dmg), which breaks permanent download links.
# Upload release/EmailOps-macos.dmg to the GitHub Release so it is reachable at
#   https://github.com/emailops/emailops/releases/latest/download/EmailOps-macos.dmg
# Run after `make build-mac && make verify-mac`.
dist-mac:
	@DMG=$$(ls -t src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg 2>/dev/null | head -1); \
	if [ -z "$$DMG" ]; then echo "ERROR: no universal .dmg found. Run 'make build-mac' first."; exit 1; fi; \
	mkdir -p release; \
	cp "$$DMG" release/EmailOps-macos.dmg; \
	echo "Staged release/EmailOps-macos.dmg (from $$(basename "$$DMG"))"

# Intel-only signed + notarized DMG. Same flow as `build-mac` but targets
# `x86_64-apple-darwin` only — and ships WITHOUT the embedded llama.cpp AI
# provider, because (a) Apple-Silicon-only `metal` acceleration means
# CPU-only inference on Intel is too slow to be a real product experience,
# and (b) the bundled embedding GGUF (~80 MB) would just bloat the .app
# without ever being usable. Intel users wanting AI can still configure
# Ollama or OpenRouter from Settings.
#
# Two knobs do the trimming:
#   --no-default-features → drops the `llamacpp` feature so llama-cpp-2
#                           isn't compiled into the binary at all.
#   --config tauri.intel.conf.json → overlays bundle.resources to []
#                           so the GGUF is excluded from the .app.
# `fetch-bundled-models` is deliberately NOT a prerequisite — there's no
# point downloading the GGUF for a build that won't bundle it. Apple
# Silicon users should still get the universal build from `build-mac`.
# Requires the x86_64-apple-darwin Rust target, which `bootstrap-mac`
# already installs. `verify-mac-intel` asserts the GGUF is absent so a
# config-merge regression is caught at verify time.
build-mac-intel:
	@if [ ! -f .env.signing ]; then \
		echo "ERROR: .env.signing not found. Copy .env.signing.example to .env.signing and fill in your Apple signing secrets."; \
		exit 1; \
	fi
	@set -a; . ./.env.signing; set +a; \
	if [ -z "$$APPLE_SIGNING_IDENTITY" ]; then echo "ERROR: APPLE_SIGNING_IDENTITY not set in .env.signing"; exit 1; fi; \
	if [ -z "$$APPLE_ID" ]; then echo "ERROR: APPLE_ID not set in .env.signing"; exit 1; fi; \
	if [ -z "$$APPLE_PASSWORD" ]; then echo "ERROR: APPLE_PASSWORD not set in .env.signing (must be an app-specific password)"; exit 1; fi; \
	if [ -z "$$APPLE_TEAM_ID" ]; then echo "ERROR: APPLE_TEAM_ID not set in .env.signing"; exit 1; fi; \
	if [ -z "$$APPLE_CERTIFICATE" ]; then echo "ERROR: APPLE_CERTIFICATE not set in .env.signing"; exit 1; fi; \
	if [ -z "$$APPLE_CERTIFICATE_PASSWORD" ]; then echo "ERROR: APPLE_CERTIFICATE_PASSWORD not set in .env.signing"; exit 1; fi; \
	npm run tauri -- build --target x86_64-apple-darwin --config src-tauri/tauri.intel.conf.json -- --no-default-features

verify-mac-intel:
	@APP=$$(ls -d src-tauri/target/x86_64-apple-darwin/release/bundle/macos/*.app 2>/dev/null | head -1); \
	if [ -z "$$APP" ]; then echo "ERROR: no .app found. Run 'make build-mac-intel' first."; exit 1; fi; \
	echo "Verifying $$APP"; \
	echo "── codesign ──"; codesign -dv --verbose=4 "$$APP" 2>&1 | sed 's/^/  /'; \
	echo "── architectures ──"; file "$$APP/Contents/MacOS/"* 2>&1 | sed 's/^/  /'; \
	echo "── spctl ──"; spctl -a -t exec -vv "$$APP" 2>&1 | sed 's/^/  /'; \
	echo "── stapler ──"; xcrun stapler validate "$$APP" 2>&1 | sed 's/^/  /'; \
	echo "── no-bundled-llm guard ──"; \
	LEAKED=$$(find "$$APP" -name "*.gguf" 2>/dev/null); \
	if [ -n "$$LEAKED" ]; then \
		echo "  ❌ FAIL: GGUF files leaked into the Intel bundle:"; echo "$$LEAKED" | sed 's/^/    /'; \
		echo "  This means tauri.intel.conf.json's bundle.resources override did NOT clear the base config's GGUF entries."; \
		echo "  Re-check the --config flag and Tauri's config-merge behaviour."; \
		exit 1; \
	else \
		echo "  ✅ no .gguf files in the bundle (Intel build is AI-disabled by policy)"; \
	fi

# Copy the freshly built Intel DMG to a stable, versionless name under release/.
# Upload release/EmailOps-macos-intel.dmg to the GitHub Release so it is reachable at
#   https://github.com/emailops/emailops/releases/latest/download/EmailOps-macos-intel.dmg
# Run after `make build-mac-intel && make verify-mac-intel`.
dist-mac-intel:
	@DMG=$$(ls -t src-tauri/target/x86_64-apple-darwin/release/bundle/dmg/*.dmg 2>/dev/null | head -1); \
	if [ -z "$$DMG" ]; then echo "ERROR: no Intel .dmg found. Run 'make build-mac-intel' first."; exit 1; fi; \
	mkdir -p release; \
	cp "$$DMG" release/EmailOps-macos-intel.dmg; \
	echo "Staged release/EmailOps-macos-intel.dmg (from $$(basename "$$DMG"))"

# ── emailops-cli release binary (universal, signed) ──────────────────────────
# Build a standalone `emailops-cli` to distribute alongside the desktop app so
# power users can drive EmailOps from the terminal. Same arches as `build-mac`
# (aarch64 + x86_64, llama.cpp on both → offline `chat` works), lipo'd into one
# universal binary. The heavy cross-compile + signing logic lives in
# scripts/build_cli_release.sh; signing only happens when .env.signing provides
# APPLE_SIGNING_IDENTITY (else the binary is left unsigned for local use).
# Requires the x86_64/aarch64 Rust targets that `bootstrap-mac` installs.
#
# Recommended follow-up: bundle this as a Tauri `externalBin` sidecar inside the
# .app so the app's own notarization covers it and an in-app "install CLI to
# PATH" action can symlink it — see docs/cli-user-guide.md.
#
#   make build-cli-mac      # build (+sign) → src-tauri/target/cli-release/emailops-cli
#   make verify-cli-mac     # assert universal + signed
#   make dist-cli-mac       # stage → release/emailops-cli (versionless)
build-cli-mac:
	@if [ -f .env.signing ]; then set -a; . ./.env.signing; set +a; fi; \
	  bash scripts/build_cli_release.sh

verify-cli-mac:
	@BIN=src-tauri/target/cli-release/emailops-cli; \
	if [ ! -x "$$BIN" ]; then echo "ERROR: $$BIN not found. Run 'make build-cli-mac' first."; exit 1; fi; \
	echo "Verifying $$BIN"; \
	echo "── architectures ──"; lipo -info "$$BIN" 2>&1 | sed 's/^/  /'; \
	echo "── codesign ──"; codesign -dv --verbose=4 "$$BIN" 2>&1 | sed 's/^/  /'

# Copy the freshly built universal CLI to a stable, versionless name under
# release/ so it can be uploaded to the GitHub Release. Run after
# `make build-cli-mac && make verify-cli-mac`.
dist-cli-mac:
	@BIN=src-tauri/target/cli-release/emailops-cli; \
	if [ ! -x "$$BIN" ]; then echo "ERROR: $$BIN not found. Run 'make build-cli-mac' first."; exit 1; fi; \
	mkdir -p release; \
	cp "$$BIN" release/emailops-cli; \
	echo "Staged release/emailops-cli (from $$BIN)"

# Regenerate the evaluations index (manifest.json + instructions to open index.html)
eval-index:
	python3 src-tauri/reports/evaluations/generate_manifest.py
	@echo "Open: src-tauri/reports/evaluations/index.html (via HTTP server)"
	@echo "Quick server: cd src-tauri/reports/evaluations && python3 -m http.server 8765"

# Run every eval suite against a single embedded llama.cpp model. The model id
# is passed as MODEL=… (catalog id, e.g. `qwen3.5-4b-q4_k_m`, `qwen3.5-9b-q4_k_m`).
# The override is propagated through EMAILOPS_EVAL_MODEL, which the shared eval
# runner reads after opening the snapshot DB. The shell scripts keep the command
# details out of this Makefile.
#
# Usage:
#   make eval-snapshot                                   # one-time per session
#   make eval-all MODEL=qwen3.5-4b-q4_k_m
#   make eval-all MODEL=qwen3.5-9b-q4_k_m
#
# Override evals on the command line with EVAL_LIMIT / EVAL_DRAFT_N.
PROVIDER ?= llamacpp
ACCOUNT ?= you@example.com
EVAL_LIMIT ?= 30
EVAL_DRAFT_N ?= 10
EVAL_SNAPSHOT_DIR ?= $(TMPDIR)eval-snapshot
EVAL_SNAPSHOT_DB := $(EVAL_SNAPSHOT_DIR)/emailops.db

.PHONY: eval-snapshot
eval-snapshot:
	EVAL_SNAPSHOT_DIR="$(EVAL_SNAPSHOT_DIR)" \
	  EVAL_SNAPSHOT_DB="$(EVAL_SNAPSHOT_DB)" \
	  bash scripts/eval_snapshot.sh

eval-all:
	@if [ -z "$(MODEL)" ]; then \
		echo "ERROR: MODEL is required. Example: make eval-all MODEL=qwen3.5-4b-q4_k_m"; \
		exit 1; \
	fi
	MODEL="$(MODEL)" \
	  PROVIDER="$(PROVIDER)" \
	  ACCOUNT="$(ACCOUNT)" \
	  EVAL_LIMIT="$(EVAL_LIMIT)" \
	  EVAL_DRAFT_N="$(EVAL_DRAFT_N)" \
	  EVAL_SNAPSHOT_DB="$(EVAL_SNAPSHOT_DB)" \
	  bash scripts/eval_all.sh
