#!/usr/bin/env python3
"""Report and free regenerable build output on the Build volume.

Every checkout links src-tauri/target to /Volumes/Build/emailops/<checkout>/target
(scripts/build_target.sh). That volume shares its APFS container with the data
volume, so its gigabytes come out of the same free space, and nothing removes a
worktree's build output once the worktree goes quiet or is deleted.

Usage:
  disk_clean.py report
      Free space, each build dir (size, days since its last build, checkout or
      orphan) and the other large consumers on this machine. Read-only.
  disk_clean.py clean [--apply] [--release] [--idle-days N]
      Plan, and with --apply carry out:
        orphan   build dir whose checkout no longer exists -> removed whole
        idle     worktree target not built for N days (default 3) -> emptied
                 (never the main checkout's)
        release  with --release: the per-arch macOS release builds
                 (aarch64/x86_64/universal-apple-darwin) in every checkout,
                 once the release they were built for has shipped
      Without --apply nothing is deleted. Only build output is ever touched.
"""

import argparse
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

BUILD_ROOT = Path("/Volumes/Build/emailops")
RELEASE_TRIPLES = ("aarch64-apple-darwin", "x86_64-apple-darwin", "universal-apple-darwin")
# Directories cargo rewrites on every build that changes something; their mtimes
# date the last build without walking the whole tree.
BUILD_MARKERS = ("deps", "incremental", "build", ".fingerprint")
HOME = Path.home()
OTHER_CONSUMERS = (
    HOME / "Library/Application Support/com.emailops.app/models",
    HOME / ".cache/lm-studio",
    HOME / ".ollama",
    HOME / ".cache/huggingface",
    HOME / ".cache/uv",
    HOME / "Library/Containers/com.docker.docker",
    HOME / "Library/Developer/Xcode/DerivedData",
    HOME / "Library/Developer/CoreSimulator",
)
DAY = 86400.0


@dataclass
class BuildDir:
    name: str
    path: str
    last_write: float | None  # None when the target holds no build
    profiles: list[str]  # entries directly under target/


@dataclass
class Action:
    kind: str  # orphan | idle | release
    path: str


def plan(dirs, worktrees, main, now, idle_days, release):
    """Decide what to delete. Pure: no filesystem access."""
    actions = []
    for d in dirs:
        if d.name not in worktrees:
            actions.append(Action("orphan", d.path))
            continue
        if d.last_write is None:
            continue
        if d.name != main and now - d.last_write >= idle_days * DAY:
            actions.append(Action("idle", f"{d.path}/target"))
            continue
        if release:
            actions += [Action("release", f"{d.path}/target/{t}") for t in RELEASE_TRIPLES if t in d.profiles]
    return actions


def git_worktrees():
    """Basenames of every checkout of this repo; the first is the main one."""
    out = subprocess.run(["git", "worktree", "list", "--porcelain"], capture_output=True, text=True, check=True).stdout
    names = [Path(line.split(" ", 1)[1]).name for line in out.splitlines() if line.startswith("worktree ")]
    return set(names), names[0]


def scan(root):
    dirs = []
    for d in sorted(p for p in root.iterdir() if p.is_dir()):
        target = d / "target"
        profiles = sorted(p.name for p in target.iterdir() if p.is_dir()) if target.is_dir() else []
        stamps = [
            m.stat().st_mtime
            for prof in profiles
            for m in [target / prof, *(target / prof / marker for marker in BUILD_MARKERS)]
            if m.exists()
        ]
        dirs.append(BuildDir(d.name, str(d), max(stamps) if stamps else None, profiles))
    return dirs


def size_kb(path):
    out = subprocess.run(["du", "-sk", str(path)], capture_output=True, text=True).stdout
    return int(out.split()[0]) if out else 0


def human(kb):
    gb = kb / 1024 / 1024
    return f"{gb:.1f}G" if gb >= 1 else f"{kb / 1024:.0f}M"


def free_space():
    usage = shutil.disk_usage("/System/Volumes/Data")
    return f"free space: {usage.free / 1024**3:.0f} GiB of {usage.total / 1024**3:.0f} GiB (shared by Data and Build)"


def report():
    print(free_space())
    worktrees, _ = git_worktrees()
    now = time.time()
    print(f"\nbuild dirs under {BUILD_ROOT}:")
    for d in scan(BUILD_ROOT):
        idle = "-" if d.last_write is None else f"{(now - d.last_write) / DAY:.0f}d"
        state = "checkout" if d.name in worktrees else "orphan"
        print(f"  {human(size_kb(d.path)):>7}  idle {idle:>4}  {state:<8}  {d.name}")
    print("\nother large consumers (not touched by clean):")
    for p in OTHER_CONSUMERS:
        if p.exists():
            print(f"  {human(size_kb(p)):>7}  {p}")


def remove(action):
    path = Path(action.path).resolve()
    if BUILD_ROOT.resolve() not in path.parents:
        raise SystemExit(f"refusing to delete outside {BUILD_ROOT}: {path}")
    shutil.rmtree(path)
    if action.kind == "idle":
        # Keep the directory the checkout's src-tauri/target symlink points at.
        path.mkdir()


def clean(apply, release, idle_days):
    worktrees, main = git_worktrees()
    actions = plan(scan(BUILD_ROOT), worktrees, main, time.time(), idle_days, release)
    if not actions:
        print("nothing to clean")
        return
    total = 0
    for a in actions:
        kb = size_kb(a.path)
        total += kb
        print(f"  {human(kb):>7}  {a.kind:<7}  {a.path}")
        if apply:
            remove(a)
    verb = "freed" if apply else "would free (re-run with --apply)"
    print(f"{verb}: {human(total)}")
    if apply:
        print(free_space())


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("report")
    c = sub.add_parser("clean")
    c.add_argument("--apply", action="store_true")
    c.add_argument("--release", action="store_true")
    c.add_argument("--idle-days", type=float, default=3)
    args = parser.parse_args()
    if not BUILD_ROOT.is_dir():
        print(f"no {BUILD_ROOT} on this machine; nothing to do")
        return 0
    if args.cmd == "report":
        report()
    else:
        clean(args.apply, args.release, args.idle_days)
    return 0


if __name__ == "__main__":
    sys.exit(main())
