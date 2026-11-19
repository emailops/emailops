#!/usr/bin/env bash
# Drive an IMAP account through the real desktop UI against the local test
# Dovecot: a fresh data dir, the first-run wizard's IMAP form, then star and
# archive from the UI checked on the server
# (.claude/skills/verify-emailops/scripts/imap_ui.mjs).
#
# Launches its own verify instance (port 1421, WebDriver 4445) and tears it
# down at the end; evidence stays under src-tauri/reports/verify/<run>/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
V="$ROOT/.claude/skills/verify-emailops/scripts/verify.sh"

bash "$ROOT/scripts/imap_test.sh" up
eval "$(bash "$ROOT/scripts/imap_test.sh" env)"

run_id="$(date +%Y%m%d-%H%M%S)"
export IMAP_UI_USER="ui-$run_id@example.test"
export IMAP_UI_SUBJECTS="UI invoice $run_id|UI meeting $run_id"
IFS='|' read -r -a subjects <<< "$IMAP_UI_SUBJECTS"
uv run --no-project python -I "$ROOT/scripts/imap_test/imap_probe.py" seed "$IMAP_UI_USER" "${subjects[@]}"

export VERIFY_DATA_DIR="$ROOT/.imap-test/ui-$run_id"
export VERIFY_RUN_DIR="$ROOT/src-tauri/reports/verify/imap-ui-$run_id"
mkdir -p "$VERIFY_DATA_DIR" "$VERIFY_RUN_DIR"
trap 'bash "$V" cleanup >/dev/null 2>&1 || true' EXIT
bash "$V" launch
node "$ROOT/.claude/skills/verify-emailops/scripts/imap_ui.mjs" "$VERIFY_RUN_DIR"
