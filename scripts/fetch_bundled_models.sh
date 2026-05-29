#!/usr/bin/env bash
# Fetch bundled model artifacts that are packaged into the macOS app.
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

sha256_file() {
  shasum -a 256 "$1" | awk '{print $1}'
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
