#!/usr/bin/env bash
# Full verification run: every test layer, results per feature, HTML report.
# Usage: scripts/verify_all.sh [--tier quick|full] [--skip layer,...] [--only layer,...]
set -euo pipefail
skill=".claude/skills/verify-emailops/scripts"
python3 "$skill/verify_all.py" "$@"
run="$(readlink src-tauri/reports/verify/current-full)"
python3 "$skill/report_all.py" "$run/results.json" "$run/informe.html"
echo "informe: $run/informe.html"
