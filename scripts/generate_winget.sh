#!/usr/bin/env bash
# Generate the winget manifests for a published GitHub release, into
# packaging/winget/<version>/.
#
# Mirrors scripts/generate_cask.sh: reads the release's asset list — including
# the sha256 digests GitHub computes for every asset — via the public API, so
# nothing is downloaded and no auth is required. Run AFTER the Windows
# installer is uploaded to the release:
#
#   scripts/generate_winget.sh            # latest release
#   scripts/generate_winget.sh v0.6.6     # specific tag
#
# Why the NSIS installer and not the MSI: winget wants one installer per
# architecture, the NSIS build is the one the README recommends (Vulkan GPU
# acceleration with a CPU fallback), and Tauri's NSIS output supports the
# silent `/S` switch winget requires. The CUDA build is deliberately absent —
# it is an NVIDIA-only alternative, not a second architecture.
#
# The installer is NOT code-signed, which winget allows: the manifest pins the
# sha256 and winget verifies it before running anything. Submission is a pull
# request to microsoft/winget-pkgs — see packaging/winget/README.md.

set -euo pipefail

REPO="emailops/emailops"
# Overridable so the generator can be rehearsed against a stub or a saved
# payload without hitting the real API — same seam as METRICS_API.
API="${WINGET_API:-https://api.github.com}"
TAG="${1:-}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PACKAGE_ID="EmailOps.EmailOps"
ASSET="EmailOps-windows-setup.exe"

if [ -n "$TAG" ]; then
  API_URL="$API/repos/$REPO/releases/tags/$TAG"
else
  API_URL="$API/repos/$REPO/releases/latest"
fi

JSON="$(curl -fsSL -H "Accept: application/vnd.github+json" "$API_URL")" || {
  echo "ERROR: failed to fetch release metadata from $API_URL" >&2
  exit 1
}

eval "$(printf '%s' "$JSON" | python3 -c "
import json, sys

release = json.load(sys.stdin)
assets = {a['name']: a.get('digest') or '' for a in release.get('assets', [])}
digest = assets.get('$ASSET', '')
sha = digest.split(':', 1)[1] if digest.startswith('sha256:') else ''

print(f\"TAG_NAME='{release['tag_name']}'\")
print(f\"EXE_SHA='{sha.upper()}'\")
print(f\"PUBLISHED='{release.get('published_at', '')[:10]}'\")
")"

VERSION="${TAG_NAME#v}"

if [ -z "$EXE_SHA" ]; then
  echo "ERROR: $ASSET not found on release $TAG_NAME (or GitHub has not computed its digest yet)." >&2
  echo "       Upload it to the release, wait a minute, and re-run." >&2
  exit 1
fi

OUT_DIR="$REPO_ROOT/packaging/winget/$VERSION"
mkdir -p "$OUT_DIR"

# Three files, as the winget schema requires: a version manifest naming the
# other two, an installer manifest, and a locale manifest with the store copy.
cat > "$OUT_DIR/$PACKAGE_ID.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: $PACKAGE_ID
PackageVersion: $VERSION
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
EOF

cat > "$OUT_DIR/$PACKAGE_ID.installer.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: $PACKAGE_ID
PackageVersion: $VERSION
InstallerType: nullsoft
Scope: user
InstallModes:
  - interactive
  - silent
ReleaseDate: $PUBLISHED
Installers:
  - Architecture: x64
    InstallerUrl: https://github.com/$REPO/releases/download/$TAG_NAME/$ASSET
    InstallerSha256: $EXE_SHA
ManifestType: installer
ManifestVersion: 1.6.0
EOF

cat > "$OUT_DIR/$PACKAGE_ID.locale.en-US.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: $PACKAGE_ID
PackageVersion: $VERSION
PackageLocale: en-US
Publisher: EmailOps
PublisherUrl: https://github.com/emailops
PublisherSupportUrl: https://github.com/$REPO/issues
PackageName: EmailOps
PackageUrl: https://github.com/$REPO
License: Apache-2.0
LicenseUrl: https://github.com/$REPO/blob/main/LICENSE
ShortDescription: Privacy-first email client whose AI runs on your machine
Description: >-
  EmailOps is a desktop email client for Gmail, Outlook and IMAP accounts whose
  AI features run on device through an embedded llama.cpp runtime: chat over
  your inbox, draft generation and classification never send message content to
  a third-party model. Mail, attachments and the search index live in a local
  SQLite database, OAuth tokens go in the OS credential store, and the app
  ships no telemetry. Ollama and OpenRouter are opt-in alternatives.
Moniker: emailops
Tags:
  - email
  - email-client
  - privacy
  - local-ai
  - llm
  - offline
ReleaseNotesUrl: https://github.com/$REPO/releases/tag/$TAG_NAME
ManifestType: defaultLocale
ManifestVersion: 1.6.0
EOF

echo "[winget] wrote manifests for $TAG_NAME to packaging/winget/$VERSION/"
echo "[winget] validate with:  winget validate --manifest packaging/winget/$VERSION"
echo "[winget] then submit per packaging/winget/README.md"
