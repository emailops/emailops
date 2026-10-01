#!/usr/bin/env bash
# Build the synthetic demo DB with this checkout's schema. The generator copies
# its schema from a source DB; that source is an empty DB the app's own
# migrations create here, not the installed app's DB, which may be missing (a
# machine without EmailOps) or behind the checkout's migrations.
#
# Usage: demo_db.sh <demo-db-path> [generate_demo_db.py flags]
#   e.g. demo_db.sh "$PWD/.emailops-demo-data/emailops.db" --lang es

set -euo pipefail

demo_db="${1:?demo DB path is required}"
shift

schema_dir="$(mktemp -d)"
trap 'rm -rf "$schema_dir"' EXIT

cargo run --release --manifest-path src-tauri/Cargo.toml --example embed_demo_db -- \
  --init-schema --demo-dir "$schema_dir"
uv run scripts/generate_demo_db.py --prod-db "$schema_dir/emailops.db" --demo-db "$demo_db" "$@"
