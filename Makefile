.PHONY: dev dev-fresh dev-trace demo demo-db demo-embed demo-es demo-db-es demo-embed-es check lint fmt test test-fast lint-fast check-fast clippy-fast cli cli-run cli-fast install-cli cli-demo cli-eval cli-bench build clean install hooks eval-index eval-all eval-junk bootstrap-mac build-mac verify-mac dist-mac build-cli-mac verify-cli-mac dist-cli-mac cask fetch-bundled-models record-cassette list-cassette-accounts bootstrap-ios ios-init ios-dev ios-build bootstrap-linux build-linux verify-linux dist-linux bootstrap-windows build-windows verify-windows dist-windows testvm-status testvm-linux testvm-windows testvm-start testvm-stop testvm-destroy

# ── Shell requirements ───────────────────────────────────────────────────────
# Every recipe here assumes GNU make plus a POSIX shell: targets use `VAR=x cmd`
# env prefixes, `$(...)` substitution, and call scripts/*.sh. On Windows, run
# these targets from Git Bash or MSYS2 — cmd.exe and PowerShell cannot execute
# them. Linux and macOS work out of the box.

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
	cargo clippy --manifest-path src-tauri/Cargo.toml --tests -- -D warnings

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
	cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --tests -- -D warnings

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

# Build an optimized (unsigned) emailops-cli and symlink it onto your PATH for
# local use. Override the dir with PREFIX=~/.local/bin. Logic in scripts/.
install-cli:
	bash scripts/install_cli.sh

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
# llama.cpp on so local models can answer. Auto-builds the demo DB + embeddings
# if missing and points `EMAILOPS_DATA_DIR` at the demo dir so case `account:`
# overrides like `alex@northwindlabs.io` resolve.
#
# Pinned to `--cases-dir src-tauri/evals/chat/cases` (the PUBLIC cases) so the
# demo-DB run never tries to use `private-evals/chat/cases/`, whose cases
# target the developer's real mailbox account (`$(EMAILOPS_PERSONAL_ACCOUNT)`,
# set in the gitignored `.env.local`) which doesn't exist in the demo DB. The
# CLI's auto-resolver prefers `private-evals/` when present (see
# `cli/eval.rs:resolve_cases_dir`), which would otherwise blow up with
# "account '<your-address>' not found in DB" the moment the user runs
# `make cli-eval` with private cases on disk.
#   make cli-eval ARGS="--tier smoke --json"
#   make cli-eval ARGS="--case kickoff_date_es"
cli-eval:
	@scripts/ensure_demo_db.sh "$(EMAILOPS_DEMO_DIR)" demo-db demo-embed
	EMAILOPS_DATA_DIR="$(EMAILOPS_DEMO_DIR)" cargo run --manifest-path src-tauri/Cargo.toml --features cli,eval --bin emailops-cli -- eval --cases-dir src-tauri/evals/chat/cases $(ARGS)

# Multi-turn chat prefill/latency bench against the demo DB (model stays loaded
# across turns). Logic lives in scripts/cli_bench.sh; questions overridable via
# env: BENCH_Q2="..." make cli-bench
cli-bench:
	EMAILOPS_DEMO_DIR="$(EMAILOPS_DEMO_DIR)" scripts/cli_bench.sh

# Cross-conversation KV-cache reuse probe: two DIFFERENT questions, each in its
# own fresh conversation, one process. Shows whether chat 2's first LLM round
# reuses the system prefix resident from chat 1. Logic in scripts/cli_kv_xconv.sh.
#   make cli-kv-xconv
cli-kv-xconv:
	EMAILOPS_DEMO_DIR="$(EMAILOPS_DEMO_DIR)" scripts/cli_kv_xconv.sh

# Same bench against the user's REAL data + account + question, sourced from
# .env.local (gitignored). Used to reproduce KV-cache bugs that only show up
# on real mailboxes / specific prompts. See .env.local.example for the env
# vars and scripts/cli_kv_personal.sh for the logic.
#   make cli-kv-personal
cli-kv-personal:
	scripts/cli_kv_personal.sh

