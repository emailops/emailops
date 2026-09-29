#!/usr/bin/env bash
# Keep src-tauri/target on the "Build" APFS volume, when this machine has one.
#
# Build output is regenerable and huge (10-25 GB per checkout). On the data
# volume Time Machine backs it up, and its local snapshots keep deleted
# gigabytes pinned for a day after a clean. A separate APFS volume excluded
# from Time Machine avoids both, and a symlink keeps every
# `src-tauri/target/...` path in the Makefile and scripts valid.
#
# One-time setup (admin):
#   sudo diskutil apfs addVolume <container> APFS Build
#   sudo tmutil addexclusion -v /Volumes/Build
#
# Usage: build_target.sh link | clean
#   link   Point src-tauri/target at /Volumes/Build/emailops/<checkout>/target.
#          No-op without the volume (CI, other machines). Runs from the
#          lefthook post-checkout hook, so new worktrees are linked when
#          created. Never deletes a real target/ directory — it asks for a
#          clean first, since the output cannot be moved for free.
#   clean  Free this checkout's build output. `cargo clean` on a symlinked
#          target removes only the link and leaves every byte on the volume,
#          so a linked target is emptied in place instead.
set -euo pipefail

VOLUME="/Volumes/Build"
ROOT="$(git rev-parse --show-toplevel)"
LINK="$ROOT/src-tauri/target"
DEST="$VOLUME/emailops/$(basename "$ROOT")/target"

case "${1:-}" in
  link)
    [ -d "$VOLUME" ] || exit 0
    if [ -L "$LINK" ] && [ "$(readlink "$LINK")" = "$DEST" ]; then
      mkdir -p "$DEST"
      exit 0
    fi
    if [ -e "$LINK" ] && [ ! -L "$LINK" ]; then
      echo "[build-target] $LINK is a real directory; run 'make clean && make link-target'" >&2
      echo "  to move this checkout's build output to $VOLUME." >&2
      exit 0
    fi
    mkdir -p "$DEST"
    ln -sfn "$DEST" "$LINK"
    echo "[build-target] src-tauri/target -> $DEST"
    ;;
  clean)
    if [ -L "$LINK" ]; then
      target="$(readlink "$LINK")"
      case "$target" in
        "$VOLUME"/emailops/*/target) ;;
        *) echo "[build-target] refusing to empty unexpected link target: $target" >&2; exit 1 ;;
      esac
      rm -rf "$target"
      mkdir -p "$target"
      echo "[build-target] emptied $target"
    else
      cargo clean --manifest-path "$ROOT/src-tauri/Cargo.toml"
    fi
    ;;
  *)
    echo "usage: $0 link | clean" >&2
    exit 2
    ;;
esac
