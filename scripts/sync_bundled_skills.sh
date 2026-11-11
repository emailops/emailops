#!/usr/bin/env bash
# Copy the reviewed app skills from an emailops-skills checkout into
# src-tauri/bundled-skills and record the commit they came from.
#   make bundled-skills SKILLS_REPO=../emailops-skills
# A skill added or removed here must also change BUNDLED in
# src-tauri/src/services/skills/bundled.rs; a test fails until it does.
set -euo pipefail

repo="${1:?usage: sync_bundled_skills.sh <emailops-skills checkout>}"
dest="src-tauri/bundled-skills"

if [ -n "$(git -C "$repo" status --porcelain -- app-skills)" ]; then
  echo "app-skills has uncommitted changes in $repo: commit them first" >&2
  exit 1
fi

rm -rf "$dest"
mkdir -p "$dest"
cp -R "$repo/app-skills/." "$dest/"
echo "emailops-skills app-skills $(git -C "$repo" rev-parse HEAD)" > "$dest/SOURCE"
