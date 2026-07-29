#!/usr/bin/env bash
# Stage Linux or Windows artifacts under release/ with stable, versionless
# names.
#
#   scripts/dist_platform.sh linux
#   scripts/dist_platform.sh windows
#
# Tauri embeds the version in every bundle filename, which breaks permanent
# download links. The names below match `dist-mac`'s convention so every
# platform is reachable at
#   https://github.com/emailops/emailops/releases/latest/download/<name>

set -euo pipefail

PLATFORM="${1:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

mkdir -p release

# stage <source-glob-dir> <extension> <destination-name> <required>
stage() {
  local dir="$1" ext="$2" dest="$3" required="$4"
  local src
  src=$(find "$dir" -name "*.$ext" 2>/dev/null | sort | tail -1 || true)
  if [ -z "$src" ]; then
    if [ "$required" = "required" ]; then
      echo "ERROR: no *.$ext found under $dir. Run the matching build target first." >&2
      return 1
    fi
    echo "  · skipped $dest (no *.$ext present)"
    return 0
  fi
  cp "$src" "release/$dest"
  echo "  Staged release/$dest (from $(basename "$src"))"
}

case "$PLATFORM" in
  linux)
    BUNDLE="src-tauri/target/x86_64-unknown-linux-gnu/release/bundle"
    stage "$BUNDLE/appimage" AppImage EmailOps-linux.AppImage required
    stage "$BUNDLE/deb"      deb      EmailOps-linux.deb      required
    stage "$BUNDLE/rpm"      rpm      EmailOps-linux.rpm      optional
    ;;
  windows)
    BUNDLE="src-tauri/target/x86_64-pc-windows-msvc/release/bundle"
    stage "$BUNDLE/msi"  msi EmailOps-windows.msi        required
    # Tauri names the NSIS output `<product>_<version>_<arch>-setup.exe`.
    stage "$BUNDLE/nsis" exe EmailOps-windows-setup.exe  required
    ;;
  "")
    echo "usage: $0 <linux|windows>" >&2; exit 2 ;;
  *)
    echo "unknown platform '$PLATFORM' (expected: linux, windows)" >&2; exit 2 ;;
esac
