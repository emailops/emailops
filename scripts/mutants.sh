#!/usr/bin/env bash
# Mutation testing of the Rust crate with cargo-mutants, run in place inside a
# dedicated detached worktree so each mutant only rebuilds the emailops crate
# against the shared target dir (a scratch copy would rebuild every dependency).
# Never runs --in-place in the main checkout.
#
# Usage: scripts/mutants.sh [cargo-mutants args]        (make mutants ARGS="...")
#   make mutants ARGS="--list --file src/sync/http_retry.rs"
#   make mutants ARGS="--file 'src/services/junk/*.rs'"
#   make mutants ARGS="--in-diff /tmp/pr.diff"   # diff relative to src-tauri/:
#       git diff --relative=src-tauri main -- src-tauri/src > /tmp/pr.diff
# Env:
#   FEATURES  cargo features on top of --no-default-features (default: none, the
#             CI test set). `llamacpp` also lifts the exclusion of src/ai/llama_cpp/.
#   LABEL     output subdirectory (default: timestamp)
#
# Mutants are generated from HEAD (the worktree is moved to it on every run), so
# commit a new test before re-running to see it catch a mutant. Settings
# (excludes, timeouts, test args) live in src-tauri/.cargo/mutants.toml.
# Output: src-tauri/reports/mutants/<LABEL>/mutants.out/ (gitignored).
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
if [[ "$ROOT" == */.claude/worktrees/* ]]; then
  echo "mutants: run from the main checkout; it manages its own worktree" >&2
  exit 2
fi
WT="$ROOT/.claude/worktrees/mutants"
SHARED_TARGET="$(cd src-tauri/target && pwd -P)"
MIN_FREE_GB=12

free_gb="$(df -g "$SHARED_TARGET" | awk 'NR==2{print $4}')"
if (( free_gb < MIN_FREE_GB )); then
  echo "mutants: only ${free_gb} GB free on $SHARED_TARGET (need ${MIN_FREE_GB})" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain -- src-tauri/src src-tauri/tests)" ]]; then
  echo "mutants: note — uncommitted Rust changes are NOT part of this run (it mutates HEAD)" >&2
fi

if [[ ! -d "$WT" ]]; then
  git worktree add --quiet --detach "$WT" HEAD
elif [[ -n "$(git -C "$WT" status --porcelain)" ]]; then
  echo "mutants: $WT has local changes (an interrupted run?); inspect with git -C $WT diff" >&2
  exit 1
else
  git -C "$WT" checkout --quiet --detach "$(git rev-parse HEAD)"
fi

# The post-checkout hook links a new worktree's target to its own empty dir on
# the Build volume; point it back at the shared one and drop the empty dir.
link="$WT/src-tauri/target"
stray=""
if [[ -L "$link" ]]; then
  stray="$(readlink "$link")"
elif [[ -e "$link" ]]; then
  rmdir "$link"
fi
ln -sfn "$SHARED_TARGET" "$link"
if [[ -n "$stray" && "$stray" != "$SHARED_TARGET" && -d "$stray" ]]; then
  rmdir "$stray" "$(dirname "$stray")" 2>/dev/null || echo "mutants: left non-empty $stray in place" >&2
fi

FEATURES="${FEATURES:-}"
args=(--in-place --no-default-features)
if [[ -n "$FEATURES" ]]; then
  args+=(--features "$FEATURES")
fi
if [[ ",$FEATURES," != *,llamacpp,* ]]; then
  # The llama_cpp module and the one llamacpp-gated helper outside it are not compiled.
  args+=(--exclude 'src/ai/llama_cpp/**' --exclude-re 'llamacpp_runtime')
fi

# Diff paths are resolved from the main checkout, the run happens in the worktree.
user=()
while (( $# )); do
  case "$1" in
    -D|--in-diff) user+=("$1" "$(cd "$(dirname "$2")" && pwd -P)/$(basename "$2")"); shift 2 ;;
    *) user+=("$1"); shift ;;
  esac
done

OUT="$ROOT/src-tauri/reports/mutants/${LABEL:-$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$OUT"
cd "$WT/src-tauri"
status=0
cargo mutants "${args[@]}" --output "$OUT" "${user[@]+"${user[@]}"}" || status=$?
# cargo-mutants restores every file after each mutant; a dirty tree means it was killed mid-run.
if [[ -n "$(git status --porcelain)" ]]; then
  echo "mutants: worktree left modified — restore with: git -C $WT checkout -- ." >&2
fi
echo "results: $OUT/mutants.out (exit $status: 0 all caught, 2 missed, 3 timeout, 4 baseline failed)"
exit "$status"
