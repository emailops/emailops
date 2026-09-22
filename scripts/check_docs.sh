#!/usr/bin/env bash
#
# Validate the published documentation against the app and render the docs
# as an HTML report, each fragment coloured by what validated it (see
# scripts/docs_report.py).
#
#   static   — parity, UI labels, file references, claim pairing (~0.1 s each)
#   catalog  — docs/site/claims.toml: tests, generated tables, release, files
#   app      — the app itself, four phases (only with --with-app)
#
# Without --with-app the app cases are reported as pending (yellow), never as
# passed, so the report is always the whole docs — never a partial all-clear.
#
# Usage:
#   scripts/check_docs.sh                  # static + catalog, seconds
#   scripts/check_docs.sh --with-app       # also drives the app
#   scripts/check_docs.sh --render <run>   # re-evaluate a run dir (e.g. after the
#                                          # agent wrote judge/judgments.json)
#
set -uo pipefail
cd "$(dirname "$0")/.."

WITH_APP=0 RENDER_ONLY=0
case "${1:-}" in
  --with-app) WITH_APP=1 ;;
  --render) RENDER_ONLY=1; RUN="${2:?usage: check_docs.sh --render <run dir>}" ;;
esac
[ "$RENDER_ONLY" = 1 ] || RUN="src-tauri/reports/docs/$(date +%Y%m%d-%H%M%S)-$(git rev-parse --short HEAD)"
mkdir -p "$RUN"

export CHECK_DOCS_RUN="$RUN" CHECK_DOCS_WITH_APP="$WITH_APP" CHECK_DOCS_RENDER_ONLY="$RENDER_ONLY"
uv run --no-project python scripts/check_docs_run.py
status=$?

uv run --no-project python scripts/docs_report.py "$RUN/results.json" "$RUN/informe.html"
echo "informe: file://$(cd "$RUN" && pwd)/informe.html"
exit $status
