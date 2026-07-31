#!/usr/bin/env bash
# Prepare the toolchain for a desktop build on Linux or Windows.
#
#   scripts/bootstrap_platform.sh linux
#   scripts/bootstrap_platform.sh windows
#
# Adds the Rust target and REPORTS missing system prerequisites with the exact
# command to install them. It deliberately never installs system packages
# itself: `make` targets that silently invoke `sudo apt-get` are a nasty
# surprise on a machine the developer did not intend to modify.
#
# Exit status is non-zero when something required is missing, so CI can gate on
# it.

set -euo pipefail

PLATFORM="${1:-}"

if [ -z "$PLATFORM" ]; then
  echo "usage: $0 <linux|windows>" >&2
  exit 2
fi

MISSING=()

have() { command -v "$1" >/dev/null 2>&1; }

# Report a missing tool/library along with how to get it.
need() {
  local what="$1" how="$2"
  MISSING+=("$what")
  echo "  ✗ $what"
  echo "      install: $how"
}

ok() { echo "  ✓ $1"; }

add_rust_target() {
  local target="$1"
  if ! have rustup; then
    need "rustup" "https://rustup.rs"
    return
  fi
  if rustup target list --installed | grep -qx "$target"; then
    ok "rust target $target"
  else
    echo "  + adding rust target $target"
    rustup target add "$target"
  fi
}

echo "[bootstrap-$PLATFORM] checking prerequisites"

case "$PLATFORM" in
  linux)
    add_rust_target x86_64-unknown-linux-gnu

    # Tauri's Linux webview and the crates that compile C/C++ (llama-cpp-sys-2,
    # sqlite-vec, rusqlite's bundled SQLite).
    APT_PKGS=(
      libwebkit2gtk-4.1-dev
      build-essential
      curl
      wget
      file
      libxdo-dev
      libssl-dev
      libayatana-appindicator3-dev
      librsvg2-dev
      patchelf
      cmake
      libclang-dev
      clang
      xdg-utils
      libfuse2
    )

    if have pkg-config; then ok "pkg-config"; else need "pkg-config" "sudo apt-get install -y pkg-config"; fi

    if have pkg-config && pkg-config --exists webkit2gtk-4.1; then
      ok "webkit2gtk-4.1"
    else
      need "webkit2gtk-4.1 (development headers)" "sudo apt-get install -y libwebkit2gtk-4.1-dev"
    fi

    if have cmake; then ok "cmake"; else need "cmake" "sudo apt-get install -y cmake"; fi
    if have c++; then ok "c++ compiler"; else need "c++ compiler" "sudo apt-get install -y build-essential"; fi
    if have patchelf; then ok "patchelf (required to bundle an AppImage)"; else need "patchelf" "sudo apt-get install -y patchelf"; fi

    # bindgen (llama-cpp-sys-2's FFI layer) needs libclang at build time — a
    # bare Ubuntu image lacks it (GitHub's ubuntu-latest runner ships it
    # preinstalled, which is why this gap stayed invisible until a from-scratch
    # VM build hit it).
    if ldconfig -p 2>/dev/null | grep -q libclang.so; then
      ok "libclang"
    else
      need "libclang (required by bindgen for llama-cpp-sys-2)" "sudo apt-get install -y libclang-dev clang"
    fi

    # linuxdeploy (which builds the .AppImage) is itself an AppImage: it needs
    # `xdg-open` at bundle time and, to run itself, either working FUSE or
    # APPIMAGE_EXTRACT_AND_RUN=1 as a fallback on machines without /dev/fuse.
    if have xdg-open; then ok "xdg-open (required to bundle an AppImage)"; else need "xdg-open" "sudo apt-get install -y xdg-utils"; fi
    if ldconfig -p 2>/dev/null | grep -q libfuse.so.2; then
      ok "libfuse2 (or set APPIMAGE_EXTRACT_AND_RUN=1 if unavailable)"
    else
      need "libfuse2 (required for linuxdeploy to self-mount; alternative: export APPIMAGE_EXTRACT_AND_RUN=1)" "sudo apt-get install -y libfuse2"
    fi

    echo
    echo "  Full apt one-liner:"
    echo "    sudo apt-get update && sudo apt-get install -y ${APT_PKGS[*]}"
    ;;

  windows)
    add_rust_target x86_64-pc-windows-msvc

    # `link.exe` comes from the MSVC Build Tools. Its absence is the single most
    # common reason a Windows Rust build fails.
    if have link || have cl; then
      ok "MSVC toolchain (link.exe / cl.exe)"
    else
      need "MSVC Build Tools" "winget install Microsoft.VisualStudio.2022.BuildTools --override \"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools\""
    fi

    if have cmake; then ok "cmake"; else need "cmake" "winget install Kitware.CMake"; fi

    # WebView2 ships with Windows 11 and current Windows 10. The installer can
    # also fetch it at install time — see tauri.windows.conf.json's
    # webviewInstallMode — so this is a warning rather than a hard requirement.
    echo "  i WebView2 runtime: preinstalled on Windows 11 / current Windows 10."
    echo "      The generated installer downloads it if absent (webviewInstallMode)."

    echo
    echo "  NOTE: the Makefile requires a POSIX shell. On Windows run these"
    echo "        targets from Git Bash or MSYS2, not cmd.exe or PowerShell."
    ;;

  *)
    echo "unknown platform '$PLATFORM' (expected: linux, windows)" >&2
    exit 2
    ;;
esac

echo
if [ ${#MISSING[@]} -gt 0 ]; then
  echo "[bootstrap-$PLATFORM] MISSING ${#MISSING[@]} prerequisite(s): ${MISSING[*]}" >&2
  exit 1
fi

echo "[bootstrap-$PLATFORM] all prerequisites present"
