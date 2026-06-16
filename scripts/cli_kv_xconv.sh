#!/usr/bin/env bash
# Cross-conversation KV-cache reuse probe against the demo DB.
#
# The within-conversation bench (cli_bench.sh) shows round0->round1 reuse. This
# script targets the OTHER case: the FIRST LLM call (round 0) of a brand-new
# conversation reusing the *system prefix* still resident in the KV cache from
# the previous conversation. To isolate that, it runs two DIFFERENT questions
# with `--fresh`, so each opens its own conversation (no shared history) but the
# model + KV cache stay loaded across both within the single process.
#
# Use it before/after the system-anchor change: the BEFORE run shows chat 2's
# round 0 cold (cachedPromptTokens 0); the AFTER run should show it reusing the
# system-prefix tokens. The full envelope is tee'd to src-tauri/reports/bench/.
#
# Invoked by `make cli-kv-xconv` from the repo root. Override questions via env:
# XCONV_Q1="..." XCONV_Q2="..." make cli-kv-xconv
set -euo pipefail

cd "$(dirname "$0")/.."

DEMO_DIR="${EMAILOPS_DEMO_DIR:-$PWD/.emailops-demo-data}"
ACCOUNT="${XCONV_ACCOUNT:-alex@northwindlabs.io}"
Q1="${XCONV_Q1:-What invoices have I received recently?}"
Q2="${XCONV_Q2:-Summarize what my unread emails are about}"

if [ ! -f "$DEMO_DIR/emailops.db" ]; then
  echo "[cli-kv-xconv] no demo DB found — building one"
  make demo-db
fi
if ! sqlite3 "$DEMO_DIR/emailops.db" "SELECT 1 FROM embedding_chunks LIMIT 1;" 2>/dev/null | grep -q 1; then
  echo "[cli-kv-xconv] no embeddings found — generating (needed for chat)"
  make demo-embed
fi

mkdir -p src-tauri/reports/bench
out="src-tauri/reports/bench/kv_xconv_$(date +%Y%m%d_%H%M%S).json"
echo "[cli-kv-xconv] writing $out"

EMAILOPS_DATA_DIR="$DEMO_DIR" cargo run --manifest-path src-tauri/Cargo.toml \
  --features cli --bin emailops-cli -- \
  --json --account "$ACCOUNT" chat --fresh --trace "$Q1" "$Q2" | tee "$out"

echo
echo "[cli-kv-xconv] round-0 prompt cache reuse per chat:"
jq -r '
  .data.turns
  | to_entries[]
  | .key as $i
  | .value.trace.llmCalls[0]
  | "  chat \($i + 1) round0: promptTokens=\(.promptTokens // "?") cachedPromptTokens=\(.cachedPromptTokens // 0) prefillMs=\(.prefillMs // "?")"
' "$out" 2>/dev/null || echo "  (jq not available or unexpected shape — inspect $out manually)"
