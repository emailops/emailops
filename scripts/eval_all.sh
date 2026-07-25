#!/usr/bin/env bash
# Run the standard eval suite against a prepared DB snapshot.

set -euo pipefail

: "${MODEL:?MODEL is required. Example: MODEL=qwen3.5-4b-q4_k_m}"
: "${EVAL_SNAPSHOT_DB:?EVAL_SNAPSHOT_DB is required}"

PROVIDER="${PROVIDER:-llamacpp}"
ACCOUNT="${ACCOUNT:-you@example.com}"
EVAL_LIMIT="${EVAL_LIMIT:-30}"
EVAL_DRAFT_N="${EVAL_DRAFT_N:-10}"

if [ ! -f "$EVAL_SNAPSHOT_DB" ]; then
  echo "ERROR: snapshot $EVAL_SNAPSHOT_DB missing. Run 'make eval-snapshot' first." >&2
  exit 1
fi

echo "[eval-all] provider=$PROVIDER model=$MODEL limit=$EVAL_LIMIT draft_n=$EVAL_DRAFT_N snapshot=$EVAL_SNAPSHOT_DB"

export EMAILOPS_EVAL_MODEL="$MODEL"
export EMAILOPS_EVAL_PROVIDER="$PROVIDER"

cd src-tauri

echo "── [eval-all] email_classification ──"
cargo run --features eval --example email_classification_eval -- \
  --account "$ACCOUNT" \
  --limit "$EVAL_LIMIT" \
  --model "$MODEL" \
  --provider "$PROVIDER" \
  --prod-db "$EVAL_SNAPSHOT_DB" \
  --in-place-dangerous

echo "── [eval-all] invoice_extract ──"
cargo run --features eval --example invoice_extract_eval -- \
  --account "$ACCOUNT" \
  --limit "$EVAL_LIMIT" \
  --prod-db "$EVAL_SNAPSHOT_DB" \
  --in-place-dangerous

echo "── [eval-all] task_extract ──"
cargo run --features eval --example task_extract_eval -- \
  --account "$ACCOUNT" \
  --limit "$EVAL_LIMIT" \
  --prod-db "$EVAL_SNAPSHOT_DB" \
  --in-place-dangerous

echo "── [eval-all] memory_extract ──"
cargo run --features eval --example memory_extract_eval -- \
  --account "$ACCOUNT" \
  --limit "$EVAL_LIMIT" \
  --prod-db "$EVAL_SNAPSHOT_DB" \
  --in-place-dangerous

echo "── [eval-all] agent_search ──"
cargo run --features eval --example agent_search_eval -- \
  --private \
  --cases-dir ../private-evals/agent_search \
  --prod-db "$EVAL_SNAPSHOT_DB" \
  --in-place-dangerous

echo "── [eval-all] draft ──"
cargo run --features eval --example draft_eval -- \
  --account "$ACCOUNT" \
  --n "$EVAL_DRAFT_N" \
  --prod-db "$EVAL_SNAPSHOT_DB" \
  --in-place-dangerous

echo "── [eval-all] translation ──"
# Fully synthetic (in-tree cases, in-memory DB) — no snapshot needed.
cargo run --features eval --example translation_eval -- \
  --model "$MODEL" \
  --provider "$PROVIDER"

echo "── [eval-all] chat ──"
cargo run --features eval --example chat_eval -- \
  --account "$ACCOUNT" \
  --private \
  --cases-dir ../private-evals/chat/cases \
  --prod-db "$EVAL_SNAPSHOT_DB" \
  --in-place-dangerous

echo "[eval-all] DONE — reports under src-tauri/reports/evaluations/"
