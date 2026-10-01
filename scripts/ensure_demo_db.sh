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
elif [ scripts/generate_demo_db.py -nt "$db" ]; then
  # A DB built before the generator last changed lacks its new threads, and
  # the eval cases written against them fail on a stale DB, not on the model.
  echo "[demo] generate_demo_db.py is newer than the demo DB — rebuilding"
  make "$db_target"
elif [ -n "$(find src-tauri/migrations -name 'V*.sql' -newer "$db" -print -quit)" ]; then
  # The generator builds the schema from these migrations; a DB older than
  # one of them misses its tables, and the app migrating it on open would
  # leave any rows the new features need unseeded.
  echo "[demo] a migration is newer than the demo DB — rebuilding"
  make "$db_target"
else
  # mtimes lie after a branch switch that keeps an older checkout's files, so
  # also compare the DB's schema version with the newest migration file.
  latest="$(ls src-tauri/migrations | sed -n 's/^V0*\([0-9][0-9]*\)__.*\.sql$/\1/p' | sort -n | tail -1)"
  have="$(sqlite3 "$db" 'SELECT MAX(version) FROM refinery_schema_history;' 2>/dev/null || true)"
  if [ "${have:-0}" -lt "$latest" ]; then
    echo "[demo] demo DB schema is V${have:-0}, migrations reach V$latest — rebuilding"
    make "$db_target"
  fi
fi

if ! sqlite3 "$db" "SELECT 1 FROM embedding_chunks LIMIT 1;" 2>/dev/null | grep -q 1; then
  echo "[demo] no embeddings found — generating (needed for chat)"
  make "$embed_target"
fi

# The demo calendar is anchored to "now" (tomorrow 10:00 is always the next
# meeting) so the calendar chat evals stay deterministic however old the DB is.
lang=en; case "$db_target" in *-es) lang=es;; esac
uv run scripts/generate_demo_db.py --refresh-calendar --lang "$lang" --demo-db "$db" >/dev/null
