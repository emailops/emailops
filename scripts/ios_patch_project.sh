#!/usr/bin/env bash
#
# Re-apply EmailOps' required edits to the generated Xcode project.
#
# `tauri ios init` regenerates everything under `src-tauri/gen/apple/` from
# cargo-mobile2's templates, discarding anything we changed. Every edit that
# must survive that lives here rather than as a manual step someone will forget.
# Run it after `ios-init` (the Makefile target does) and any time the generated
# project looks wrong.
#
# What it applies, and why each one is load bearing:
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
#  3. CFBundleURLTypes. iOS only routes a callback URL to the app if its scheme
#     is declared in Info.plist; declaring it in tauri.conf.json alone merely
#     embeds it in the binary. Without this, OAuth completes in Safari and never
#     returns — silently, which is why it belongs in a script with a check.
#
#  4. `Externals` out of the resources phase. xcodegen copies unrecognised
#     files in a `sources` group into the bundle, so the 336 MB `libapp.a` that
#     Xcode links was ALSO being copied into EmailOps.app (508 MB total). A
#     static archive inside the bundle is a rejected upload, not just bloat.
#
#  5. ITSAppUsesNonExemptEncryption = false. The app uses only TLS and the
#     system keychain — exempt. Without the key every upload stops to ask.
#
#  6. The increased-memory-limit entitlement. Sizing the local model against
#     `os_proc_available_memory()` (util/system.rs) only pays off if the limit
#     has actually been raised; without it a 3 GB model is jetsammed on a
#     device that could otherwise hold it.
#
#  7. The app icons. `ios init` installs cargo-mobile2's placeholder rings;
#     scripts/ios_icons.sh copies the real ones in and strips the alpha channel
#     the App Store rejects. See that script for the details.
#
#  8. PrivacyInfo.xcprivacy, copied from src-tauri/ios/. Apple rejects uploads
#     that use required-reason APIs without a manifest (ITMS-91053).
#
#  9. ExportOptions.plist method `app-store-connect`. The template ships
#     `debugging`, which produces an IPA that App Store Connect refuses.
#     `IOS_EXPORT_METHOD=development` (scripts/ios.sh) overrides it for a build
#     you intend to install on your own device.
#
# Idempotent: safe to run repeatedly. Regenerates the .xcodeproj at the end.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APPLE_DIR="$REPO_ROOT/src-tauri/gen/apple"
PROJECT_YML="$APPLE_DIR/project.yml"
ENTITLEMENTS="$APPLE_DIR/emailops_iOS/emailops_iOS.entitlements"
EXPORT_OPTIONS="$APPLE_DIR/ExportOptions.plist"
PRIVACY_SRC="$REPO_ROOT/src-tauri/ios/PrivacyInfo.xcprivacy"
PRIVACY_DEST="$APPLE_DIR/emailops_iOS/PrivacyInfo.xcprivacy"
BGREFRESH_SRC="$REPO_ROOT/src-tauri/ios/EmailOpsBackgroundRefresh.m"
FOUNDATION_SRC="$REPO_ROOT/src-tauri/ios/EmailOpsFoundationModels.swift"
FOUNDATION_DEST="$APPLE_DIR/Sources/emailops/EmailOpsFoundationModels.swift"
AIBRIDGE_SRC="$REPO_ROOT/src-tauri/ios/EmailOpsAiBridge.m"
AIBRIDGE_DEST="$APPLE_DIR/Sources/emailops/EmailOpsAiBridge.m"
LOGBRIDGE_SRC="$REPO_ROOT/src-tauri/ios/EmailOpsLogBridge.m"
LOGBRIDGE_DEST="$APPLE_DIR/Sources/emailops/EmailOpsLogBridge.m"
BGREFRESH_DEST="$APPLE_DIR/Sources/emailops/EmailOpsBackgroundRefresh.m"

if [ ! -f "$PROJECT_YML" ]; then
    echo "ERROR: $PROJECT_YML not found. Run 'make ios-init' first." >&2
    exit 1
fi

changed=0
note() {
    echo "patched: $1"
    changed=1
}

# ── project.yml ──────────────────────────────────────────────────────────────

if ! grep -q 'Accelerate.framework' "$PROJECT_YML"; then
    # Insert directly above the first stock framework so the list stays grouped.
    perl -i -pe 's{^(\s*)- sdk: CoreGraphics\.framework$}{$1# Required by ggml CPU ops (vDSP_*); see scripts/ios_patch_project.sh\n$1- sdk: Accelerate.framework\n$1- sdk: CoreGraphics.framework}' "$PROJECT_YML"
    note "added Accelerate.framework"
fi

if grep -qE '^\s+iOS: 14\.0$' "$PROJECT_YML"; then
    perl -i -pe 's{^(\s*)iOS: 14\.0$}{$1iOS: 26.0}' "$PROJECT_YML"
    note "deployment target 14.0 -> 26.0"
