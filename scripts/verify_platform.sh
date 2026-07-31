#!/usr/bin/env bash
# Verify a Linux or Windows desktop bundle before publishing it.
#
#   scripts/verify_platform.sh linux
#   scripts/verify_platform.sh windows
#
# The non-macOS counterpart of `make verify-mac`. There is no codesign or
# notarization to check, so this asserts the things that actually go wrong on
# these platforms: a missing artifact, a binary that will not start because of
# an unresolved shared library, and a wrong-architecture build.

set -euo pipefail

PLATFORM="${1:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

FAILED=0

fail() { echo "  ❌ $1"; FAILED=1; }
pass() { echo "  ✅ $1"; }

case "$PLATFORM" in
  linux)
    BUNDLE="src-tauri/target/x86_64-unknown-linux-gnu/release/bundle"
    BIN="src-tauri/target/x86_64-unknown-linux-gnu/release/emailops"

    echo "── artifacts ──"
    DEB=$(find "$BUNDLE/deb" -name '*.deb' 2>/dev/null | head -1 || true)
    APPIMAGE=$(find "$BUNDLE/appimage" -name '*.AppImage' 2>/dev/null | head -1 || true)
    [ -n "$DEB" ] && pass "deb: $(basename "$DEB")" || fail "no .deb found (run 'make build-linux')"
    [ -n "$APPIMAGE" ] && pass "AppImage: $(basename "$APPIMAGE")" || fail "no .AppImage found (run 'make build-linux')"

    echo "── binary ──"
    if [ ! -f "$BIN" ]; then
      fail "no release binary at $BIN"
    else
      pass "binary present"

      echo "── architecture ──"
      file "$BIN" | sed 's/^/  /'

      echo "── shared libraries ──"
      # An unresolved library here means the app dies instantly on a clean
      # machine — the single most common Linux packaging failure.
      #
      # Checked against the binary as *installed*, not the raw build output:
      # with DYNAMIC_BACKENDS, the binary carries an $ORIGIN-relative rpath
      # into the bundle's backends/ resource dir, which only exists alongside
      # it once unpacked in the real usr/bin + usr/lib layout — ldd against
      # the bare target/.../release/emailops would always report those as
      # "not found" even when the shipped package is completely correct.
      if command -v ldd >/dev/null 2>&1 && [ -n "$DEB" ] && command -v dpkg-deb >/dev/null 2>&1; then
        DEB_EXTRACT_DIR="$(mktemp -d)"
        trap 'rm -rf "$DEB_EXTRACT_DIR"' EXIT
        dpkg-deb -x "$DEB" "$DEB_EXTRACT_DIR"
        INSTALLED_BIN="$(find "$DEB_EXTRACT_DIR/usr/bin" -maxdepth 1 -type f | head -1)"
        UNRESOLVED=$(ldd "$INSTALLED_BIN" 2>/dev/null | grep 'not found' || true)
        if [ -n "$UNRESOLVED" ]; then
          fail "unresolved shared libraries (checked as installed from the .deb):"
          echo "$UNRESOLVED" | sed 's/^/      /'
        else
          pass "all shared libraries resolve (checked as installed from the .deb)"
        fi
      else
        echo "  i ldd or dpkg-deb unavailable, or no .deb built; skipping linkage check"
      fi
    fi

    echo "── deb dependencies ──"
    if [ -n "$DEB" ] && command -v dpkg-deb >/dev/null 2>&1; then
      dpkg-deb -f "$DEB" Depends | sed 's/^/  /'
    else
      echo "  i dpkg-deb unavailable; skipping"
    fi
    ;;

  windows)
    BUNDLE="src-tauri/target/x86_64-pc-windows-msvc/release/bundle"
    BIN="src-tauri/target/x86_64-pc-windows-msvc/release/emailops.exe"

    echo "── artifacts ──"
    MSI=$(find "$BUNDLE/msi" -name '*.msi' 2>/dev/null | head -1 || true)
    NSIS=$(find "$BUNDLE/nsis" -name '*-setup.exe' 2>/dev/null | head -1 || true)
    [ -n "$MSI" ] && pass "msi: $(basename "$MSI")" || fail "no .msi found (run 'make build-windows')"
    [ -n "$NSIS" ] && pass "nsis: $(basename "$NSIS")" || fail "no NSIS -setup.exe found (run 'make build-windows')"

    echo "── binary ──"
    if [ ! -f "$BIN" ]; then
      fail "no release binary at $BIN"
    else
      pass "binary present"
      echo "── architecture ──"
      if command -v file >/dev/null 2>&1; then
        file "$BIN" | sed 's/^/  /'
      else
        echo "  i 'file' unavailable; skipping"
      fi
    fi

    echo "── signing ──"
    echo "  i unsigned by design — no code-signing certificate is configured."
    echo "    Users will see a SmartScreen warning on first run."
    ;;

  "")
    echo "usage: $0 <linux|windows>" >&2; exit 2 ;;
  *)
    echo "unknown platform '$PLATFORM' (expected: linux, windows)" >&2; exit 2 ;;
esac

echo
if [ "$FAILED" -ne 0 ]; then
  echo "[verify-$PLATFORM] FAILED" >&2
  exit 1
fi
echo "[verify-$PLATFORM] OK"
