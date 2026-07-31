#!/usr/bin/env bash
# Build the desktop bundle for Linux or Windows.
#
#   scripts/build_platform.sh linux
#   scripts/build_platform.sh windows
#   CARGO_FEATURES=cuda scripts/build_platform.sh linux
#
# Mirrors the `build-mac-intel` pattern: a per-platform overlay merged over
# src-tauri/tauri.conf.json via --config. The base config keeps macOS-only
# bundle targets (app, dmg) so the signed/notarized mac release path is
# byte-for-byte unaffected by this script's existence.
#
# No code signing. Linux packages are conventionally unsigned, and Windows
# signing needs an EV/OV certificate the project does not currently hold —
# unsigned Windows installers show a SmartScreen warning on first run.

set -euo pipefail

PLATFORM="${1:-}"
CARGO_FEATURES="${CARGO_FEATURES:-}"
# Set to 1 to build without embedded llama.cpp — skips the CMake/C++ toolchain.
# CI uses this to validate the bundle configuration quickly, leaving the slow
# llamacpp compile to a dedicated job.
NO_DEFAULT_FEATURES="${NO_DEFAULT_FEATURES:-}"
# Set to 1 to ship ggml's backends as loadable modules so one artifact can use
# a GPU when the driver is present and fall back to CPU when it is not.
DYNAMIC_BACKENDS="${DYNAMIC_BACKENDS:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

case "$PLATFORM" in
  linux)   CONFIG="src-tauri/tauri.linux.conf.json";   TARGET="x86_64-unknown-linux-gnu" ;;
  windows) CONFIG="src-tauri/tauri.windows.conf.json"; TARGET="x86_64-pc-windows-msvc" ;;
  "")      echo "usage: $0 <linux|windows>" >&2; exit 2 ;;
  *)       echo "unknown platform '$PLATFORM' (expected: linux, windows)" >&2; exit 2 ;;
esac

if [ ! -f "$CONFIG" ]; then
  echo "ERROR: missing $CONFIG" >&2
  exit 1
fi

# The bundled embedding GGUF must exist before tauri-build validates
# bundle.resources.
bash scripts/fetch_bundled_models.sh

# Cargo args are forwarded after the second `--`, exactly as build-mac-intel
# forwards `--no-default-features`.
CARGO_ARGS=()
if [ "$NO_DEFAULT_FEATURES" = "1" ]; then
  echo "[build-$PLATFORM] building WITHOUT embedded llama.cpp"
  CARGO_ARGS+=(--no-default-features)
fi
if [ -n "$CARGO_FEATURES" ]; then
  echo "[build-$PLATFORM] extra cargo features: $CARGO_FEATURES"
  CARGO_ARGS+=(--features "$CARGO_FEATURES")
fi

BACKENDS_RES_DIR="src-tauri/resources/backends"