fi

# `- path: Externals` with no buildPhase lands in Copy Bundle Resources. The
# guard tests for the *result*, not the anchor — the anchor line survives the
# edit, so testing it would re-insert the block on every run.
if ! grep -qE '^\s+buildPhase: none$' "$PROJECT_YML"; then
    perl -i -pe 's{^(\s*)- path: Externals$}{$1- path: Externals\n$1  # Linked via LIBRARY_SEARCH_PATHS, never copied: libapp.a is 336 MB and a\n$1  # static archive inside the .app is a rejected upload.\n$1  buildPhase: none}' "$PROJECT_YML"
    note "Externals excluded from the resources phase"
fi

if ! grep -q 'CFBundleURLTypes' "$PROJECT_YML"; then
    # Anchored on a stock Info.plist property so the indentation is right
    # whatever else the template emits.
    perl -i -pe 's{^(\s*)LSRequiresIPhoneOS: true$}{$1LSRequiresIPhoneOS: true\n$1# OAuth callback schemes. iOS only hands a URL to the app if its scheme\n$1# is declared here — declaring it in tauri.conf.json alone merely embeds\n$1# it in the binary, which is not what the OS consults when routing.\n$1CFBundleURLTypes:\n$1  # Google: the reversed client ID (see sync/oauth.rs::reversed_client_id).\n$1  - CFBundleURLName: com.emailops.app.oauth.google\n$1    CFBundleURLSchemes:\n$1      - com.googleusercontent.apps.60095465878-nvq0qdolh6qj953b2ii4ec6mqn4vqokd\n$1  # Microsoft: the app'"'"'s own scheme, matching the redirect URI registered\n$1  # under "Mobile and desktop applications" in the Azure app registration.\n$1  - CFBundleURLName: com.emailops.app.oauth\n$1    CFBundleURLSchemes:\n$1      - com.emailops.app}' "$PROJECT_YML"
    note "added CFBundleURLTypes (OAuth callback schemes)"
fi

if ! grep -q 'BGTaskSchedulerPermittedIdentifiers' "$PROJECT_YML"; then
    perl -i -pe 's{^(\s*)LSRequiresIPhoneOS: true$}{$1LSRequiresIPhoneOS: true\n$1# Background refresh. The identifier must be declared here or the system\n$1# rejects the request; the handler is registered in\n$1# Sources/emailops/EmailOpsBackgroundRefresh.m.\n$1UIBackgroundModes:\n$1  - fetch\n$1BGTaskSchedulerPermittedIdentifiers:\n$1  - com.emailops.app.refresh}' "$PROJECT_YML"
    note "added the background-refresh task identifier + fetch mode"
fi

if ! grep -q 'SWIFT_VERSION' "$PROJECT_YML"; then
    perl -i -pe 's{^(\s*)ENABLE_BITCODE: false$}{$1ENABLE_BITCODE: false\n$1# The target is otherwise pure ObjC/C++. FoundationModels is a Swift-only\n$1# framework, so reaching it needs a Swift file, which needs a language\n$1# version -- without this the compiler refuses the file outright.\n$1SWIFT_VERSION: 5.0}' "$PROJECT_YML"
    note "added SWIFT_VERSION for the Foundation Models shim"
fi

if ! grep -q 'FoundationModels.framework' "$PROJECT_YML"; then
    # Weak: the app deploys to iOS 26 where the framework exists, but linking
    # it strongly makes a device that cannot load it fail at launch rather than
    # report "unavailable" through the probe.
    perl -i -pe 's{^(\s*)- sdk: CoreGraphics\.framework$}{$1# Apple on-device model. Weak-linked; see EmailOpsFoundationModels.swift.\n$1- sdk: FoundationModels.framework\n$1  weak: true\n$1- sdk: CoreGraphics.framework}' "$PROJECT_YML"
    note "added FoundationModels.framework (weak)"
fi

if ! grep -q 'BackgroundTasks.framework' "$PROJECT_YML"; then
    perl -i -pe 's{^(\s*)- sdk: CoreGraphics\.framework$}{$1# BGTaskScheduler, for background refresh.\n$1- sdk: BackgroundTasks.framework\n$1- sdk: CoreGraphics.framework}' "$PROJECT_YML"
    note "added BackgroundTasks.framework"
fi

if ! grep -q 'ITSAppUsesNonExemptEncryption' "$PROJECT_YML"; then
    perl -i -pe 's{^(\s*)LSRequiresIPhoneOS: true$}{$1LSRequiresIPhoneOS: true\n$1# TLS and the system keychain only — exempt from export compliance.\n$1ITSAppUsesNonExemptEncryption: false}' "$PROJECT_YML"
    note "added ITSAppUsesNonExemptEncryption"
