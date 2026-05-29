#!/usr/bin/env bash
# Create a reusable DB snapshot for eval runs.

set -euo pipefail

SOURCE_DATA_DIR="${EMAILOPS_DATA_DIR:?EMAILOPS_DATA_DIR is required}"
SNAPSHOT_DIR="${EVAL_SNAPSHOT_DIR:?EVAL_SNAPSHOT_DIR is required}"
SNAPSHOT_DB="${EVAL_SNAPSHOT_DB:?EVAL_SNAPSHOT_DB is required}"
SOURCE_DB="$SOURCE_DATA_DIR/emailops.db"

if [ ! -f "$SOURCE_DB" ]; then
  echo "ERROR: source DB not found at $SOURCE_DB" >&2
  exit 1
fi

mkdir -p "$SNAPSHOT_DIR"
echo "[eval-snapshot] copying source DB from $SOURCE_DATA_DIR -> $SNAPSHOT_DB"
cp "$SOURCE_DB" "$SNAPSHOT_DB"

if [ -f "$SOURCE_DB-wal" ]; then
  cp "$SOURCE_DB-wal" "$SNAPSHOT_DIR/"
fi

if [ -f "$SOURCE_DB-shm" ]; then
  cp "$SOURCE_DB-shm" "$SNAPSHOT_DIR/"
fi

ls -lh "$SNAPSHOT_DB"
