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

# Windows only: llama-cpp-sys-2's nested CMake ExternalProject
# (vulkan-shaders-gen) builds several directories deep under a hash-named
# target/.../build/ subdir, and the resulting absolute object-file path
# exceeds CMake's own CMAKE_OBJECT_PATH_MAX safety threshold (250 chars) even
# with Windows' long-path support enabled — CMake warns about it, then cl.exe
# fails with the exact same "C1041: cannot open program database" text it
# shows for a genuine concurrent-write race, even for a single, uncontended
# compile. Root-caused on a from-scratch VM: the fix that actually worked
# there was shortening the checkout path. On GH Actions the checkout path
# itself (D:\a\emailops\emailops) is fixed by convention, but CARGO_TARGET_DIR
# controls where cargo (and everything nested under it) puts its build output,
# and that's the overwhelming majority of the path length — point it at a
# short, fixed location instead of the default `target/` under the checkout.
if [ "$PLATFORM" = "windows" ]; then
  export CARGO_TARGET_DIR="C:/ct"
fi
TARGET_DIR="${CARGO_TARGET_DIR:-src-tauri/target}"

# The bundled embedding GGUF must exist before tauri-build validates
# bundle.resources.
bash scripts/fetch_bundled_models.sh

# Cargo args are forwarded after the second `--`, exactly as build-mac-intel
# forwards `--no-default-features`.
CARGO_ARGS=()
if [ "$NO_DEFAULT_FEATURES" = "1" ]; then
  echo "[build-$PLATFORM] building WITHOUT embedded llama.cpp"
  # `default = ["desktop", "llamacpp"]` in Cargo.toml — --no-default-features
  # drops both, not just llamacpp. This script only ever builds the desktop
  # bundle, so re-add `desktop` explicitly or the `emailops` bin (which is
  # `required-features = ["desktop"]`) never gets built: cargo silently
  # matches zero bin targets, tauri-bundler still reports "Built application
  # at: <path>", and the actual bundling step then fails with "can't open
  # main binary ... No such file or directory".
  CARGO_ARGS+=(--no-default-features --features desktop)
fi
if [ -n "$CARGO_FEATURES" ]; then
  echo "[build-$PLATFORM] extra cargo features: $CARGO_FEATURES"
  CARGO_ARGS+=(--features "$CARGO_FEATURES")
fi