# Ad-hoc chat query against the user's real mailbox. Inherits EMAILOPS_DATA_DIR
# and EMAILOPS_PERSONAL_ACCOUNT from .env.local (gitignored) so nothing
# sensitive lands in a tracked file; the question itself stays on the
# developer's terminal (and in shell history) — never committed.
#   make cli-ask Q='cuáles son los últimos correos de <persona>?'
cli-ask:
	@[ -n "$(Q)" ] || (echo "Usage: make cli-ask Q='your question'" >&2; exit 1)
	@[ -n "$(EMAILOPS_PERSONAL_ACCOUNT)" ] || (echo "Set EMAILOPS_PERSONAL_ACCOUNT in .env.local (see .env.local.example)" >&2; exit 1)
	EMAILOPS_DATA_DIR="$(EMAILOPS_DATA_DIR)" cargo run --manifest-path src-tauri/Cargo.toml \
	  --features cli --bin emailops-cli -- --account "$(EMAILOPS_PERSONAL_ACCOUNT)" \
	  chat --fresh --trace "$(Q)"

# Probe the "planner" idea: turn ONE query into a search_emails filter via the
# query_plan_probe example (one model completion, no chat tool-loop), printing
# the filter + latency. An exploratory diagnostic — see examples/query_plan_probe.rs.
# Defaults to the REAL DB (the example resolves the prod DB + the single enabled
# account / ai_model pref). Pass a query with Q=...; override the model with
# MODEL=...; pass extra flags (--account, --prod-db) via ARGS. No Q → synthetic set.
#   make plan-probe Q='last 3 emails I sent to alex'
#   make plan-probe Q='emails I sent' MODEL=qwen3.5-9b-q4_k_m
#   make plan-probe ARGS='--prod-db .emailops-demo-data/emailops.db --account ulises@emailopslabs.dev'
plan-probe:
	cargo run --manifest-path src-tauri/Cargo.toml --features eval --example query_plan_probe -- \
	  $(if $(MODEL),--model $(MODEL),) $(ARGS) $(if $(Q),"$(Q)",)

# Open the single-file KV-cache visualizer in the default browser.
# Drop a kv_xconv_*.json onto the page, paste actor logs, or pick a canned
# example. See tools/kv_viz/README.md for what each panel shows.
kv-viz:
	open tools/kv_viz/index.html

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
	npm run tauri build -- --bundles app
	bash scripts/install_to_applications.sh src-tauri/target/release/bundle/macos/EmailOps.app

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
# `build-mac` is universal (arm64 + x86_64) on purpose: ONE macOS download that
# launches on every Mac, so neither the website nor the user has to figure out
# which chip they have.
#
# The catch to know about: cargo features are per-build, not per-slice, so the
# x86_64 slice necessarily carries the `llamacpp` default feature WITH Metal
# (llama-cpp-sys-2 disables Metal only for watchOS, not for Intel). An Intel Mac
# has no Apple7-family GPU to run those kernels on, and every AI turn dies with
# `Decode Error -3: unknown`. A build flag cannot express "this slice only", so
# the exclusion is enforced at RUNTIME instead:
# `ai::gpu_plan::embedded_runtime_supported` gates the capability probe, the
# provider loader, model auto-select and both provider pickers.
#
# That runtime gate is load-bearing — it is the only thing standing between an
# Intel Mac and a guaranteed inference failure. Do not weaken it on the
# assumption that the bundle "shouldn't" reach Intel; this bundle is built to.
#
# The cost accepted here is ~100 MB of llama.cpp code and bundled embedding GGUF
# that an Intel Mac downloads and can never use. That was judged cheaper than
# maintaining two release pipelines and an arch-detecting download page.
bootstrap-mac:
	rustup target add aarch64-apple-darwin x86_64-apple-darwin

# ── iOS ─────────────────────────────────────────────────────────────────────
#
# One-time setup:
#   make bootstrap-ios      # rust targets; reports anything else that's missing
#   make ios-init           # generates src-tauri/gen/apple/ (Xcode project)
#
# Then:
#   make ios-dev            # run on a booted simulator or attached device
#   make ios-build          # release IPA
#
# All three go through scripts/ios.sh, which puts CocoaPods on PATH and strips
# RVM's gem environment — see the comments in that script for why both are
# required. Deployment target is pinned to iOS 26 in tauri.ios.conf.json.
bootstrap-ios:
	rustup target add aarch64-apple-ios aarch64-apple-ios-sim
	@command -v xcodebuild >/dev/null 2>&1 || echo "MISSING: full Xcode (not just Command Line Tools) — install from the App Store, then: sudo xcode-select -s /Applications/Xcode.app"
	@command -v pod >/dev/null 2>&1 || /opt/homebrew/bin/brew list cocoapods >/dev/null 2>&1 || echo "MISSING: cocoapods — install with: /opt/homebrew/bin/brew install cocoapods"

ios-init:
	bash scripts/ios.sh init

ios-dev:
	bash scripts/ios.sh dev

