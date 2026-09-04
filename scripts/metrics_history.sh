#!/usr/bin/env bash
# Read/write the metrics history (`.github/workflows/metrics.yml`).
#
# The history is a handful of CSVs that grow by a row a day: downloads/stars,
# per-day traffic, and dated referrer snapshots. They deliberately do NOT live
# on `main`: a daily bot commit there would bury real work in the log and show
# up in every `git log`, blame and release diff. Instead they live alone on an
# orphan `metrics` branch, which shares no history with `main`.
#
# `push` writes that branch with git plumbing (hash-object → mktree →
# commit-tree) rather than checking the branch out. Switching branches mid-job
# would fight the workflow's checkout of `main`, and a worktree would need a
# second clone; plumbing just builds the commit object and pushes it.
#
# Both commands work on a DIRECTORY, not a single file, and `push` rebuilds the
# tree from every CSV in it. That is load-bearing: `mktree` writes a whole tree,
# so committing one file at a time would drop the others from the branch.
#
#   scripts/metrics_history.sh pull <dir>   # stored CSVs into <dir> (may be none)
#   scripts/metrics_history.sh push <dir>   # commit + push if anything changed

set -euo pipefail

BRANCH="${METRICS_BRANCH:-metrics}"

usage() {
  echo "usage: $0 {pull|push} <dir>" >&2
  exit 2
}

[ $# -eq 2 ] || usage
command="$1"
dir="$2"

case "$command" in
  pull)
    mkdir -p "$dir"
    # A missing branch is the expected first run: leave the directory empty and
    # let the collector start fresh histories.
    if git fetch --quiet origin "$BRANCH" 2>/dev/null; then
      for name in $(git ls-tree --name-only FETCH_HEAD); do
        git show "FETCH_HEAD:$name" > "$dir/$name"
        echo "[metrics] pulled $name ($(( $(wc -l < "$dir/$name") - 1 )) rows)"
      done
    else
      echo "[metrics] no origin/$BRANCH yet — starting a new history"
    fi
    ;;

  push)
    entries=$(
      for file in "$dir"/*.csv; do
        [ -s "$file" ] || continue
        printf '100644 blob %s\t%s\n' "$(git hash-object -w "$file")" "$(basename "$file")"
      done
    )
    [ -n "$entries" ] || { echo "[metrics] no non-empty CSV in $dir — refusing to push" >&2; exit 1; }
    tree=$(printf '%s\n' "$entries" | git mktree)

    parent=""
    if git fetch --quiet origin "$BRANCH" 2>/dev/null; then
      parent=$(git rev-parse FETCH_HEAD)
      # Comparing trees, not files: a quiet day should not create an empty commit.
      if [ "$(git rev-parse "$parent^{tree}")" = "$tree" ]; then
        echo "[metrics] history unchanged — nothing to push"
        exit 0
      fi
    fi

    message="metrics: $(date -u +%Y-%m-%d)"
    if [ -n "$parent" ]; then
      commit=$(git commit-tree "$tree" -p "$parent" -m "$message")
    else
      commit=$(git commit-tree "$tree" -m "$message")
    fi

    git push --quiet origin "$commit:refs/heads/$BRANCH"
    echo "[metrics] pushed $commit to $BRANCH"
    ;;

  *)
    usage
    ;;
esac
