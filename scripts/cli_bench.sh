#!/usr/bin/env bash
# Multi-turn chat prefill/latency bench against the demo DB.
#
# Measures the numbers the KV-cache work optimises: per-LLM-call latency,
# prompt tokens, prefill ms, and cached prompt tokens. Runs four questions as
# consecutive turns in ONE process + ONE conversation (model stays loaded,
# history grows turn over turn — the prefix-caching scenario), with
# `--json --trace` so each turn's stats land under data.turns[*].trace.llmCalls.
# The envelope is tee'd to src-tauri/reports/bench/ for before/after diffing.
#
# Invoked by `make cli-bench` from the repo root. Override any question via
# env: BENCH_Q2="..." make cli-bench
set -euo pipefail

cd "$(dirname "$0")/.."

DEMO_DIR="${EMAILOPS_DEMO_DIR:-$PWD/.emailops-demo-data}"
# The generated demo DB has two enabled accounts; the work one holds the
# invoice/project mail the default questions target.
ACCOUNT="${BENCH_ACCOUNT:-ulises@emailopslabs.dev}"
Q1="${BENCH_Q1:-What invoices have I received recently?}"
Q2="${BENCH_Q2:-Who sent the most recent one and what is it about?}"
Q3="${BENCH_Q3:-Summarize what my unread emails are about}"
Q4="${BENCH_Q4:-Of those, which one looks the most urgent and why?}"

if [ ! -f "$DEMO_DIR/emailops.db" ]; then
  echo "[cli-bench] no demo DB found — building one"
  make demo-db
fi
if ! sqlite3 "$DEMO_DIR/emailops.db" "SELECT 1 FROM embedding_chunks LIMIT 1;" 2>/dev/null | grep -q 1; then
  echo "[cli-bench] no embeddings found — generating (needed for chat)"
  make demo-embed
fi

mkdir -p src-tauri/reports/bench
out="src-tauri/reports/bench/chat_bench_$(date +%Y%m%d_%H%M%S).json"
echo "[cli-bench] writing $out"

EMAILOPS_DATA_DIR="$DEMO_DIR" cargo run --manifest-path src-tauri/Cargo.toml \
  --features cli --bin emailops-cli -- \
  --json --account "$ACCOUNT" chat --trace "$Q1" "$Q2" "$Q3" "$Q4" | tee "$out"
