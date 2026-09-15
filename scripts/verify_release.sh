#!/usr/bin/env bash
# Full verification for a release (Phase 1b of the release skill): one `make verify`,
# a Markdown summary written to docs/verification/ (the release commit carries it; the
# HTML report stays local) and only the last KEEP local runs kept.
# Usage: scripts/verify_release.sh
set -euo pipefail
cd "$(dirname "$0")/.."
REPORTS=src-tauri/reports/verify
KEEP=10

# Another EmailOps instance on the demo DB holds the GPU: every eval would fail
# with Metal out-of-memory and the e2e launch refuses to double-drive it.
if lsof -t .emailops-demo-data/emailops.db >/dev/null 2>&1; then
  echo "error: EmailOps process(es) $(lsof -t .emailops-demo-data/emailops.db | tr '\n' ' ')have the demo DB open; close them and rerun" >&2
  exit 1
fi

VERIFY_EVAL_MODEL=qwen3.6-35b-a3b-ud-q4_k_xl make verify
python3 .claude/skills/verify-emailops/scripts/summary_md.py "$REPORTS/current-full/results.json" docs/verification

runs=()
while IFS= read -r r; do runs+=("$r"); done < <(ls -1d "$REPORTS"/[0-9]*-full | sort)
for ((i = 0; i < ${#runs[@]} - KEEP; i++)); do rm -rf -- "${runs[$i]}"; done
