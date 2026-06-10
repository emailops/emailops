#!/usr/bin/env bash
# Ensure a demo data dir has both a synthetic DB and embeddings, building them
# on demand. Shared by the `demo`, `demo-es`, and `cli-demo` Makefile targets so
# the build-if-missing guard lives in exactly one place.
#
# Usage: ensure_demo_db.sh <demo-dir> <db-make-target> <embed-make-target>
#   e.g. ensure_demo_db.sh "$PWD/.emailops-demo-data" demo-db demo-embed

set -euo pipefail

demo_dir="${1:?demo dir is required}"
db_target="${2:?db make target is required}"
embed_target="${3:?embed make target is required}"
db="$demo_dir/emailops.db"

if [ ! -f "$db" ]; then
  echo "[demo] no demo DB found — building one"
  make "$db_target"
fi

if ! sqlite3 "$db" "SELECT 1 FROM embedding_chunks LIMIT 1;" 2>/dev/null | grep -q 1; then
  echo "[demo] no embeddings found — generating (needed for chat)"
  make "$embed_target"
fi