fi

# ── entitlements ─────────────────────────────────────────────────────────────
#
# Declared in project.yml, NOT by editing the .entitlements file: xcodegen
# rewrites that file from this block on every `generate`, so a PlistBuddy edit
# to it survives exactly until the next run of this script.

if ! grep -q 'increased-memory-limit' "$PROJECT_YML"; then
    perl -i -pe 's{^(\s*)path: emailops_iOS/emailops_iOS\.entitlements$}{$1path: emailops_iOS/emailops_iOS.entitlements\n$1properties:\n$1  # Raises the jetsam ceiling so a 3-4B Q4 model can actually be held.\n$1  # util/system.rs sizes the runtime from os_proc_available_memory(),\n$1  # which already reflects whether this entitlement applied.\n$1  com.apple.developer.kernel.increased-memory-limit: true}' "$PROJECT_YML"
    note "added the increased-memory-limit entitlement"
fi

# ── privacy manifest ─────────────────────────────────────────────────────────

if [ ! -f "$PRIVACY_SRC" ]; then
    echo "ERROR: $PRIVACY_SRC not found — the privacy manifest is tracked outside gen/apple." >&2
    exit 1
fi
# Lands in `emailops_iOS/`, which is already a `sources` path, so xcodegen
# copies it to the bundle root without any further project.yml change.
if ! cmp -s "$PRIVACY_SRC" "$PRIVACY_DEST"; then
    cp "$PRIVACY_SRC" "$PRIVACY_DEST"
    note "copied PrivacyInfo.xcprivacy into the generated project"
fi

# ── background refresh ───────────────────────────────────────────────────────
#
# Lands in `Sources/`, already a source group, so xcodegen compiles it without
# any further project.yml change. It registers the BGTaskScheduler handler from
# `+load` — see the file's own header for why it cannot be an app delegate.

if [ ! -f "$BGREFRESH_SRC" ]; then
    echo "ERROR: $BGREFRESH_SRC not found — background refresh is tracked outside gen/apple." >&2
    exit 1
fi
if ! cmp -s "$BGREFRESH_SRC" "$BGREFRESH_DEST"; then
    cp "$BGREFRESH_SRC" "$BGREFRESH_DEST"
    note "copied EmailOpsBackgroundRefresh.m into the generated project"
fi

if [ ! -f "$FOUNDATION_SRC" ]; then
    echo "ERROR: $FOUNDATION_SRC not found — the Foundation Models shim is tracked outside gen/apple." >&2
    exit 1
fi
if ! cmp -s "$FOUNDATION_SRC" "$FOUNDATION_DEST"; then
    cp "$FOUNDATION_SRC" "$FOUNDATION_DEST"
    note "copied EmailOpsFoundationModels.swift into the generated project"
fi

if [ ! -f "$AIBRIDGE_SRC" ]; then
    echo "ERROR: $AIBRIDGE_SRC not found — the AI bridge is tracked outside gen/apple." >&2
    exit 1
fi
if ! cmp -s "$AIBRIDGE_SRC" "$AIBRIDGE_DEST"; then
    cp "$AIBRIDGE_SRC" "$AIBRIDGE_DEST"
    note "copied EmailOpsAiBridge.m into the generated project"
fi

if [ ! -f "$LOGBRIDGE_SRC" ]; then
    echo "ERROR: $LOGBRIDGE_SRC not found — the log bridge is tracked outside gen/apple." >&2
    exit 1
fi
if ! cmp -s "$LOGBRIDGE_SRC" "$LOGBRIDGE_DEST"; then
    cp "$LOGBRIDGE_SRC" "$LOGBRIDGE_DEST"
    note "copied EmailOpsLogBridge.m into the generated project"
fi

# ── app icons ────────────────────────────────────────────────────────────────

bash "$REPO_ROOT/scripts/ios_icons.sh"

# ── export options ───────────────────────────────────────────────────────────

# Only the template's `debugging` is overwritten. A deliberate
# `IOS_EXPORT_METHOD=development` build (scripts/ios.sh) must survive this, or
# every device install would need the value re-set by hand.
if [ -f "$EXPORT_OPTIONS" ] && grep -q '<string>debugging</string>' "$EXPORT_OPTIONS"; then
    /usr/libexec/PlistBuddy -c "Set :method app-store-connect" "$EXPORT_OPTIONS" >/dev/null
    note "ExportOptions method -> app-store-connect"
fi

if [ "$changed" -eq 0 ]; then
    echo "generated project already patched; nothing to do"
fi

# ── verification ─────────────────────────────────────────────────────────────
#
# Assert every invariant independently of whether this run changed anything, so
# an upstream template change fails loudly here instead of as a confusing
# linker error, a silently broken sign-in, or a rejected upload.

