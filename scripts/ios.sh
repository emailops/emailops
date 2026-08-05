#!/usr/bin/env bash
#
# iOS build driver — wraps `tauri ios <subcommand>` with the environment fixes
# this machine class needs, so the Makefile targets stay one-liners.
#
# Usage: scripts/ios.sh <init|dev|build|run> [extra tauri args...]
#
# Two things this exists to handle:
#
#  1. CocoaPods must be on PATH. Tauri shells out to `pod` when wiring the
#     Swift plugin sources into the generated Xcode project. Homebrew installs
#     it under the arm64 prefix (/opt/homebrew), which is not necessarily on
#     PATH first when an x86_64 Homebrew is also present.
#
#  2. RVM's gem environment must NOT leak in. If GEM_HOME/GEM_PATH point at an
#     RVM ruby, Homebrew's `pod` resolves its own dependencies against that
#     ruby's gem set, cannot find them, and dies with a Gem::MissingSpecError
#     ("Could not find 'base64'"). Unsetting the four RVM variables makes the
#     Homebrew ruby use its own bundled gems. This is why `pod --version` can
#     fail while `env -u GEM_HOME -u GEM_PATH pod --version` succeeds.
#
set -euo pipefail

SUBCOMMAND="${1:-}"
if [ -z "$SUBCOMMAND" ]; then
    echo "ERROR: no subcommand. Usage: scripts/ios.sh <init|dev|build|run> [args...]" >&2
    exit 2
fi
shift

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Prefer the arm64 Homebrew prefix, where `brew install cocoapods` puts `pod`.
if [ -d /opt/homebrew/bin ]; then
    PATH="/opt/homebrew/bin:$PATH"
    export PATH
fi

if ! command -v pod >/dev/null 2>&1; then
    echo "ERROR: cocoapods not found on PATH." >&2
    echo "       Install it with: /opt/homebrew/bin/brew install cocoapods" >&2
    exit 1
fi

# Drop the RVM gem environment for this process tree only — see note 2 above.
run_tauri() {
    env -u GEM_HOME -u GEM_PATH -u MY_RUBY_HOME -u IRBRC \
        npm run tauri ios "$SUBCOMMAND" -- "$@"
}

if [ "$SUBCOMMAND" = "init" ]; then
    # `init` rewrites gen/apple/project.yml from cargo-mobile2's template,
    # dropping the Accelerate framework and the iOS 26 deployment target. Both
    # are required to link and to match the recorded decision, so re-apply them
    # immediately rather than leaving a build that fails later with an opaque
    # "Undefined symbols: _vDSP_*" error.
    run_tauri "$@"
    exec env -u GEM_HOME -u GEM_PATH -u MY_RUBY_HOME -u IRBRC \
        bash "$REPO_ROOT/scripts/ios_patch_project.sh"
fi

run_tauri "$@"
