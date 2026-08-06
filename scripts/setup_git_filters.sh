#!/usr/bin/env bash
#
# Register this repo's git content filters (one-time, per clone).
#
# `strip-apple-team` keeps the Apple Development Team ID out of the tracked
# tree. `tauri ios dev` / `tauri ios build` write
# `DEVELOPMENT_TEAM = "XXXXXXXXXX";` into
# `src-tauri/gen/apple/emailops.xcodeproj/project.pbxproj` — a *tracked*,
# generated file — every time they run with `APPLE_DEVELOPMENT_TEAM` set (which
# `scripts/ios.sh` does, reading it from the gitignored `.env.signing`). Without
# this filter the id lands in the first commit anyone makes after an iOS build.
#
# A clean filter runs on the way *into* git (add / diff / status), so the id
# stays in the working copy where Xcode needs it and never reaches the index.
# `git diff` applies the same filter, so a post-build worktree still reads as
# clean instead of showing a permanent phantom modification.
#
# Filters cannot be configured by the repo itself (that would be arbitrary code
# execution on clone), which is why this is a script you run. The lefthook
# `no-apple-team-id` pre-commit check is the backstop for a clone that never
# ran it.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

git config filter.strip-apple-team.clean "sed -e '/DEVELOPMENT_TEAM = /d'"
# Identity smudge: nothing to restore on checkout — the id is supplied by
# APPLE_DEVELOPMENT_TEAM at build time, not by the file.
git config filter.strip-apple-team.smudge cat

echo "git filter 'strip-apple-team' registered (Apple team id stays out of commits)"
