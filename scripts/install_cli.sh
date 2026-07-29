#!/usr/bin/env bash
# Build an optimized emailops-cli and symlink it onto the user's PATH.
#
# Unlike `build-cli-mac`, this does NOT sign/notarize — it's for local dev use
# only. Builds with the `cli` feature (embedded llama.cpp on, so chat/classify/
# embed work offline), then symlinks the binary into a PATH dir.
#
# Override the install dir with PREFIX, e.g.  PREFIX=~/.local/bin make install-cli
set -euo pipefail

MANIFEST="src-tauri/Cargo.toml"
BIN="src-tauri/target/release/emailops-cli"

# Pick an install dir: explicit PREFIX wins, else first writable candidate.
if [ -n "${PREFIX:-}" ]; then
  DEST_DIR="${PREFIX/#\~/$HOME}"
else
  DEST_DIR=""
  for d in /usr/local/bin "$HOME/.local/bin"; do
    if [ -d "$d" ] && [ -w "$d" ]; then DEST_DIR="$d"; break; fi
  done
  [ -z "$DEST_DIR" ] && DEST_DIR="$HOME/.local/bin"
fi
mkdir -p "$DEST_DIR"

echo "Building optimized emailops-cli (with embedded llama.cpp)…"
cargo build --release --manifest-path "$MANIFEST" --features cli --bin emailops-cli

# cargo emits the binary next to the manifest's target dir; resolve absolute.
ABS_BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
if [ ! -x "$ABS_BIN" ]; then
  echo "ERROR: expected binary not found at $ABS_BIN" >&2
  exit 1
fi

LINK="$DEST_DIR/emailops-cli"
ln -sf "$ABS_BIN" "$LINK"
echo "Symlinked $LINK -> $ABS_BIN"

# Point the user at the rc file their own login shell actually reads, rather
# than assuming ~/.zshrc (the macOS default but not Linux's).
shell_rc() {
  case "$(basename "${SHELL:-/bin/sh}")" in
    zsh)  echo "~/.zshrc" ;;
    bash) [ "$(uname -s)" = "Darwin" ] && echo "~/.bash_profile" || echo "~/.bashrc" ;;
    fish) echo "~/.config/fish/config.fish" ;;
    *)    echo "~/.profile" ;;
  esac
}

case ":$PATH:" in
  *":$DEST_DIR:"*) echo "Run: emailops-cli --version" ;;
  *) echo "NOTE: $DEST_DIR is not on your PATH. Add it, e.g.:"
     echo "  echo 'export PATH=\"$DEST_DIR:\$PATH\"' >> $(shell_rc)" ;;
esac
