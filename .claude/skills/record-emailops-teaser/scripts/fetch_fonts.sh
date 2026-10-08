#!/usr/bin/env bash
# Fetch Inter (SIL OFL) into the cache teaser_fx.py reads from. Idempotent.
set -euo pipefail
DEST="${TEASER_FONTS:-$HOME/.cache/emailops-teaser/fonts}"
if [[ -f "$DEST/InterDisplay-SemiBold.ttf" && -f "$DEST/Inter-Medium.ttf" && -f "$DEST/Inter-Regular.ttf" ]]; then
  echo "fonts ready: $DEST"; exit 0
fi
mkdir -p "$DEST"
TMP="$(mktemp -d)"
curl -sfL -o "$TMP/inter.zip" https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip
unzip -q -o "$TMP/inter.zip" -d "$TMP/inter"
for f in InterDisplay-SemiBold Inter-Medium Inter-Regular; do
  cp "$TMP/inter/extras/ttf/$f.ttf" "$DEST/"
done
cp "$TMP/inter/LICENSE.txt" "$DEST/LICENSE-Inter.txt"
echo "fonts ready: $DEST"
