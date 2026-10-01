#!/usr/bin/env bash
# Notarize and staple the universal DMG that `make build-mac` produced.
#
# Tauri notarizes and staples the .app inside the DMG, but leaves the DMG
# itself signed and unnotarized, so Gatekeeper rejects the download as a
# container ("source=Unnotarized Developer ID") even though the app it holds
# is fine. Notarizing the DMG too makes the whole download verify, offline
# included once the ticket is stapled (CASA/DASA 3.2.1). Same flow as the CLI
# DMG in build_cli_release.sh.
#
# Needs APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID in
# the environment (the Makefile sources them from .env.signing).
set -euo pipefail

. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib_data_dir.sh"
require_macos

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG="$(ls -t "$ROOT"/src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg 2>/dev/null | head -1 || true)"
if [ -z "$DMG" ]; then
  echo "[notarize-dmg] ERROR: no universal .dmg found. Run 'make build-mac' first." >&2
  exit 1
fi

for var in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  if [ -z "${!var:-}" ]; then
    echo "[notarize-dmg] ERROR: $var is not set (source .env.signing)" >&2
    exit 1
  fi
done

# Tauri signs the DMG already; re-sign so a DMG built without that step is
# still accepted by the notary service (it rejects unsigned containers).
echo "[notarize-dmg] signing $(basename "$DMG") ($APPLE_SIGNING_IDENTITY)"
codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG"

echo "[notarize-dmg] notarizing (submitting to Apple — can take a few minutes)"
xcrun notarytool submit "$DMG" \
  --apple-id "$APPLE_ID" \
  --password "$APPLE_PASSWORD" \
  --team-id "$APPLE_TEAM_ID" \
  --wait

echo "[notarize-dmg] stapling ticket"
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
echo "[notarize-dmg] done → $DMG (notarized + stapled)"
