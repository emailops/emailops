#!/usr/bin/env bash
# Give the universal DMG its Finder window layout without driving Finder.
#
# Tauri's bundle_dmg.sh lays the window out by sending AppleScript to Finder,
# which macOS only allows from an app the user granted Automation to — never
# from a background job. `make build-mac` therefore builds with CI=true (Tauri
# then passes --skip-jenkins and skips the AppleScript), and this script copies
# the layout Finder wrote for an earlier release (src-tauri/dmg/layout.DS_Store:
# window size and the positions of EmailOps.app and the Applications link) into
# the DMG. Runs before scripts/notarize_mac_dmg.sh, which re-signs the DMG.
set -euo pipefail

. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib_data_dir.sh"
require_macos

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LAYOUT="$ROOT/src-tauri/dmg/layout.DS_Store"
DMG="$(ls -t "$ROOT"/src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg 2>/dev/null | head -1 || true)"
if [ -z "$DMG" ]; then
  echo "[dmg-layout] ERROR: no universal .dmg found. Run 'make build-mac' first." >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'hdiutil detach "$WORK/mnt" >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

echo "[dmg-layout] adding the Finder layout to $(basename "$DMG")"
hdiutil convert "$DMG" -quiet -format UDRW -o "$WORK/rw.dmg"
mkdir "$WORK/mnt"
hdiutil attach "$WORK/rw.dmg" -quiet -nobrowse -noautoopen -mountpoint "$WORK/mnt"
cp "$LAYOUT" "$WORK/mnt/.DS_Store"
hdiutil detach "$WORK/mnt" -quiet
hdiutil convert "$WORK/rw.dmg" -quiet -format UDZO -imagekey zlib-level=9 -o "$WORK/final.dmg"
mv "$WORK/final.dmg" "$DMG"
echo "[dmg-layout] done → $DMG"
