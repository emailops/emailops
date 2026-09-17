#!/usr/bin/env bash
# Reproduce the "EmailOps help" validation end to end on the synthetic demo DB:
#   1. the same app question with the feature OFF (before) and ON (after),
#   2. the app_help eval cases (src-tauri/evals/chat/cases/app_help.yaml) only,
#   3. the HTML report (scripts/help_docs_report.py).
# Writes JSON under src-tauri/reports/help-docs/ (gitignored). Needs the local
# chat + embedding models the app already has (make cli-eval uses the same).
#
# Usage: scripts/help_docs_eval.sh            # everything
#        scripts/help_docs_eval.sh --skip-turns   # eval + report only
set -euo pipefail
cd "$(dirname "$0")/.."

DEMO_DIR="${EMAILOPS_DEMO_DIR:-$PWD/.emailops-demo-data}"
RUNS="$PWD/src-tauri/reports/help-docs"
ACCOUNT="${HELP_DOCS_EVAL_ACCOUNT:-ulises@emailopslabs.dev}"
QUESTION="how do I make EmailOps use my local Ollama instead of the built-in model?"
CLI=(cargo run --manifest-path src-tauri/Cargo.toml --features cli,eval --bin emailops-cli --)
mkdir -p "$RUNS"

scripts/ensure_demo_db.sh "$DEMO_DIR" demo-db demo-embed
db="$DEMO_DIR/emailops.db"
set_pref() { sqlite3 "$db" "INSERT OR REPLACE INTO user_preferences(key,value) VALUES('$1','$2');"; }

if [ "${1:-}" != "--skip-turns" ]; then
  echo "[help-docs] before: help_docs_enabled=false"
  set_pref help_docs_enabled false
  EMAILOPS_DATA_DIR="$DEMO_DIR" "${CLI[@]}" chat --account "$ACCOUNT" "$QUESTION" --json --trace --fresh > "$RUNS/before.json"
  echo "[help-docs] after: help_docs_enabled=true"
  set_pref help_docs_enabled true
  EMAILOPS_DATA_DIR="$DEMO_DIR" "${CLI[@]}" chat --account "$ACCOUNT" "$QUESTION" --json --trace --fresh > "$RUNS/after.json"
fi
set_pref help_docs_enabled true

# Only the app_help cases: the CLI takes one cases dir, so stage a copy.
cases_dir="$(mktemp -d)"
cp src-tauri/evals/chat/cases/app_help.yaml "$cases_dir/"
echo "[help-docs] eval: app_help cases"
EMAILOPS_DATA_DIR="$DEMO_DIR" "${CLI[@]}" eval --account "$ACCOUNT" --cases-dir "$cases_dir" --json > "$RUNS/eval.json" || true
rm -rf "$cases_dir"

python3 - "$RUNS/meta.json" <<'PY'
import json, platform, subprocess, sys
chip = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True).stdout.strip() if sys.platform == "darwin" else platform.processor()
json.dump({"machine": f"modelo qwen3.5-4b-q4_k_m · llama.cpp · {chip or platform.machine()}"}, open(sys.argv[1], "w"))
PY
uv run scripts/help_docs_report.py --runs "$RUNS" --demo-db "$db"
echo "[help-docs] report: $RUNS/report.html"
