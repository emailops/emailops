#!/usr/bin/env bash
# Stage Linux or Windows artifacts under release/ with stable, versionless
# names.
#
#   scripts/dist_platform.sh linux
#   scripts/dist_platform.sh windows
#   scripts/dist_platform.sh windows cuda   # -> EmailOps-windows-cuda.msi etc.
#
# Tauri embeds the version in every bundle filename, which breaks permanent
# download links. The names below match `dist-mac`'s convention so every
# platform is reachable at
#   https://github.com/emailops/emailops/releases/latest/download/<name>
#
# The optional second arg is a variant suffix, needed when a platform ships
# more than one downloadable build of the same installer type (currently:
# Windows Vulkan, the default GPU-agnostic build, alongside Windows CUDA,
# NVIDIA-only and faster to build but not the recommended default) — without
# it, both variants would stage to the same filename and the second `cp`
# would silently overwrite the first in the same release.

set -euo pipefail

PLATFORM="${1:-}"
VARIANT="${2:-}"
SUFFIX=""
if [ -n "$VARIANT" ]; then
  SUFFIX="-$VARIANT"
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

mkdir -p release

# Must match build_platform.sh's CARGO_TARGET_DIR override for Windows (see
# that script for why): this runs as a separate CI step, so it can't inherit
# an `export` from the Build step's own shell — it has to compute the same
# value independently.
if [ "$PLATFORM" = "windows" ]; then
  export CARGO_TARGET_DIR="C:/ct"
fi
TARGET_DIR="${CARGO_TARGET_DIR:-src-tauri/target}"

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
    BUNDLE="$TARGET_DIR/x86_64-unknown-linux-gnu/release/bundle"
    stage "$BUNDLE/appimage" AppImage EmailOps-linux.AppImage required
    stage "$BUNDLE/deb"      deb      EmailOps-linux.deb      required
    stage "$BUNDLE/rpm"      rpm      EmailOps-linux.rpm      optional
    ;;
  windows)
    BUNDLE="$TARGET_DIR/x86_64-pc-windows-msvc/release/bundle"
    stage "$BUNDLE/msi"  msi "EmailOps-windows${SUFFIX}.msi"        required
    # Tauri names the NSIS output `<product>_<version>_<arch>-setup.exe`.
    stage "$BUNDLE/nsis" exe "EmailOps-windows${SUFFIX}-setup.exe"  required
    ;;
  "")
    echo "usage: $0 <linux|windows>" >&2; exit 2 ;;
  *)
    echo "unknown platform '$PLATFORM' (expected: linux, windows)" >&2; exit 2 ;;
esac
