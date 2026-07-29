#!/usr/bin/env bash
# Fetch bundled model artifacts that are packaged into the app (all platforms).
#
# Defaults fetch the Nomic embedding GGUF used for first-run semantic search.
# Environment overrides are intentionally supported so the script can be tested
# against tiny local fixtures without touching the network.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

MODEL_PATH="${BUNDLED_MODEL_PATH:-$ROOT/src-tauri/resources/models/nomic-embed-text-v1.5-q4_k_m.gguf}"
MODEL_URL="${BUNDLED_MODEL_URL:-https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF/resolve/main/nomic-embed-text-v1.5.Q4_K_M.gguf}"
MODEL_SHA256="${BUNDLED_MODEL_SHA256:-d4e388894e09cf3816e8b0896d81d265b55e7a9fff9ab03fe8bf4ef5e11295ac}"

MODEL_NAME="$(basename "$MODEL_PATH")"
PARTIAL_PATH="$MODEL_PATH.partial"

# Portable SHA-256. `shasum` is a Perl script that ships with macOS and
# Git-for-Windows but is absent from a stock Linux install, where `sha256sum`
# (coreutils) is the one that is always there. This target is a prerequisite of
# `make dev`, so hardcoding `shasum` made a fresh Linux checkout unable to run
# the app at all.
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$1" | awk '{print $NF}'
  else
    echo "ERROR: no SHA-256 tool available (install coreutils, perl, or openssl)" >&2
    return 1
  fi
}

if [ -f "$MODEL_PATH" ]; then
  echo "[fetch-bundled-models] already present: $MODEL_PATH"
  exit 0
fi

mkdir -p "$(dirname "$MODEL_PATH")"

echo "[fetch-bundled-models] downloading $MODEL_NAME"
rm -f "$PARTIAL_PATH"
curl -L --fail --progress-bar -o "$PARTIAL_PATH" "$MODEL_URL"

ACTUAL="$(sha256_file "$PARTIAL_PATH")"
if [ "$ACTUAL" != "$MODEL_SHA256" ]; then
  echo "ERROR: SHA-256 mismatch for $MODEL_NAME: expected $MODEL_SHA256, got $ACTUAL" >&2
  rm -f "$PARTIAL_PATH"
  exit 1
fi

mv "$PARTIAL_PATH" "$MODEL_PATH"
echo "[fetch-bundled-models] verified $MODEL_NAME"