if [ "$DYNAMIC_BACKENDS" = "1" ]; then
  CARGO_ARGS+=(--features dynamic-backends)

  if [ "$PLATFORM" = "linux" ]; then
    # With dynamic-backends, libggml-base becomes a real shared-library
    # dependency of the main binary (DT_NEEDED) rather than being statically
    # linked in — the dlopen'd backend .so's need a shared libggml-base to
    # resolve their own symbols against at runtime. It ships under the
    # bundle's backends/ resource dir (staged below), so the binary needs an
    # rpath pointing there relative to its own install location. For the
    # .deb this is /usr/bin/<bin> -> /usr/lib/<ProductName>/backends; set
    # before the first cargo invocation so both build passes agree (mismatched
    # RUSTFLAGS between the two would force a full rebuild on pass 2 instead
    # of the intended cache hit).
    PRODUCT_NAME="$(node -e "console.log(require('./src-tauri/tauri.conf.json').productName)")"
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,\$ORIGIN/../lib/$PRODUCT_NAME/backends"
  fi

  # llama-cpp-sys-2's build.rs hard-links its shared libs into target/.../release/
  # guarded by `if !dst.exists()` — but Rust's Path::exists() follows symlinks
  # and returns false for a *dangling* one. A prior build leaves these as
  # dangling symlinks (their sibling versioned .so lives only in that build's
  # now-stale OUT_DIR), so the guard is fooled and the next build's hard_link()
  # call panics with AlreadyExists on the very entry it thought was absent.
  # Removing them first makes repeated builds idempotent instead of one-shot.
  find "src-tauri/target/$TARGET/release" \
       "src-tauri/target/$TARGET/release/examples" \
       "src-tauri/target/$TARGET/release/deps" \
       -maxdepth 1 \( -iname 'libggml*.so*' -o -iname 'libllama*.so*' \) -delete 2>/dev/null || true

  # Two-pass build. `tauri build` validates bundle.resources while compiling,
  # so the backend modules must already be staged — but they only exist AFTER
  # llama-cpp-sys-2's build script has run. So compile first, stage, then
  # bundle (the second cargo invocation is a cache hit).
  echo "[build-$PLATFORM] pass 1/2: compiling to produce ggml backend modules"
  cargo build --release --manifest-path src-tauri/Cargo.toml --target "$TARGET" "${CARGO_ARGS[@]}"

  # llama-cpp-sys-2 installs the modules under its OUT_DIR and advertises the
  # location via `cargo:backends_dir`. Locate the most recent one rather than
  # parsing build output, so this survives cargo rebuilding the sys crate.
  SRC_DIR="$(find "src-tauri/target/$TARGET/release/build" -type d -name backends -path '*llama-cpp-sys-2*' \
             -exec ls -dt {} + 2>/dev/null | head -1 || true)"

  if [ -z "$SRC_DIR" ] || [ -z "$(ls -A "$SRC_DIR" 2>/dev/null)" ]; then
    echo "ERROR: dynamic-backends requested but no backend modules were produced." >&2
    echo "       Looked under src-tauri/target/$TARGET/release/build/*llama-cpp-sys-2*/out/backends" >&2
    echo "       Check that the dynamic-backends feature reached llama-cpp-sys-2." >&2
    exit 1
  fi

  rm -rf "$BACKENDS_RES_DIR"
  mkdir -p "$BACKENDS_RES_DIR"
  # Only the loadable modules — the directory also holds CMake bookkeeping.
  find "$SRC_DIR" -maxdepth 1 -type f \( -name '*.so' -o -name '*.dll' -o -name '*.dylib' \) \
    -exec cp {} "$BACKENDS_RES_DIR/" \;

  # libggml-base/libggml/libllama/libllama-common (ggml-base.dll etc. on
  # Windows) are direct shared-library dependencies of the main binary (each
  # is its own `cargo:rustc-link-lib=dylib=...` from llama-cpp-sys-2, not just
  # the always-linked ggml core) — they are not among the hot-swappable
  # backend modules above, and live in OUT_DIR's sibling lib/ dir, not
  # backends/. Stage them too: without them, the installed .deb fails to
  # start (missing shared dependency) and linuxdeploy aborts the whole
  # AppImage bundle rather than just omitting it.
  # -a (not plain cp) preserves the SONAME symlink chain: e.g.
  # libggml-base.so.0 — the exact name ldd/linuxdeploy resolve DT_NEEDED
  # against — is a symlink to libggml-base.so.0.13.1, not a regular file, so a
  # type-f-only copy silently drops it and leaves the dependency unresolvable
  # under the correct name.
  #
  # KNOWN GAP (Windows): this stages ggml-base.dll etc. into backends/, but
  # unlike the Linux rpath fix above there is no Windows equivalent here yet —
  # Windows' default DLL search order checks the exe's own directory, not a
  # backends\ subdirectory, so a staged DLL sitting only in backends/ may
  # still fail to resolve as an implicit load-time dependency of emailops.exe.
  # This needs verifying on an actual Windows build before trusting it.
  BASE_LIB_SRC_DIR="$(dirname "$SRC_DIR")/lib"
  find "$BASE_LIB_SRC_DIR" -maxdepth 1 \( -type f -o -type l \) \
    \( -name 'libggml*.so*' -o -name 'libllama*.so*' -o -iname 'ggml*.dll' -o -iname 'llama*.dll' \) \
    -exec cp -a {} "$BACKENDS_RES_DIR/" \; 2>/dev/null || true

  echo "[build-$PLATFORM] staged $(ls -1 "$BACKENDS_RES_DIR" | wc -l | tr -d ' ') backend module(s) from $SRC_DIR"
  ls -1 "$BACKENDS_RES_DIR" | sed 's/^/    /'

  CONFIG="$CONFIG src-tauri/tauri.backends.conf.json"

  if [ "$PLATFORM" = "linux" ]; then
    # linuxdeploy resolves the AppImage binary's shared-library dependencies
    # via an ldd-style scan *before* bundle.resources are merged in, so
    # libggml-base.so.0 must already be on the search path or the whole
    # AppImage bundle step aborts (rather than just skipping the AppImage).
    export LD_LIBRARY_PATH="$PWD/$BACKENDS_RES_DIR:${LD_LIBRARY_PATH:-}"
  fi
else
  # Stale modules from a previous GPU build would otherwise be bundled into a
  # CPU-only artifact and loaded at runtime.
  rm -rf "$BACKENDS_RES_DIR"
fi

echo "[build-$PLATFORM] target=$TARGET config=$CONFIG"

# `--config` is repeatable and Tauri merges the overlays left to right, so the
# backends overlay (when present) layers on top of the per-platform one.
CONFIG_ARGS=()
for c in $CONFIG; do CONFIG_ARGS+=(--config "$c"); done

if [ ${#CARGO_ARGS[@]} -gt 0 ]; then
  npm run tauri -- build --target "$TARGET" "${CONFIG_ARGS[@]}" -- "${CARGO_ARGS[@]}"
else
  npm run tauri -- build --target "$TARGET" "${CONFIG_ARGS[@]}"
fi

echo "[build-$PLATFORM] done — artifacts under src-tauri/target/$TARGET/release/bundle/"
