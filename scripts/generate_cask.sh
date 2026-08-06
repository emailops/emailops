#!/usr/bin/env bash
# Generate homebrew/Casks/emailops.rb from a published GitHub release.
#
# Reads the release's asset list — including the sha256 digests GitHub
# computes for every asset — via the public API, so nothing is downloaded
# and no auth is required. Run AFTER the DMGs are uploaded to the release:
#
#   scripts/generate_cask.sh            # latest release
#   scripts/generate_cask.sh v0.6.2     # specific tag
#
# Emits a single-artifact cask: macOS ships ONE universal DMG
# (EmailOps-macos.dmg) that launches on both Apple Silicon and Intel, so the
# cask needs no `arch` stanza and no `depends_on arch:`. Embedded AI is
# refused on Intel at runtime rather than by shipping a second build — see
# the `bootstrap-mac` comment in the Makefile.
#
# The `depends_on macos: ">= :monterey"` string form is deliberate: older
# Homebrew treats the bare-symbol form (`macos: :monterey`) as an EXACT
# version match and refuses to install on anything newer ("does not run on
# macOS versions other than Monterey"), while only recent Homebrew reads it
# as a minimum. The string form works on both, so `brew style` reporting one
# Homebrew/OSDependsOn offense on that line is EXPECTED — do not "fix" it
# (inline rubocop:disable comments and tap-local .rubocop.yml are both
# rejected/ignored by brew style, so it cannot be silenced).
#
# Publishing flow lives in homebrew/README.md.
#
# macOS-only by nature: Homebrew casks describe macOS .app installs.
set -euo pipefail

. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib_data_dir.sh"
require_macos

REPO="emailops/emailops"
TAG="${1:-}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$REPO_ROOT/homebrew/Casks"
OUT="$OUT_DIR/emailops.rb"

if [ -n "$TAG" ]; then
  API_URL="https://api.github.com/repos/$REPO/releases/tags/$TAG"
else
  API_URL="https://api.github.com/repos/$REPO/releases/latest"
fi

JSON="$(curl -fsSL -H "Accept: application/vnd.github+json" "$API_URL")" || {
  echo "ERROR: failed to fetch release metadata from $API_URL" >&2
  exit 1
}

# Extract tag + per-asset sha256 digests as shell assignments.
eval "$(printf '%s' "$JSON" | python3 -c "
import json, sys

release = json.load(sys.stdin)
assets = {a['name']: a.get('digest') or '' for a in release.get('assets', [])}

def sha(name):
    digest = assets.get(name, '')
    return digest.split(':', 1)[1] if digest.startswith('sha256:') else ''

print(f\"TAG_NAME='{release['tag_name']}'\")
print(f\"DMG_SHA='{sha('EmailOps-macos.dmg')}'\")
")"

VERSION="${TAG_NAME#v}"

if [ -z "$DMG_SHA" ]; then
  echo "ERROR: EmailOps-macos.dmg not found on release $TAG_NAME (or GitHub has not computed its digest yet)." >&2
  echo "       Upload release/EmailOps-macos.dmg to the release, wait a minute, and re-run." >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

cat > "$OUT" <<EOF
cask "emailops" do
  version "$VERSION"
  sha256 "$DMG_SHA"

  url "https://github.com/emailops/emailops/releases/download/v#{version}/EmailOps-macos.dmg"
  name "EmailOps"
  desc "Privacy-first, AI-native email client"
  homepage "https://github.com/emailops/emailops"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :monterey"

  app "EmailOps.app"

  zap trash: [
    "~/Library/Application Support/com.emailops.app",
    "~/Library/Caches/com.emailops.app",
    "~/Library/HTTPStorages/com.emailops.app",
    "~/Library/Preferences/com.emailops.app.plist",
    "~/Library/Saved Application State/com.emailops.app.savedState",
    "~/Library/WebKit/com.emailops.app",
  ]
end
EOF

echo "Wrote $OUT (version $VERSION, tag $TAG_NAME)"