ios-build:
	bash scripts/ios.sh build

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
	echo "── stapler ──"; xcrun stapler validate "$$APP" 2>&1 | sed 's/^/  /'; \
	echo "── universal-slice guard ──"; \
	SLICES=$$(file "$$APP/Contents/MacOS/"* 2>/dev/null); \
	MISSING=""; \
	echo "$$SLICES" | grep -q "arm64"  || MISSING="$$MISSING arm64"; \
	echo "$$SLICES" | grep -q "x86_64" || MISSING="$$MISSING x86_64"; \
	if [ -n "$$MISSING" ]; then \
		echo "  ❌ FAIL: this is not a universal bundle — missing:$$MISSING"; \
		echo "  One macOS DMG has to launch on every Mac; a dropped slice silently strands"; \
		echo "  that half of users on a download that will not open."; \
		exit 1; \
	else \
		echo "  ✅ universal (arm64 + x86_64); embedded AI is gated off Intel at runtime"; \
	fi

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

# ── Linux release: .deb + .AppImage ──────────────────────────────────────────
#
#   make bootstrap-linux    # rust target + report missing system packages
#   make build-linux        # unsigned deb + AppImage
#   make verify-linux       # artifacts, architecture, unresolved .so check
#   make dist-linux         # stage into release/ with stable names
#
# Embedded llama.cpp is ON (the default feature), so cmake and a C++ toolchain
# are required — `bootstrap-linux` checks for both.
#
# GPU builds. Prefer DYNAMIC_BACKENDS=1: it ships each backend as a loadable
# module so ONE artifact runs on a GPU when a driver is present and falls back
# to CPU when it is not. Without it, a GPU-linked binary will not start at all
# on a machine lacking the runtime, which means a separate download per backend.
#   make build-linux DYNAMIC_BACKENDS=1 CARGO_FEATURES=vulkan  # AMD/Intel/NVIDIA
#   make build-linux DYNAMIC_BACKENDS=1 CARGO_FEATURES=cuda    # NVIDIA only, faster
# Vulkan is the better default for a single artifact: it covers all three
# vendors and needs only the user's normal graphics driver at runtime, whereas
# CUDA needs the NVIDIA toolkit at build time and NVIDIA hardware at run time.
#
# Unsigned by convention; Linux packages are not code-signed.
bootstrap-linux:
	bash scripts/bootstrap_platform.sh linux

build-linux:
	CARGO_FEATURES="$(CARGO_FEATURES)" DYNAMIC_BACKENDS="$(DYNAMIC_BACKENDS)" NO_DEFAULT_FEATURES="$(NO_DEFAULT_FEATURES)" bash scripts/build_platform.sh linux

verify-linux:
	bash scripts/verify_platform.sh linux

dist-linux:
	bash scripts/dist_platform.sh linux

# ── Windows release: .msi + NSIS .exe ────────────────────────────────────────
#
#   make bootstrap-windows  # rust target + report missing MSVC/cmake
#   make build-windows      # unsigned msi + nsis installer
#   make verify-windows     # artifacts + architecture
#   make dist-windows       # stage into release/ with stable names
#
# Run from Git Bash or MSYS2 (see "Shell requirements" at the top of this file).
# GPU builds use the same CARGO_FEATURES=cuda|vulkan switch as Linux.
#
# NOT code-signed: the project holds no Windows signing certificate, so
# installers trigger a SmartScreen warning on first run. Adding signing means
# adding a cert + `signtool` step to build_platform.sh.
bootstrap-windows:
	bash scripts/bootstrap_platform.sh windows

build-windows:
	CARGO_FEATURES="$(CARGO_FEATURES)" DYNAMIC_BACKENDS="$(DYNAMIC_BACKENDS)" NO_DEFAULT_FEATURES="$(NO_DEFAULT_FEATURES)" bash scripts/build_platform.sh windows

verify-windows:
	bash scripts/verify_platform.sh windows

dist-windows:
	bash scripts/dist_platform.sh windows

# ── Azure GPU test VM ────────────────────────────────────────────────────────
#
#   make testvm-status     # VMs, snapshots, remaining T4 quota
#   make testvm-linux      # restore the Linux GPU VM from its snapshot
#   make testvm-windows    # create the Windows 11 GPU VM
#   make testvm-stop       # deallocate (stops compute billing)
#   make testvm-destroy    # snapshot the OS disk, then delete VM/disk/NIC
#
# The one box where GPU offload can actually be verified — CI runners have no
# GPU, so their smoke test only ever exercises the CPU fallback path. The
# NCASv3_T4 quota fits exactly ONE of these VMs, so Linux and Windows take
# turns: destroy one before creating the other. `testvm-destroy` snapshots
# first so the swap back is a restore, not an hour of reprovisioning.
#
# Runbook, costs, and gotchas: docs/TEST-VMS.md
testvm-status:
	bash scripts/testvm.sh status

