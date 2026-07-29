#!/usr/bin/env bash
# Install a freshly built .app into /Applications, working around macOS
# App Management protection (macOS 13+). Apps that have been launched carry a
# `com.apple.provenance` xattr; the kernel then blocks any CLI process from
# deleting or overwriting them with "Operation not permitted" — sudo does NOT
# help. Finder holds the App Management entitlement, so we route the delete
# through it. The first run triggers a one-time "control Finder" automation
# prompt; approve it. If Finder deletion fails, we fall back to a plain rm
# (which succeeds only when the terminal itself has App Management / Full Disk
# Access granted).
#
# macOS-only by nature: /Applications, Finder automation and xattrs have no
# equivalent elsewhere. Linux/Windows install via `make dist-linux` /
# `make dist-windows` and the resulting package.
set -euo pipefail

. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib_data_dir.sh"
require_macos

SRC="${1:?usage: install_to_applications.sh <path-to-.app>}"
NAME="$(basename "$SRC")"
DEST="/Applications/$NAME"

if [ ! -d "$SRC" ]; then
  echo "ERROR: source app not found: $SRC" >&2
  exit 1
fi

if [ -d "$DEST" ]; then
  echo "Removing existing $DEST via Finder (App Management protection)…"
  if ! osascript -e "tell application \"Finder\" to delete POSIX file \"$DEST\"" >/dev/null 2>&1; then
    echo "Finder delete failed; trying direct rm…" >&2
    if ! rm -rf "$DEST" 2>/dev/null; then
      echo "ERROR: could not remove $DEST." >&2
      echo "Grant your terminal 'App Management' (or 'Full Disk Access') in" >&2
      echo "System Settings → Privacy & Security, then relaunch it and retry." >&2
      exit 1
    fi
  fi
fi

cp -R "$SRC" /Applications/
echo "Installed $DEST"
