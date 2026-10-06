#!/usr/bin/env bash
# Verify the signed universal macOS build before it is published: codesign,
# architectures, Gatekeeper, notarization tickets (app and DMG), the universal
# slice guard, and the entitlement guard. Exits non-zero on any failure.
#
# Usage: verify_mac.sh [APP] [DMG]   (defaults: the latest `make build-mac`
# output; pass a downloaded release's app/DMG to check what actually shipped)
set -euo pipefail

. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib_data_dir.sh"
require_macos

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE="$ROOT/src-tauri/target/universal-apple-darwin/release/bundle"
APP="${1:-$(ls -d "$BUNDLE"/macos/*.app 2>/dev/null | head -1 || true)}"
DMG="${2:-$(ls -t "$BUNDLE"/dmg/*.dmg 2>/dev/null | head -1 || true)}"
if [ -z "$APP" ]; then
  echo "ERROR: no .app found. Run 'make build-mac' first."
  exit 1
fi

indent() { sed 's/^/  /'; }
FAILED=0
fail() { echo "  ❌ FAIL: $*"; FAILED=1; }

echo "Verifying $APP"
echo "── codesign ──"; codesign -dv --verbose=4 "$APP" 2>&1 | indent
echo "── architectures ──"; file "$APP/Contents/MacOS/"* 2>&1 | indent
echo "── spctl (app) ──"; spctl -a -t exec -vv "$APP" 2>&1 | indent || fail "Gatekeeper rejected the app"
echo "── stapler (app) ──"; xcrun stapler validate "$APP" 2>&1 | indent || fail "no notarization ticket stapled to the app"

echo "── universal-slice guard ──"
SLICES="$(file "$APP/Contents/MacOS/"* 2>/dev/null)"
MISSING=""
echo "$SLICES" | grep -q "arm64"  || MISSING="$MISSING arm64"
echo "$SLICES" | grep -q "x86_64" || MISSING="$MISSING x86_64"
if [ -n "$MISSING" ]; then
  fail "this is not a universal bundle — missing:$MISSING"
  echo "  One macOS DMG has to launch on every Mac; a dropped slice silently strands"
  echo "  that half of users on a download that will not open."
else
  echo "  ✅ universal (arm64 + x86_64); embedded AI is gated off Intel at runtime"
fi

# Hardened Runtime exception entitlements weaken code-signing protections
# (CASA/DASA 3.3.2). The app needs none — see src-tauri/entitlements.plist.
echo "── entitlement guard ──"
ENTITLEMENTS="$(codesign -d --entitlements - --xml "$APP" 2>/dev/null || true)"
DANGEROUS="$(printf '%s' "$ENTITLEMENTS" | grep -oE 'com\.apple\.security\.cs\.[a-z-]+|com\.apple\.security\.get-task-allow' | sort -u || true)"
if [ -n "$DANGEROUS" ]; then
  fail "the app carries Hardened Runtime exception entitlements:"
  printf '%s\n' "$DANGEROUS" | sed 's/^/      /'
else
  echo "  ✅ no Hardened Runtime exception entitlements"
fi

echo "── DMG ──"
if [ -z "$DMG" ]; then
  fail "no .dmg found next to the app"
else
  echo "  $(basename "$DMG")"
  spctl -a -t open --context context:primary-signature -vv "$DMG" 2>&1 | indent \
    || fail "Gatekeeper rejected the DMG (run scripts/notarize_mac_dmg.sh)"
  xcrun stapler validate "$DMG" 2>&1 | indent || fail "no notarization ticket stapled to the DMG"
  # The window layout comes from src-tauri/dmg/layout.DS_Store (see
  # scripts/dmg_apply_layout.sh); without it Finder opens the DMG unarranged.
  MNT="$(mktemp -d)"
  if hdiutil attach "$DMG" -nobrowse -readonly -mountpoint "$MNT" >/dev/null; then
    if [ -f "$MNT/.DS_Store" ]; then echo "  ✅ Finder layout (.DS_Store) present"; else fail "the DMG has no Finder layout (.DS_Store)"; fi
    hdiutil detach "$MNT" >/dev/null || hdiutil detach -force "$MNT" >/dev/null
  else
    fail "could not mount the DMG to check its layout"
  fi
  rmdir "$MNT" 2>/dev/null || true
fi

exit "$FAILED"