testvm-linux:
	bash scripts/testvm.sh create-linux

testvm-windows:
	bash scripts/testvm.sh create-windows

testvm-start:
	bash scripts/testvm.sh start

testvm-stop:
	bash scripts/testvm.sh stop

testvm-destroy:
	bash scripts/testvm.sh destroy

# ── emailops-cli release binary (universal, signed + notarized) ──────────────
# Build a standalone `emailops-cli` to distribute alongside the desktop app so
# power users can drive EmailOps from the terminal. Same arches as `build-mac`
# (aarch64 + x86_64, llama.cpp on both → offline `chat` works), lipo'd into one
# universal binary. The heavy cross-compile + signing/notarization logic lives
# in scripts/build_cli_release.sh; signing happens when .env.signing provides
# APPLE_SIGNING_IDENTITY, and the binary is wrapped in a notarized + stapled
# .dmg when the full notary credential set (APPLE_ID / APPLE_PASSWORD /
# APPLE_TEAM_ID) is present. The .dmg is the distributable — a bare Mach-O can't
# be stapled, so we staple the container so it verifies offline.
# Requires the x86_64/aarch64 Rust targets that `bootstrap-mac` installs.
#
#   make build-cli-mac      # build → sign → notarize → staple .dmg
#   make verify-cli-mac     # assert universal + signed + stapled
#   make dist-cli-mac       # stage → release/EmailOps-CLI-macos.dmg (versionless)
build-cli-mac:
	@if [ -f .env.signing ]; then set -a; . ./.env.signing; set +a; fi; \
	  bash scripts/build_cli_release.sh

verify-cli-mac:
	@BIN=src-tauri/target/cli-release/emailops-cli; \
	DMG=src-tauri/target/cli-release/EmailOps-CLI.dmg; \
	if [ ! -x "$$BIN" ]; then echo "ERROR: $$BIN not found. Run 'make build-cli-mac' first."; exit 1; fi; \
	echo "Verifying $$BIN"; \
	echo "── architectures ──"; lipo -info "$$BIN" 2>&1 | sed 's/^/  /'; \
	echo "── codesign ──"; codesign -dv --verbose=4 "$$BIN" 2>&1 | sed 's/^/  /'; \
	if [ -f "$$DMG" ]; then \
		echo "Verifying $$DMG"; \
		echo "── stapler ──"; xcrun stapler validate "$$DMG" 2>&1 | sed 's/^/  /'; \
		echo "── spctl ──"; spctl -a -t open --context context:primary-signature -vv "$$DMG" 2>&1 | sed 's/^/  /'; \
	else \
		echo "── .dmg ── not built (notary env was incomplete at build time)"; \
	fi

# Copy the freshly built notarized CLI .dmg to a stable, versionless name under
# release/ so it can be uploaded to the GitHub Release. Run after
# `make build-cli-mac && make verify-cli-mac`.
dist-cli-mac:
	@DMG=src-tauri/target/cli-release/EmailOps-CLI.dmg; \
	if [ ! -f "$$DMG" ]; then echo "ERROR: $$DMG not found. Run 'make build-cli-mac' first (with notary creds in .env.signing)."; exit 1; fi; \
	mkdir -p release; \
	cp "$$DMG" release/EmailOps-CLI-macos.dmg; \
	echo "Staged release/EmailOps-CLI-macos.dmg (from $$DMG)"

# ── Homebrew cask ────────────────────────────────────────────────────────────
# Regenerate homebrew/Casks/emailops.rb from the published GitHub release
# assets (GitHub's own sha256 digests — no download). Run AFTER uploading the
# DMGs to the release, then copy the cask into the emailops/homebrew-tap repo.
# Full publishing flow: homebrew/README.md.
#   make cask              # from the latest release
#   make cask TAG=v0.6.2   # from a specific tag
cask:
	bash scripts/generate_cask.sh $(TAG)

# Junk detector gate (spam / phishing / graymail). Synthetic corpus, no model,
# no DB — runs in seconds. Exits non-zero when the false-positive budget is
# blown. A missed spam message is a warning; a false positive on real mail is a
# build failure.
#   make eval-junk
#   make eval-junk ARGS="--case phish-bec-lookalike-domain-reply-to-mismatch"
eval-junk:
	bash scripts/eval_junk.sh $(ARGS)

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
