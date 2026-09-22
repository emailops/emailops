#!/usr/bin/env bash
#
# Run the published-docs claims against the app itself, in four phases:
#
#   fresh   a brand-new data dir: first-run wizard, factory defaults, every
#           settings tab, what lands on disk; ends by setting a main password
#   locked  the same data dir relaunched: the lock screen, the DB still readable
#   demo    the synthetic demo mailbox: reading pane, views, tag board, chat
#   cli     emailops-cli against a copy of the demo data dir
#
# Each phase writes <out>/<phase>.json; check_docs_run.py merges them. Every
# instance this script starts is stopped again even when a phase fails — a
# leftover instance would hold the port and the next run would refuse to start.
#
# Usage: scripts/check_docs_app.sh <out_dir>
set -uo pipefail
cd "$(dirname "$0")/.."

OUT="$1"
mkdir -p "$OUT"
V=.claude/skills/verify-emailops/scripts/verify.sh
DC=.claude/skills/verify-emailops/scripts/doc_claims.mjs
# Resolved with pwd -P: on macOS $TMPDIR is under /var, a symlink to /private/var.
# The app opens the real path, and the launcher, matching the path it was given,
# would wait out its whole timeout for an instance that is already up.
FRESH="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/emailops-docs-fresh.XXXXXX")" && pwd -P)"

uv run --no-project python - "$OUT/claims.json" <<'PY'
import json, sys, pathlib
sys.path.insert(0, "scripts")
import docs_claims_lib as L
claims = {b.claim: b.text for p in L.pages("en") for b in L.blocks(p) if b.claim}
pathlib.Path(sys.argv[1]).write_text(json.dumps(claims))
PY

phase() {  # phase <name> <data dir or ""> <run dir>
  local name="$1" data="$2" run="$3"
  echo "── $name ──"
  if VERIFY_DATA_DIR="${data:-$PWD/.emailops-demo-data}" VERIFY_RUN_DIR="$run" bash "$V" launch; then
    CLAIMS_JSON="$OUT/claims.json" DATA_DIR="$data" node "$DC" "$name" "$OUT" \
      || echo "$name: doc_claims.mjs exited non-zero (its JSON still records what ran)"
  else
    echo "[{\"claim\":\"__launch__\",\"status\":\"fail\",\"detail\":\"$name: the instance did not start\"}]" > "$OUT/$name.json"
  fi
  VERIFY_DATA_DIR="${data:-$PWD/.emailops-demo-data}" VERIFY_RUN_DIR="$run" bash "$V" cleanup >/dev/null 2>&1 || true
}

phase fresh  "$FRESH" "$OUT/fresh-run"
phase locked "$FRESH" "$OUT/locked-run"
rm -rf "$FRESH"
phase demo   ""       "$OUT/demo-run"

echo "── cli ──"
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --features cli --bin emailops-cli --quiet \
  && uv run --no-project python scripts/docs_cli_claims.py src-tauri/target/debug/emailops-cli .emailops-demo-data "$OUT"
