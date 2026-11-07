#!/usr/bin/env bash
# End-to-end validation of IMAP accounts against real local servers
# (Dovecot + Mailpit + Toxiproxy in Docker, see scripts/imap_test.sh).
#
#   scripts/imap_e2e.sh live   the provider contract, sending and network
#                              faults through ImapClient (tests/imap_live.rs)
#   scripts/imap_e2e.sh cli    the CLI flow: add account, sync, list, search,
#                              show, send, re-sync (scripts/imap_test/e2e_cli.py)
#   scripts/imap_e2e.sh all    both (default)
#
# Starts the servers if needed and leaves them running; `make imap-down`
# stops them.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODE="${1:-all}"

bash "$ROOT/scripts/imap_test.sh" up
eval "$(bash "$ROOT/scripts/imap_test.sh" env)"

status=0
if [ "$MODE" = live ] || [ "$MODE" = all ]; then
  echo "── live ImapClient tests ──"
  cargo test --manifest-path "$ROOT/src-tauri/Cargo.toml" --no-default-features --test imap_live || status=1
fi
if [ "$MODE" = cli ] || [ "$MODE" = all ]; then
  echo "── CLI end-to-end ──"
  cargo build --manifest-path "$ROOT/src-tauri/Cargo.toml" --no-default-features --features cli --bin emailops-cli
  target_dir="$(cargo metadata --manifest-path "$ROOT/src-tauri/Cargo.toml" --format-version 1 --no-deps \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  uv run --no-project python -I "$ROOT/scripts/imap_test/e2e_cli.py" \
    "$target_dir/debug/emailops-cli" "$ROOT/.imap-test" || status=1
fi
exit $status
