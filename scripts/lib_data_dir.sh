#!/usr/bin/env bash
# Shared helper: resolve EmailOps' app data directory for the current platform.
#
# Source it, don't execute it:
#   . "$(dirname "${BASH_SOURCE[0]}")/lib_data_dir.sh"
#   DB="$(emailops_data_dir)/emailops.db"
#
# The values must agree with what the app itself resolves — Tauri's
# `app_data_dir()` in src-tauri/src/lib.rs and `default_data_dir()` in
# src-tauri/src/cli/session.rs. Scripts used to hardcode the macOS path, which
# made every one of them silently point at a non-existent file on Linux.

# App identifier, matching `identifier` in src-tauri/tauri.conf.json.
EMAILOPS_APP_ID="com.emailops.app"

emailops_data_dir() {
  # An explicit override always wins — this is what `make dev` sets.
  if [ -n "${EMAILOPS_DATA_DIR:-}" ]; then
    printf '%s\n' "$EMAILOPS_DATA_DIR"
    return 0
  fi

  case "$(uname -s)" in
    Darwin)
      printf '%s\n' "$HOME/Library/Application Support/$EMAILOPS_APP_ID"
      ;;
    # Git Bash / MSYS2 / Cygwin on Windows. Tauri resolves %APPDATA% (Roaming).
    MINGW* | MSYS* | CYGWIN*)
      printf '%s\n' "${APPDATA:-$HOME/AppData/Roaming}/$EMAILOPS_APP_ID"
      ;;
    # Linux and other XDG platforms.
    *)
      printf '%s\n' "${XDG_DATA_HOME:-$HOME/.local/share}/$EMAILOPS_APP_ID"
      ;;
  esac
}

# Exit with a helpful message when the script is macOS-only by nature (Apple
# codesigning, notarization, Homebrew, /Applications).
require_macos() {
  if [ "$(uname -s)" != "Darwin" ]; then
    echo "ERROR: $(basename "${BASH_SOURCE[1]:-$0}") only works on macOS —" >&2
    echo "       it uses Apple-only tooling (codesign / notarytool / hdiutil / Homebrew)." >&2
    exit 1
  fi
}
