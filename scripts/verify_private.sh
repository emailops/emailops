#!/usr/bin/env bash
# Private verification run: private eval suites against the production DB
# snapshot, HTML report under the gitignored reports tree (never publish it).
# Usage: scripts/verify_private.sh [--tier smoke|full|lab|all] [--skip layer,...] [--only layer,...]
set -euo pipefail
skill=".claude/skills/verify-emailops/scripts"
python3 "$skill/verify_private.py" "$@"
run="$(readlink src-tauri/reports/verify-private/current-private)"
python3 "$skill/report_all.py" "$run/results.json" "$run/informe.html"
echo "informe privado: $run/informe.html"
