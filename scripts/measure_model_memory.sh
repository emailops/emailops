#!/usr/bin/env bash
# Peak memory (max RSS, which counts the mmapped weights) of one real chat turn
# per catalog model: load + prefill + answer.
#
# These are the "measured peak" figures quoted in docs/site/*/ai-features.md
# (model catalog). They depend on the machine — the context window scales with
# installed memory — so re-run on the reference machine (a 16 GB Mac) and
# update the four languages when a model or the runtime changes.
#
# Usage: scripts/measure_model_memory.sh [model-id ...]
# Runs against a copy of the demo DB; models are read from the app's own
# models dir, so download them in the app first.
set -uo pipefail
cd "$(dirname "$0")/.."

DEMO_DIR="${EMAILOPS_DEMO_DIR:-.emailops-demo-data}"
MODELS_DIR="${MODELS_DIR:-$HOME/Library/Application Support/com.emailops.app/models}"
ACCOUNT="${ACCOUNT:-ulises@emailopslabs.dev}"
QUESTION="${QUESTION:-What did Nadia Brunner ask about?}"
MODELS=("$@")
[ ${#MODELS[@]} -eq 0 ] && MODELS=(qwen3.5-4b-q4_k_m qwen3.5-4b-q8_0 qwen3.5-9b-q4_k_m gemma-4-12b-it-qat-ud-q4_k_xl)

[ -f "$DEMO_DIR/emailops.db" ] || { echo "no demo DB at $DEMO_DIR — run make demo-db" >&2; exit 2; }
cargo build --manifest-path src-tauri/Cargo.toml --features cli --bin emailops-cli >&2 || exit 1
CLI=src-tauri/target/debug/emailops-cli

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cp "$DEMO_DIR"/emailops.db* "$WORK/"
ln -s "$MODELS_DIR" "$WORK/models"

printf "model\tpeak_gb\tok\n"
for model in "${MODELS[@]}"; do
  "/usr/bin/time" -l "$CLI" --data-dir "$WORK" --account "$ACCOUNT" --model "$model" \
    chat "$QUESTION" --json > "$WORK/out.json" 2> "$WORK/err.log"
  peak=$(awk '/maximum resident set size/ {printf "%.2f", $1 / 1e9}' "$WORK/err.log")
  ok=$(grep -q '"ok":true' "$WORK/out.json" && echo yes || echo no)
  printf "%s\t%s\t%s\n" "$model" "${peak:-?}" "$ok"
done
