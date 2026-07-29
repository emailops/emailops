#!/usr/bin/env bash
# Build the desktop bundle for Linux or Windows.
#
#   scripts/build_platform.sh linux
#   scripts/build_platform.sh windows
#   CARGO_FEATURES=cuda scripts/build_platform.sh linux
#
# Mirrors the `build-mac-intel` pattern: a per-platform overlay merged over
# src-tauri/tauri.conf.json via --config. The base config keeps macOS-only
# bundle targets (app, dmg) so the signed/notarized mac release path is
# byte-for-byte unaffected by this script's existence.
#
# No code signing. Linux packages are conventionally unsigned, and Windows
# signing needs an EV/OV certificate the project does not currently hold —
# unsigned Windows installers show a SmartScreen warning on first run.

set -euo pipefail

PLATFORM="${1:-}"
CARGO_FEATURES="${CARGO_FEATURES:-}"
# Set to 1 to build without embedded llama.cpp — skips the CMake/C++ toolchain.
# CI uses this to validate the bundle configuration quickly, leaving the slow
# llamacpp compile to a dedicated job.
NO_DEFAULT_FEATURES="${NO_DEFAULT_FEATURES:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

case "$PLATFORM" in
  linux)   CONFIG="src-tauri/tauri.linux.conf.json";   TARGET="x86_64-unknown-linux-gnu" ;;
  windows) CONFIG="src-tauri/tauri.windows.conf.json"; TARGET="x86_64-pc-windows-msvc" ;;
  "")      echo "usage: $0 <linux|windows>" >&2; exit 2 ;;
  *)       echo "unknown platform '$PLATFORM' (expected: linux, windows)" >&2; exit 2 ;;
esac

if [ ! -f "$CONFIG" ]; then
  echo "ERROR: missing $CONFIG" >&2
  exit 1
fi

# The bundled embedding GGUF must exist before tauri-build validates
# bundle.resources.
bash scripts/fetch_bundled_models.sh

# Cargo args are forwarded after the second `--`, exactly as build-mac-intel
# forwards `--no-default-features`.
CARGO_ARGS=()
if [ "$NO_DEFAULT_FEATURES" = "1" ]; then
  echo "[build-$PLATFORM] building WITHOUT embedded llama.cpp"
  CARGO_ARGS+=(--no-default-features)
fi
if [ -n "$CARGO_FEATURES" ]; then
  echo "[build-$PLATFORM] extra cargo features: $CARGO_FEATURES"
  CARGO_ARGS+=(--features "$CARGO_FEATURES")
fi

echo "[build-$PLATFORM] target=$TARGET config=$CONFIG"

if [ ${#CARGO_ARGS[@]} -gt 0 ]; then
  npm run tauri -- build --target "$TARGET" --config "$CONFIG" -- "${CARGO_ARGS[@]}"
else
  npm run tauri -- build --target "$TARGET" --config "$CONFIG"
fi

echo "[build-$PLATFORM] done — artifacts under src-tauri/target/$TARGET/release/bundle/"