BACKENDS_RES_DIR="src-tauri/resources/backends"
# Windows-only: the same base libs are ALSO staged here (see the "KNOWN GAP"
# note below) so tauri.backends.windows.conf.json can map them next to
# emailops.exe, not just into backends/.
BASE_LIBS_ROOT_RES_DIR="src-tauri/resources/backends-root"

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
  find "$TARGET_DIR/$TARGET/release" \
       "$TARGET_DIR/$TARGET/release/examples" \
       "$TARGET_DIR/$TARGET/release/deps" \
       -maxdepth 1 \( -iname 'libggml*.so*' -o -iname 'libllama*.so*' \) -delete 2>/dev/null || true

  CARGO_JOBS_ARGS=()
  if [ "$PLATFORM" = "windows" ]; then
    # llama-cpp-sys-2's vendored CMake build compiles its vulkan-shaders-gen
    # sub-tool (and runs CMake's own C/CXX compiler-ABI detection try_compiles
    # for that sub-project) with concurrent cl.exe invocations — without /FS,
    # they race on the same debug .pdb file and fail with "C1041: cannot open
    # program database ... if multiple CL.EXE write to the same .PDB file,
    # please use /FS". MSVC's documented fix is the CL env var, read for
    # default flags on every cl.exe invocation.
    #
    # Scoped to just this cargo invocation, not the whole script: this step
    # runs under Git Bash, whose MSYS runtime mangles a bare "/FS" into
    # "C:/Program Files/Git/FS" (both single- and double-leading-slash — the
    # doubled-slash escape is for argv, not env var values inherited by a
    # native child process, which is what happens here). MSYS_NO_PATHCONV
    # disables that conversion, but it's a blunt, process-wide switch — set
    # earlier (e.g. for the whole build step in CI) it also broke
    # fetch_bundled_models.sh's own legitimate curl download, which relies on
    # the same conversion for its POSIX-style output path.
    export CL="/FS"
    export MSYS_NO_PATHCONV=1

    # CL=/FS alone doesn't reach this race: CMake's own internal compiler-ABI
    # detection (CMakeTestCCompiler.cmake / CMakeTestCXXCompiler.cmake, run
    # for the vulkan-shaders-gen sub-project's own from-scratch `project()`
    # bootstrap) compiles a scratch test file using CMake's hardcoded default
    # debug flags, not our CMAKE_C_FLAGS/CMAKE_CXX_FLAGS overrides — so /FS
    # never reaches that specific cl.exe invocation no matter how it's passed
    # in. Confirmed against a real CI failure log: our
    # -DCMAKE_C_FLAGS="... /FS ..." was present in the cmake configure
    # command, yet C1041 still fired from exactly this ABI-detection step.
    #
    # CMAKE_BUILD_PARALLEL_LEVEL is a red herring here: passing it via -D at
    # configure time is inert (CMake warns "Manually-specified variables were
    # not used by the project"), and exporting it as an env var is *also*
    # inert in practice — the `cmake` crate (which llama-cpp-sys-2 uses to
    # drive the build) constructs its `cmake --build ... --parallel N` call
    # from cargo's own NUM_JOBS env var, not CMAKE_BUILD_PARALLEL_LEVEL. Cargo
    # sets NUM_JOBS itself (to match its own build concurrency) for every
    # build script invocation, overriding whatever we export ahead of time —
    # confirmed both by reading the vendored `cmake` crate source and by the
    # real CI log still showing `--parallel 4` despite
    # CMAKE_BUILD_PARALLEL_LEVEL=1 being exported.
    #
    # Remove the race at its source instead: force this cargo invocation
    # itself to a single job. That makes cargo set NUM_JOBS=1 for the
    # build script, which the `cmake` crate turns into `--parallel 1` for
    # every cmake-driven sub-build (the main ggml build and the
    # vulkan-shaders-gen ExternalProject alike) — so no two cl.exe processes
    # are ever writing a .pdb concurrently, regardless of whether that
    # particular invocation picked up /FS.
    #
    # Scoped to vulkan specifically (not "any Windows build"): vulkan-shaders-gen
    # is only ever configured when GGML_VULKAN is ON (see llama-cpp-sys-2's
    # build.rs), which only happens for CARGO_FEATURES=vulkan. A CUDA build
    # never touches that sub-project, so it never hits this race — verified by
    # timing a from-scratch CUDA release compile at 270m40s forced to --jobs 1
    # vs 31m54s with full parallelism restored (8.5x, on a 4-vCPU test VM).
    # Applying --jobs 1 unconditionally to every Windows build (as this used
    # to) silently paid that ~4.5h tax on every CUDA build for no reason.
    if [[ "$CARGO_FEATURES" == *"vulkan"* ]]; then
      CARGO_JOBS_ARGS+=(--jobs 1)
    fi

    # CMAKE_GENERATOR=Ninja routes around the C1041 race above entirely —
    # Ninja invokes cl.exe one translation unit at a time with no
    # nested-MSBuild parallel-project layer for llama-cpp-sys-2's
    # vulkan-shaders-gen ExternalProject sub-build to race inside of. Scoped
    # here (not set via the CI workflow's `env:`) because GitHub Actions
    # always defines an `env:` key even when its expression evaluates to an
    # empty string — CMAKE_GENERATOR="" broke the Linux leg with "CMake
    # Error: Could not create named generator ''", since CMake treats an
    # explicitly-empty generator as a real (invalid) request, not "unset".
    # The CI workflow's ilammy/msvc-dev-cmd step (Windows only) still
    # populates VCToolsInstallDir/INCLUDE/LIB so Ninja can find MSVC (unlike
    # the "Visual Studio" generator, Ninja does not locate the toolset
    # itself) — but once that's done, rustc's own linker search trusts that
    # an MSVC environment is already configured and falls back to a plain
    # PATH lookup for "link.exe" instead of doing its own registry-based
    # MSVC detection — and on this "shell: bash" step, Git Bash's own
    # coreutils `usr/bin/link.exe` (a hardlink tool, not a linker) sits
    # earlier on PATH than MSVC's, so cargo silently invokes the wrong
    # one and every crate fails to link with a confusing "extra operand"
    # error. Pin the linker explicitly so cargo never has to search PATH.
    if [ -n "${VCToolsInstallDir:-}" ]; then
      export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER="${VCToolsInstallDir}bin\\Hostx64\\x64\\link.exe"
    fi

    export CMAKE_GENERATOR=Ninja
  fi

  # Two-pass build. `tauri build` validates bundle.resources while compiling,
  # so the backend modules must already be staged — but they only exist AFTER
  # llama-cpp-sys-2's build script has run. So compile first, stage, then
  # bundle (the second cargo invocation is a cache hit).
  echo "[build-$PLATFORM] pass 1/2: compiling to produce ggml backend modules"
  cargo build --release "${CARGO_JOBS_ARGS[@]}" --manifest-path src-tauri/Cargo.toml --target "$TARGET" "${CARGO_ARGS[@]}"

  # llama-cpp-sys-2 installs the modules under its OUT_DIR and advertises the
  # location via `cargo:backends_dir`. Locate the most recent one rather than
  # parsing build output, so this survives cargo rebuilding the sys crate.
  SRC_DIR="$(find "$TARGET_DIR/$TARGET/release/build" -type d -name backends -path '*llama-cpp-sys-2*' \
             -exec ls -dt {} + 2>/dev/null | head -1 || true)"

  if [ -z "$SRC_DIR" ] || [ -z "$(ls -A "$SRC_DIR" 2>/dev/null)" ]; then
    echo "ERROR: dynamic-backends requested but no backend modules were produced." >&2
    echo "       Looked under $TARGET_DIR/$TARGET/release/build/*llama-cpp-sys-2*/out/backends" >&2
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
  BASE_LIB_SRC_DIR="$(dirname "$SRC_DIR")/lib"
  if [ "$PLATFORM" = "windows" ] && [ -z "$(find "$BASE_LIB_SRC_DIR" -maxdepth 1 -iname '*.dll' 2>/dev/null)" ]; then
    # CMake's default GNUInstallDirs convention puts *runtime* DLLs in bin/ on
    # Windows, reserving lib/ for the .lib import-library stubs the linker
    # needs at build time — unlike Linux, where .so files conventionally
    # install to lib/ (what the lib/ assumption above is based on, and is
    # correct there). First real Windows build to reach this staging step at
    # all (every earlier attempt died earlier, at the C1041 CMake bug), so
    # this fallback was never exercised until now.
    BIN_CANDIDATE="$(dirname "$SRC_DIR")/bin"
    if [ -n "$(find "$BIN_CANDIDATE" -maxdepth 1 -iname '*.dll' 2>/dev/null)" ]; then
      BASE_LIB_SRC_DIR="$BIN_CANDIDATE"
    else
      echo "[build-$PLATFORM] WARNING: no *.dll under $(dirname "$SRC_DIR")/lib or /bin — listing $(dirname "$SRC_DIR") for diagnosis:" >&2
      find "$(dirname "$SRC_DIR")" -maxdepth 2 >&2 || true
    fi
  fi
  find "$BASE_LIB_SRC_DIR" -maxdepth 1 \( -type f -o -type l \) \
    \( -name 'libggml*.so*' -o -name 'libllama*.so*' -o -iname 'ggml*.dll' -o -iname 'llama*.dll' \) \
    -exec cp -a {} "$BACKENDS_RES_DIR/" \; 2>/dev/null || true

  if [ "$PLATFORM" = "windows" ]; then
    # Windows' default DLL search order for an *implicit* link-time dependency
    # (ggml-base.dll/ggml.dll/llama.dll/llama-common.dll are each a direct
    # `cargo:rustc-link-lib=dylib=...` of the main binary, resolved by the OS
    # loader before any Rust code runs — unlike the hot-swappable backend
    # modules above, which the app itself dlopen's from an explicit path via
    # `load_backends_from_path`) only checks the executable's own directory,
    # system directories, and PATH. It does NOT check a backends\
    # subdirectory, so staging these only into backends/ (as above) leaves
    # the installed app failing to start with "The code execution cannot
    # proceed because ggml-base.dll was not found." Stage a second copy into
    # a resource dir that tauri.backends.windows.conf.json maps to the bundle
    # root (".") instead.
    rm -rf "$BASE_LIBS_ROOT_RES_DIR"
    mkdir -p "$BASE_LIBS_ROOT_RES_DIR"
    find "$BASE_LIB_SRC_DIR" -maxdepth 1 -type f -iname '*.dll' \
      \( -iname 'ggml*.dll' -o -iname 'llama*.dll' \) \
      -exec cp {} "$BASE_LIBS_ROOT_RES_DIR/" \;
  fi

  echo "[build-$PLATFORM] staged $(ls -1 "$BACKENDS_RES_DIR" | wc -l | tr -d ' ') backend module(s) from $SRC_DIR"
  ls -1 "$BACKENDS_RES_DIR" | sed 's/^/    /'

  CONFIG="$CONFIG src-tauri/tauri.backends.conf.json"

  if [ "$PLATFORM" = "linux" ]; then
    # linuxdeploy resolves the AppImage binary's shared-library dependencies
    # via an ldd-style scan *before* bundle.resources are merged in, so
    # libggml-base.so.0 must already be on the search path or the whole
    # AppImage bundle step aborts (rather than just skipping the AppImage).
    export LD_LIBRARY_PATH="$PWD/$BACKENDS_RES_DIR:${LD_LIBRARY_PATH:-}"
  elif [ "$PLATFORM" = "windows" ]; then
    CONFIG="$CONFIG src-tauri/tauri.backends.windows.conf.json"
  fi
else
  # Stale modules from a previous GPU build would otherwise be bundled into a
  # CPU-only artifact and loaded at runtime.
  rm -rf "$BACKENDS_RES_DIR" "$BASE_LIBS_ROOT_RES_DIR"
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

echo "[build-$PLATFORM] done — artifacts under $TARGET_DIR/$TARGET/release/bundle/"