fail() {
    echo "ERROR: $1" >&2
    exit 1
}

grep -q 'Accelerate.framework' "$PROJECT_YML" || fail "Accelerate.framework missing — cargo-mobile2's template likely changed."
grep -qE '^\s+iOS: 26\.0$' "$PROJECT_YML" || fail "deployment target is not 26.0."
grep -q 'com.googleusercontent.apps' "$PROJECT_YML" || fail "the Google reversed-client-id scheme is missing."
grep -qE '^\s+buildPhase: none$' "$PROJECT_YML" || fail "Externals is back in the resources phase — libapp.a would ship inside the .app."
grep -q 'increased-memory-limit' "$PROJECT_YML" || fail "the increased-memory-limit entitlement is missing from project.yml."
cmp -s "$PRIVACY_SRC" "$PRIVACY_DEST" || fail "PrivacyInfo.xcprivacy did not land in the generated project."
cmp -s "$BGREFRESH_SRC" "$BGREFRESH_DEST" || fail "EmailOpsBackgroundRefresh.m did not land in the generated project."
cmp -s "$FOUNDATION_SRC" "$FOUNDATION_DEST" || fail "EmailOpsFoundationModels.swift did not land in the generated project."
cmp -s "$AIBRIDGE_SRC" "$AIBRIDGE_DEST" || fail "EmailOpsAiBridge.m did not land — Rust would never receive the probe."
cmp -s "$LOGBRIDGE_SRC" "$LOGBRIDGE_DEST" || fail "EmailOpsLogBridge.m did not land — device logs would be invisible."
grep -q 'SWIFT_VERSION' "$PROJECT_YML" || fail "SWIFT_VERSION is missing — the Swift shim would not compile."
grep -q 'FoundationModels.framework' "$PROJECT_YML" || fail "FoundationModels.framework is missing — the availability probe would not link."
icon="$APPLE_DIR/Assets.xcassets/AppIcon.appiconset/AppIcon-512@2x.png"
[ -f "$icon" ] || fail "the App Store icon is missing from the asset catalog."
# `hasAlpha: yes` here is a guaranteed upload rejection (ITMS-90717), and is
# also how the cargo-mobile2 placeholder announces itself after a fresh init.
sips -g hasAlpha "$icon" 2>/dev/null | grep -q "hasAlpha: no" || fail "the App Store icon still carries an alpha channel."
grep -q 'BackgroundTasks.framework' "$PROJECT_YML" || fail "BackgroundTasks.framework missing — background refresh would not link."
grep -q '<string>debugging</string>' "$EXPORT_OPTIONS" &&
    fail "ExportOptions still uses the template's 'debugging' method, which App Store Connect refuses."


# `Externals/` (the Rust staticlib) and `assets/` (bundled resources) are build
# outputs and gitignored, so a fresh clone or worktree has neither — and
# xcodegen refuses to generate when a declared source directory is missing.
# Creating them empty is enough: the build populates them.
mkdir -p "$APPLE_DIR/Externals" "$APPLE_DIR/assets"

cd "$APPLE_DIR"
xcodegen generate

# Post-generate: assert what actually reaches the bundle, since xcodegen writes
# Info.plist and the .entitlements file itself and would happily drop a key we
# put in the wrong place in project.yml.
plist_has() {
    /usr/libexec/PlistBuddy -c "Print :$2" "$1" >/dev/null 2>&1
}

plist_has "$ENTITLEMENTS" "com.apple.developer.kernel.increased-memory-limit" ||
    fail "xcodegen did not write the increased-memory-limit entitlement."
plist_has "$APPLE_DIR/emailops_iOS/Info.plist" "CFBundleURLTypes" ||
    fail "CFBundleURLTypes is missing from the generated Info.plist — OAuth would fail silently."
plist_has "$APPLE_DIR/emailops_iOS/Info.plist" "ITSAppUsesNonExemptEncryption" ||
    fail "ITSAppUsesNonExemptEncryption is missing from the generated Info.plist."
plist_has "$APPLE_DIR/emailops_iOS/Info.plist" "BGTaskSchedulerPermittedIdentifiers:0" ||
    fail "the background-refresh identifier is missing from the generated Info.plist."
plist_has "$APPLE_DIR/emailops_iOS/Info.plist" "UIBackgroundModes:0" ||
    fail "UIBackgroundModes is missing from the generated Info.plist."
# CFBundleVersion is deliberately NOT asserted here. xcodegen writes it from
# project.yml, and then `tauri ios build` overwrites it from the Tauri config on
# every build — verified by inspecting a signed IPA. The build number therefore
# lives in `bundle.iOS.bundleVersion` in src-tauri/tauri.conf.json, which is the
# only value that reaches the bundle. Bump it for a re-upload of the same
# marketing version.

echo "generated project verified"
