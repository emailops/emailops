#!/usr/bin/env bash
#
# Compare chat models across the four evals that measure a reply: chat,
# query planner, classifier and drafts. Reports accuracy, wall time and peak
# resident memory per model, as a text table and an HTML page.
#
# Binaries are built ONCE and then executed directly rather than through
# `cargo run`, so the pid sampled for memory is the process that loads the
# model, not cargo's.
#
# Usage:
#   scripts/model_bench.sh --models a,b [--repeats N] [--tier smoke] [--drafts N]
#
# Env:
#   LLAMA_PATCH_DIR  path to a patched llama-cpp-sys-2, for a candidate whose
#                    GGUF stock llama.cpp cannot load. When set, every cargo
#                    invocation gets the corresponding [patch.crates-io] via
#                    --config, so nothing committed changes and the normal
#                    build is untouched. Leave unset for the stock runtime.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="${EMAILOPS_DEMO_DIR:-$REPO_ROOT/.emailops-demo-data}"
ACCOUNT="${EMAILOPS_BENCH_ACCOUNT:-ulises@emailopslabs.dev}"
OUT_DIR="$REPO_ROOT/reports/bench"
STAMP="$(date +%Y%m%d_%H%M%S)"
WORK="$OUT_DIR/model_bench_${STAMP}"

MODELS=""
REPEATS=1
TIER="smoke"
DRAFTS=6

while [[ $# -gt 0 ]]; do
  case "$1" in
    --models)  MODELS="$2"; shift 2 ;;
    --repeats) REPEATS="$2"; shift 2 ;;
    --tier)    TIER="$2"; shift 2 ;;
    --drafts)  DRAFTS="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$MODELS" ]]; then
  echo "usage: $0 --models <id>[,<id>...] [--repeats N] [--tier smoke|full] [--drafts N]" >&2
  exit 2
fi

mkdir -p "$WORK"

CARGO_CONFIG=()
if [[ -n "${LLAMA_PATCH_DIR:-}" ]]; then
  CARGO_CONFIG=(--config "patch.crates-io.llama-cpp-sys-2.path=\"$LLAMA_PATCH_DIR\"")
  echo "[bench] llama.cpp patched from $LLAMA_PATCH_DIR"
fi

EXAMPLES=(tag_classification_eval query_plan_eval chat_eval draft_eval)
echo "[bench] building harnesses"
BUILD_ARGS=()
for ex in "${EXAMPLES[@]}"; do BUILD_ARGS+=(--example "$ex"); done
cargo build --quiet --manifest-path "$REPO_ROOT/src-tauri/Cargo.toml" \
  "${CARGO_CONFIG[@]}" --features eval "${BUILD_ARGS[@]}"
BIN_DIR="$REPO_ROOT/src-tauri/target/debug/examples"

# Run one harness, sampling its resident set every 200 ms. Writes stdout to
# $2 and echoes "<peak_rss_kib> <wall_ms>".
run_sampled() {
  local stdout_file="$1"; shift
  local started
  started=$(python3 -c 'import time; print(int(time.time()*1000))')
  "$@" > "$stdout_file" 2>"${stdout_file%.out}.err" &
  local pid=$!
  local peak=0 rss
  while kill -0 "$pid" 2>/dev/null; do
    rss=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || true)
    if [[ -n "$rss" && "$rss" =~ ^[0-9]+$ && "$rss" -gt "$peak" ]]; then peak="$rss"; fi
    sleep 0.2
  done
  wait "$pid" || true
  local ended
  ended=$(python3 -c 'import time; print(int(time.time()*1000))')
  echo "$peak $((ended - started))"
}

newest_in() { ls -t "$1"/*.json 2>/dev/null | head -1; }

for model in ${MODELS//,/ }; do
  safe="${model//[^a-zA-Z0-9_.-]/_}"
  echo "[bench] ── $model ──"

  echo "[bench]   classifier"
  read -r rss ms < <(run_sampled "$WORK/${safe}_classify.out" \
    env EMAILOPS_DATA_DIR="$DEMO_DIR" "$BIN_DIR/tag_classification_eval" \
    --prod-db "$DEMO_DIR/emailops.db" --model "$model" --repeats "$REPEATS" --json)
  echo "{\"rssKib\": $rss, \"wallMs\": $ms}" > "$WORK/${safe}_classify.meta.json"

  echo "[bench]   planner"
  read -r rss ms < <(run_sampled "$WORK/${safe}_plan.out" \
    env EMAILOPS_DATA_DIR="$DEMO_DIR" "$BIN_DIR/query_plan_eval" \
    --prod-db "$DEMO_DIR/emailops.db" --account "$ACCOUNT" --model "$model" --json)
  echo "{\"rssKib\": $rss, \"wallMs\": $ms}" > "$WORK/${safe}_plan.meta.json"

  echo "[bench]   chat (tier $TIER)"
  read -r rss ms < <(run_sampled "$WORK/${safe}_chat.out" \
    env EMAILOPS_DATA_DIR="$DEMO_DIR" "$BIN_DIR/chat_eval" \
    --prod-db "$DEMO_DIR/emailops.db" --account "$ACCOUNT" --model "$model" --tier "$TIER")
  echo "{\"rssKib\": $rss, \"wallMs\": $ms}" > "$WORK/${safe}_chat.meta.json"
  cp "$(newest_in "$REPO_ROOT/reports/evaluations/chat")" "$WORK/${safe}_chat.report.json" 2>/dev/null || true

  echo "[bench]   drafts (n=$DRAFTS)"
  read -r rss ms < <(run_sampled "$WORK/${safe}_drafts.out" \
    env EMAILOPS_DATA_DIR="$DEMO_DIR" EMAILOPS_EVAL_MODEL="$model" "$BIN_DIR/draft_eval" \
    --prod-db "$DEMO_DIR/emailops.db" --account "$ACCOUNT" --n "$DRAFTS")
  echo "{\"rssKib\": $rss, \"wallMs\": $ms}" > "$WORK/${safe}_drafts.meta.json"
  cp "$(newest_in "$REPO_ROOT/reports/evaluations/drafts")" "$WORK/${safe}_drafts.report.json" 2>/dev/null || true
done

python3 "$REPO_ROOT/scripts/model_bench_report.py" "$WORK" "$MODELS" "$TIER" "$REPEATS"
echo "[bench] text → ${WORK}.txt"
echo "[bench] html → ${WORK}.html"
