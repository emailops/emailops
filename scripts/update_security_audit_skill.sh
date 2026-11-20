#!/usr/bin/env bash
# Refresh the vendored security-audit skill from upstream and record the commit.
#   make security-audit-skill            # latest upstream default branch
#   make security-audit-skill REF=<sha>  # a specific commit or tag
# The skill drives agents with shell access, so it is vendored rather than
# installed: an update lands as a working-tree diff to review and commit, and
# the upstream commits since the previous pin are listed here.
set -euo pipefail

repo="${SECURITY_AUDIT_SKILL_REPO:-https://github.com/cloudflare/security-audit-skill}"
ref="${1:-HEAD}"
dest=".claude/skills/security-audit"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

git clone --quiet --no-checkout "$repo" "$tmp/src"
commit="$(git -C "$tmp/src" rev-parse --verify "$ref^{commit}")"
date="$(git -C "$tmp/src" log -1 --format=%cs "$commit")"

mkdir "$tmp/out"
git -C "$tmp/src" archive "$commit" skills/security-audit LICENSE | tar -x -C "$tmp/out"
if [ -n "$(find "$tmp/out" -type l)" ]; then
  echo "upstream $commit has a symlink under skills/security-audit; not vendoring it:" >&2
  find "$tmp/out" -type l >&2
  exit 1
fi
if [ ! -f "$tmp/out/skills/security-audit/SKILL.md" ]; then
  echo "upstream $commit has no skills/security-audit/SKILL.md" >&2
  exit 1
fi

previous="$(sed -n 's/^commit: \([0-9a-f]\{40\}\).*/\1/p' "$dest/SOURCE" 2>/dev/null || true)"
if [ -n "$previous" ] && git -C "$tmp/src" cat-file -e "$previous^{commit}" 2>/dev/null; then
  echo "Upstream commits since ${previous:0:12}:"
  git -C "$tmp/src" log --oneline "$previous..$commit" -- skills/security-audit LICENSE
fi

rm -rf "$dest"
mkdir -p "$dest"
cp -R "$tmp/out/skills/security-audit/." "$dest/"
cp "$tmp/out/LICENSE" "$dest/LICENSE"
cat > "$dest/SOURCE" <<EOF
Vendored, unmodified, from $repo
path: skills/security-audit/ (plus the repository LICENSE, MIT)
commit: $commit ($date)
To update: make security-audit-skill [REF=<commit or tag>], then review git diff -- $dest
EOF
echo "Vendored $dest at ${commit:0:12} ($date). Review: git diff -- $dest"
