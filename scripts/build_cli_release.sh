#!/usr/bin/env bash
# Build a standalone, universal `emailops-cli` for distribution alongside the
# macOS desktop app.
#
# Same arches as `make build-mac` (aarch64 + x86_64, with the default `llamacpp`
# feature so terminal `chat` works fully offline on both slices), lipo'd into a
# single universal Mach-O. When APPLE_SIGNING_IDENTITY is set (the Makefile
# sources it from .env.signing) the binary is codesigned with the hardened
# runtime and the app's entitlements, matching how the .app itself is signed.
#
# When the full notarization env is also present (APPLE_ID / APPLE_PASSWORD /
# APPLE_TEAM_ID), the signed binary is wrapped in a .dmg, submitted to Apple's
# notary service, and the ticket is STAPLED to the .dmg. We staple a .dmg (not
# the bare binary) because `stapler` only accepts container formats — a stapled
# .dmg verifies offline, which matters for an offline-first tool. The .dmg is
# the distributable; the bare binary is left next to it for local use.

#
# macOS-only by nature: lipo, codesign, hdiutil and notarytool are Apple
# tooling with no cross-platform equivalent.
set -euo pipefail

. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib_data_dir.sh"
require_macos

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/src-tauri/Cargo.toml"
ENTITLEMENTS="$ROOT/src-tauri/entitlements.plist"

TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
FEATURES="${CLI_RELEASE_FEATURES:-cli}"

OUT_DIR="$ROOT/src-tauri/target/cli-release"
OUT_BIN="$OUT_DIR/emailops-cli"
OUT_DMG="$OUT_DIR/EmailOps-CLI.dmg"

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

# ── package + notarize + staple a .dmg ───────────────────────────────────────
# Only when the full notary credential set is available. notarytool requires the
# payload (the binary) to already be signed with the hardened runtime + secure
# timestamp, which the codesign step above provides.
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ] && \
   [ -n "${APPLE_ID:-}" ] && \
   [ -n "${APPLE_PASSWORD:-}" ] && \
   [ -n "${APPLE_TEAM_ID:-}" ]; then
  echo "[build-cli] packaging .dmg"
  STAGE="$(mktemp -d)"
  trap 'rm -rf "$STAGE"' EXIT
  cp "$OUT_BIN" "$STAGE/emailops-cli"
  rm -f "$OUT_DMG"
  hdiutil create -volname "EmailOps CLI" -srcfolder "$STAGE" \
    -ov -format UDZO "$OUT_DMG"

  echo "[build-cli] signing .dmg ($APPLE_SIGNING_IDENTITY)"
  codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$OUT_DMG"

  echo "[build-cli] notarizing .dmg (submitting to Apple — can take a few minutes)"
  xcrun notarytool submit "$OUT_DMG" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait

  echo "[build-cli] stapling notarization ticket to .dmg"
  xcrun stapler staple "$OUT_DMG"
  xcrun stapler validate "$OUT_DMG"

  echo "[build-cli] done → $OUT_DMG (notarized + stapled)"
else
  echo "[build-cli] notary env incomplete (need APPLE_SIGNING_IDENTITY + APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID) — skipping .dmg packaging"
fi

echo "[build-cli] done → $OUT_BIN"
lipo -info "$OUT_BIN" | sed 's/^/  /'
