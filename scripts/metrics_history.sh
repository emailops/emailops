#!/usr/bin/env bash
# Read/write the release-metrics history (`.github/workflows/metrics.yml`).
#
# The history is one CSV that grows by a row a day. It deliberately does NOT
# live on `main`: a daily bot commit there would bury real work in the log and
# show up in every `git log`, blame and release diff. Instead it lives alone on
# an orphan `metrics` branch, which shares no history with `main` and holds
# exactly one file.
#
# `push` writes that branch with git plumbing (hash-object → mktree →
# commit-tree) rather than checking the branch out. Switching branches mid-job
# would fight the workflow's checkout of `main`, and a worktree would need a
# second clone; plumbing just builds the commit object and pushes it.
#
#   scripts/metrics_history.sh pull <csv-path>   # newest history, or empty file
#   scripts/metrics_history.sh push <csv-path>   # commit + push if it changed

set -euo pipefail

BRANCH="${METRICS_BRANCH:-metrics}"
FILENAME="downloads.csv"

usage() {
  echo "usage: $0 {pull|push} <csv-path>" >&2
  exit 2
}

[ $# -eq 2 ] || usage
command="$1"
path="$2"

case "$command" in
  pull)
    # A missing branch is the expected first run: hand back an empty file and
    # let the collector start a fresh history.
    if git fetch --quiet origin "$BRANCH" 2>/dev/null; then
      git show "FETCH_HEAD:$FILENAME" > "$path"
      echo "[metrics] pulled $(( $(wc -l < "$path") - 1 )) rows from origin/$BRANCH"
    else
      : > "$path"
      echo "[metrics] no origin/$BRANCH yet — starting a new history"
    fi
    ;;

  push)
    [ -s "$path" ] || { echo "[metrics] $path is empty — refusing to push" >&2; exit 1; }

    parent=""
    if git fetch --quiet origin "$BRANCH" 2>/dev/null; then
      parent=$(git rev-parse FETCH_HEAD)
      # Nothing to record when the file is byte-identical to what is already
      # on the branch — a quiet day should not create an empty commit.
      if git show "$parent:$FILENAME" 2>/dev/null | cmp -s - "$path"; then
        echo "[metrics] history unchanged — nothing to push"
        exit 0
      fi
    fi

    blob=$(git hash-object -w "$path")
    tree=$(printf '100644 blob %s\t%s\n' "$blob" "$FILENAME" | git mktree)
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
