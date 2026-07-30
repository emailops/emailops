#!/usr/bin/env bash
# Junk detector gate: spam / phishing / graymail.
#
# Fully synthetic and deterministic — no model, no network, no database — so
# this is fast enough to run on every change. Exits non-zero when the
# false-positive budget is blown.
#
#   bash scripts/eval_junk.sh
#   bash scripts/eval_junk.sh --case phish-bec-lookalike-domain-reply-to-mismatch
#   bash scripts/eval_junk.sh --tier smoke
#
# Reports land under src-tauri/reports/evaluations/junk/ (gitignored):
#   <run_id>.json          standard eval schema, for the dashboard
#   <run_id>_metrics.json  confusion matrices, PR curves, gate results
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="src-tauri/reports/evaluations/junk"
mkdir -p "$OUT_DIR"

# Default to the public synthetic corpus; a caller-supplied --cases-dir (e.g.
# the private golden set exported from a real mailbox) wins.
CASES_DIR="src-tauri/evals/junk/cases"
ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --cases-dir) CASES_DIR="$2"; shift 2 ;;
    --cases-dir=*) CASES_DIR="${1#*=}"; shift ;;
    *) ARGS+=("$1"); shift ;;
  esac
done

# `--no-default-features` skips the embedded llama.cpp build: this suite never
# loads a model, so there is nothing to gain from compiling one in.
exec cargo run \
  --manifest-path src-tauri/Cargo.toml \
  --no-default-features \
  --features eval \
  --example junk_eval -- \
  --cases-dir "$CASES_DIR" \
  --out "$OUT_DIR" \
  ${ARGS+"${ARGS[@]}"}
