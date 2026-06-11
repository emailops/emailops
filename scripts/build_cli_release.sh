#!/usr/bin/env bash
# Build a standalone, universal `emailops-cli` for distribution alongside the
# macOS desktop app.
#
# Same arches as `make build-mac` (aarch64 + x86_64, with the default `llamacpp`
# feature so terminal `chat` works fully offline on both slices), lipo'd into a
# single universal Mach-O. When APPLE_SIGNING_IDENTITY is set (the Makefile
# sources it from .env.signing) the binary is codesigned with the hardened
# runtime and the app's entitlements, matching how the .app itself is signed.
# Notarization of the standalone binary is intentionally left out — the
# recommended long-term path is to ship the CLI as a Tauri `externalBin`
# sidecar so the .app's own notarization covers it.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/src-tauri/Cargo.toml"
ENTITLEMENTS="$ROOT/src-tauri/entitlements.plist"

TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
FEATURES="${CLI_RELEASE_FEATURES:-cli}"

OUT_DIR="$ROOT/src-tauri/target/cli-release"
OUT_BIN="$OUT_DIR/emailops-cli"

mkdir -p "$OUT_DIR"

SLICES=()
for target in "${TARGETS[@]}"; do
  echo "[build-cli] compiling emailops-cli for $target (features: $FEATURES)"
  cargo build --release \
    --manifest-path "$MANIFEST" \
    --features "$FEATURES" \
    --bin emailops-cli \
    --target "$target"
  SLICES+=("$ROOT/src-tauri/target/$target/release/emailops-cli")
done

echo "[build-cli] lipo → universal binary"
lipo -create -output "$OUT_BIN" "${SLICES[@]}"

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  echo "[build-cli] codesigning with hardened runtime ($APPLE_SIGNING_IDENTITY)"
  codesign --force --options runtime --timestamp \
    ${ENTITLEMENTS:+--entitlements "$ENTITLEMENTS"} \
    --sign "$APPLE_SIGNING_IDENTITY" \
    "$OUT_BIN"
else
  echo "[build-cli] APPLE_SIGNING_IDENTITY not set — leaving binary UNSIGNED (fine for local use)"
fi

echo "[build-cli] done → $OUT_BIN"
lipo -info "$OUT_BIN" | sed 's/^/  /'
