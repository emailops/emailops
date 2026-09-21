#!/usr/bin/env bash
#
# Validate the published documentation against the app and render one HTML
# report, case by case, with a proposed edit for anything that fails.
#
# Three layers, cheapest first:
#   static   — parity, UI labels, file references, claim pairing (~0.1 s each)
#   contract — cargo tests comparing docs/site tables against the real catalog
#   doc      — docClaim() cases driving the running app (only with --with-app)
#
# Without --with-app the app-driven claims are reported as skipped rather than
# silently omitted, so the report is always a complete list of what the docs
# promise — never a partial one that reads as "all clear".
#
# Usage:
#   scripts/check_docs.sh                # static + contract, seconds
#   scripts/check_docs.sh --with-app     # also launches the verification instance
#
set -uo pipefail
cd "$(dirname "$0")/.."

WITH_APP=0
[ "${1:-}" = "--with-app" ] && WITH_APP=1

RUN="src-tauri/reports/docs/$(date +%Y%m%d-%H%M%S)-$(git rev-parse --short HEAD)"
mkdir -p "$RUN"

export CHECK_DOCS_RUN="$RUN"
export CHECK_DOCS_WITH_APP="$WITH_APP"
uv run --no-project python scripts/check_docs_run.py
status=$?

uv run --no-project python .claude/skills/verify-emailops/scripts/report_all.py \
  "$RUN/results.json" "$RUN/informe.html" >/dev/null
echo "informe: file://$(cd "$RUN" && pwd)/informe.html"
exit $status
