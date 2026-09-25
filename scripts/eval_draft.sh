#!/usr/bin/env bash
# Run only the draft eval (real reply pairs) against the eval snapshot DB.
# Draft-only sibling of eval_all.sh so a draft change doesn't pay for the
# whole suite.

set -euo pipefail

: "${MODEL:?MODEL is required. Example: MODEL=qwen3.5-4b-q4_k_m}"
: "${EVAL_SNAPSHOT_DB:?EVAL_SNAPSHOT_DB is required}"
: "${ACCOUNT:?ACCOUNT is required (account email or id)}"

PROVIDER="${PROVIDER:-llamacpp}"
EVAL_DRAFT_N="${EVAL_DRAFT_N:-10}"
# Optional fixed judge so different generators are scored by the same model.
JUDGE_ARGS=()
if [ -n "${JUDGE_MODEL:-}" ]; then JUDGE_ARGS=(--judge-model "$JUDGE_MODEL"); fi

if [ ! -f "$EVAL_SNAPSHOT_DB" ]; then
  echo "ERROR: snapshot $EVAL_SNAPSHOT_DB missing. Run 'make eval-snapshot' first." >&2
  exit 1
fi

export EMAILOPS_EVAL_MODEL="$MODEL"
export EMAILOPS_EVAL_PROVIDER="$PROVIDER"

cd src-tauri
cargo run --features eval --example draft_eval -- \
  --account "$ACCOUNT" \
  --n "$EVAL_DRAFT_N" \
  --prod-db "$EVAL_SNAPSHOT_DB" \
  --in-place-dangerous \
  ${JUDGE_ARGS[@]+"${JUDGE_ARGS[@]}"}
