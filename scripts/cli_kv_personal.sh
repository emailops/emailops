#!/usr/bin/env bash
# Run the cli-kv bench against the user's PRODUCTION data dir + a personal
# account, with the exact "today summary" prompt the in-app shortcut sends,
# so we can reproduce KV-cache regressions that only show up on real data.
#
# Both inputs are sourced from environment — nothing sensitive ever lands in
# a git-tracked file:
#
#   EMAILOPS_DATA_DIR           Path to the prod app data dir. Default macOS
#                               location is
#                               ~/Library/Application Support/com.emailops.app
#                               You already use this var for `make dev`, so
#                               setting it in .env.local does double duty.
#
#   EMAILOPS_PERSONAL_ACCOUNT   Account email registered in that data dir
#                               (any row in the `accounts` table works).
#
# The prompt is the literal string the frontend's "today summary" shortcut
# sends. It's hardcoded here on purpose — keeping a personal prompt inside a
# tracked script is fine when the prompt is also visible in the app UI; the
# sensitive bit is the data, not the question.
#
# Output:
#   src-tauri/reports/bench/kv_personal_<ts>.stdout.log   (JSON envelope)
#   src-tauri/reports/bench/kv_personal_<ts>.stderr.log   (chat: + ai: logs)
# Both gitignored under src-tauri/reports/.

set -euo pipefail
cd "$(dirname "$0")/.."

err() { echo "[cli-kv-personal] $*" >&2; exit 1; }

[ -n "${EMAILOPS_DATA_DIR:-}" ]          || err "EMAILOPS_DATA_DIR is not set (add it to .env.local; see .env.local.example)"
[ -n "${EMAILOPS_PERSONAL_ACCOUNT:-}" ]  || err "EMAILOPS_PERSONAL_ACCOUNT is not set (add it to .env.local; see .env.local.example)"

[ -d "$EMAILOPS_DATA_DIR" ]              || err "data dir not found: $EMAILOPS_DATA_DIR"
[ -f "$EMAILOPS_DATA_DIR/emailops.db" ]  || err "emailops.db not found under $EMAILOPS_DATA_DIR — wrong data dir?"

# The exact today-summary prompt the frontend shortcut sends. Mirrors
# `src/components/Chat/ChatShortcuts.tsx` so the bench reproduces the in-app
# trace byte-for-byte. Update both places together if you change one.
readonly PROMPT="Summarise the emails I've received today. Format it as a markdown table with the columns | Sender | Subject | Time | Urgency | Summary |, sorted by urgency. Cite each email with its reference number. End with a short paragraph highlighting the most important things of the day."

# The Tauri app holds an exclusive flock on emailops.lock; the CLI doesn't
# take it, but two processes loading the same chat model double the RAM
# footprint (≈4–9 GB each). Warn so the user can close the app first.
if [ -f "$EMAILOPS_DATA_DIR/emailops.lock" ]; then
  pid=$(cat "$EMAILOPS_DATA_DIR/emailops.lock" 2>/dev/null || echo '?')
  if kill -0 "$pid" 2>/dev/null; then
    echo "[cli-kv-personal] ⚠ EmailOps app appears to be running (pid=$pid)."
    echo "[cli-kv-personal]   The CLI will load the chat model into a SECOND process — on M1 16 GB"
    echo "[cli-kv-personal]   this can swap heavily or OOM. Close the app first if you can."
    echo "[cli-kv-personal]   SQLite WAL allows the read path; --fresh writes a NEW conversation."
    echo
  fi
fi

mkdir -p src-tauri/reports/bench
ts=$(date +%Y%m%d_%H%M%S)
out_stdout="src-tauri/reports/bench/kv_personal_${ts}.stdout.log"
out_stderr="src-tauri/reports/bench/kv_personal_${ts}.stderr.log"
echo "[cli-kv-personal] stdout → $out_stdout"
echo "[cli-kv-personal] stderr → $out_stderr"
echo "[cli-kv-personal] account: $EMAILOPS_PERSONAL_ACCOUNT"
echo "[cli-kv-personal] data dir: $EMAILOPS_DATA_DIR"
echo

EMAILOPS_DATA_DIR="$EMAILOPS_DATA_DIR" cargo run \
  --manifest-path src-tauri/Cargo.toml \
  --features cli --bin emailops-cli -- \
  --json --account "$EMAILOPS_PERSONAL_ACCOUNT" \
  chat --fresh --trace "$PROMPT" \
  >"$out_stdout" 2>"$out_stderr"

echo
echo "[cli-kv-personal] LLM calls in this run:"
jq -r '
  .data.turns[0].trace.llmCalls
  | to_entries[]
  | "  \(.value.kind) round \(.value.round): promptTokens=\(.value.promptTokens // "?") cachedPromptTokens=\(.value.cachedPromptTokens // 0) prefillMs=\(.value.prefillMs // "?") plan=\(.value.prefixPlan // "?") sysAfter=\(.value.sysCachedAfter // 0)"
' "$out_stdout" 2>/dev/null || echo "  (jq not available or unexpected shape — inspect $out_stdout manually)"

# Surface ALL system_prefix_bytes None branches — these are the ones we
# instrumented in runtime.rs to pin down which condition returns None.
if grep -q "llamacpp sys_prefix_bytes:" "$out_stderr"; then
  echo
  echo "[cli-kv-personal] system_prefix_bytes None branches that fired:"
  grep "llamacpp sys_prefix_bytes:" "$out_stderr" | sed 's/^/  /'
else
  echo
  echo "[cli-kv-personal] (no sys_prefix_bytes None branches fired — anchor seeded on every call)"
fi

echo
echo "[cli-kv-personal] per-call kv plan (cached / sys_cached transitions):"
grep -E "llamacpp kv: (cached=|uncached)" "$out_stderr" | sed 's/^/  /'
