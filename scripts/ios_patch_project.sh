#!/usr/bin/env bash
#
# Re-apply EmailOps' required edits to the generated Xcode project.
#
# `tauri ios init` regenerates `src-tauri/gen/apple/project.yml` from
# cargo-mobile2's template, discarding anything we changed. Two edits are load
# bearing and must survive every regeneration, so they live here rather than as
# manual steps someone will forget:
#
#  1. Accelerate.framework. ggml's CPU ops reference vDSP_* symbols and
#     llama-cpp-sys-2 links Accelerate via `cargo:rustc-link-lib`. Cargo only
#     builds the staticlib — Xcode does the final link and never sees cargo's
#     link directives — so without this the build fails with
#     "Undefined symbols for architecture arm64: _vDSP_maxv, _vDSP_sve, ...".
#     It is not in cargo-mobile2's stock framework list.
#
#  2. Deployment target 26.0 (docs/DECISIONS.md, 2026-08-05). The template pins
#     14.0 and Tauri's `bundle.iOS.minimumSystemVersion` does NOT drive it —
#     verified both as a tauri.ios.conf.json overlay and in the base config.
#     Leaving it at 14.0 also emits ~183 "object file was built for newer
#     iOS-simulator version (26.0)" warnings, since rustc targets 26 regardless.
#
# Idempotent: safe to run repeatedly. Regenerates the .xcodeproj at the end.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT_YML="$REPO_ROOT/src-tauri/gen/apple/project.yml"

if [ ! -f "$PROJECT_YML" ]; then
    echo "ERROR: $PROJECT_YML not found. Run 'make ios-init' first." >&2
    exit 1
fi

changed=0

if ! grep -q 'Accelerate.framework' "$PROJECT_YML"; then
    # Insert directly above the first stock framework so the list stays grouped.
    perl -i -pe 's{^(\s*)- sdk: CoreGraphics\.framework$}{$1# Required by ggml CPU ops (vDSP_*); see scripts/ios_patch_project.sh\n$1- sdk: Accelerate.framework\n$1- sdk: CoreGraphics.framework}' "$PROJECT_YML"
    echo "patched: added Accelerate.framework"
    changed=1
fi

if grep -qE '^\s+iOS: 14\.0$' "$PROJECT_YML"; then
    perl -i -pe 's{^(\s*)iOS: 14\.0$}{$1iOS: 26.0}' "$PROJECT_YML"
    echo "patched: deployment target 14.0 -> 26.0"
    changed=1
fi

if [ "$changed" -eq 0 ]; then
    echo "project.yml already patched; nothing to do"
fi

# Verify both invariants actually hold before regenerating, so a template
# change upstream fails loudly here instead of as a confusing linker error.
grep -q 'Accelerate.framework' "$PROJECT_YML" || {
    echo "ERROR: Accelerate.framework still missing after patch — cargo-mobile2's template likely changed." >&2
    exit 1
}
grep -qE '^\s+iOS: 26\.0$' "$PROJECT_YML" || {
    echo "ERROR: deployment target is not 26.0 after patch — cargo-mobile2's template likely changed." >&2
    exit 1
}

cd "$REPO_ROOT/src-tauri/gen/apple"
exec xcodegen generate
