#!/usr/bin/env bash
# Mutation testing of the Rust crate with cargo-mutants, run in place inside
# dedicated detached worktrees, each with its own persistent target dir, so a
# mutant only rebuilds the emailops crate incrementally (cargo-mutants' default
# scratch copy would rebuild every dependency per copy). Never runs --in-place
# in the main checkout.
#
# Why not the main checkout's target: cargo names the crate's artifacts without
# the checkout path, so two checkouts building the same features overwrite each
# other's library and incremental cache — measured 43 s per mutant build instead
# of 4-6 s while the main checkout (or another shard) was also building.
#
# Usage: scripts/mutants.sh [cargo-mutants args]        (make mutants ARGS="...")
#   make mutants ARGS="--list --file src/sync/http_retry.rs"
#   make mutants ARGS="--file src/sync/http_retry.rs -- -- sync::http_retry"   # fast pass:
#       only the tests whose path matches the filter run for each mutant
#   make mutants ARGS="--in-diff /tmp/pr.diff"   # diff relative to src-tauri/:
#       git diff --relative=src-tauri main -- src-tauri/src > /tmp/pr.diff
# Env:
#   FEATURES  cargo features on top of --no-default-features (default: none, the
#             CI test set). `llamacpp` also lifts the exclusion of src/ai/llama_cpp/.
#   LABEL     output subdirectory (default: timestamp)
#   SHARDS    run N shards in parallel, one worktree and target dir each
#             (~3 GB and a ~75 s dependency build the first time). Default 1.
#   RECHECK   label of an earlier run: re-test only the mutants it reported
#             missed, against the whole library and integration suite.
#
# Mutants are generated from HEAD (the worktrees are moved to it on every run), so
# commit a new test before re-running to see it catch a mutant. Settings
# (excludes, timeouts, test args) are read from this checkout's
# src-tauri/.cargo/mutants.toml.
# Output: src-tauri/reports/mutants/<LABEL>/[shard-<i>/]mutants.out/ (gitignored).
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
if [[ "$ROOT" == */.claude/worktrees/* ]]; then
  echo "mutants: run from the main checkout; it manages its own worktrees" >&2
  exit 2
fi
REPORTS="$ROOT/src-tauri/reports/mutants"
SHARDS="${SHARDS:-1}"
MIN_FREE_GB=12

free_gb="$(df -g "$(cd src-tauri/target && pwd -P)" | awk 'NR==2{print $4}')"
if (( free_gb < MIN_FREE_GB )); then
  echo "mutants: only ${free_gb} GB free for build output (need ${MIN_FREE_GB})" >&2
  exit 1
fi
if [[ -n "$(git status --porcelain -- src-tauri/src src-tauri/tests)" ]]; then
  echo "mutants: note — uncommitted Rust changes are NOT part of this run (it mutates HEAD)" >&2
fi

# Create or move a worktree to HEAD. Its target is linked by build_target.sh (also the
# post-checkout hook) to /Volumes/Build/emailops/<worktree>/target when that volume exists.
prepare_worktree() {
  local wt="$1"
  if [[ ! -d "$wt" ]]; then
    git worktree add --quiet --detach "$wt" HEAD
  elif [[ -n "$(git -C "$wt" status --porcelain)" ]]; then
    echo "mutants: $wt has local changes (an interrupted run?); inspect with git -C $wt diff" >&2
    exit 1
  else
    git -C "$wt" checkout --quiet --detach "$(git rev-parse HEAD)"
  fi
  (cd "$wt" && bash scripts/build_target.sh link > /dev/null)
  # Incremental rebuilds leave the crate's codegen objects in deps/ (about 1 GB
  # per 15 minutes of mutants). They cannot be pruned one by one — the next
  # incremental build links against them — so drop the crate's incremental
  # cache and artifacts together: dependencies stay, the crate rebuilds once.
  local target="$wt/src-tauri/target/debug"
  if [[ -d "$target/deps" ]]; then
    target="$(cd "$target" && pwd -P)"
    rm -rf "$target"/incremental/emailops_lib-* "$target"/deps/emailops_lib-*
  fi
}

FEATURES="${FEATURES:-}"
args=(--in-place --no-default-features --config "$ROOT/src-tauri/.cargo/mutants.toml")
if [[ -n "$FEATURES" ]]; then
  args+=(--features "$FEATURES")
fi
if [[ ",$FEATURES," != *,llamacpp,* ]]; then
  # The llama_cpp module and the one llamacpp-gated helper outside it are not compiled.
  args+=(--exclude 'src/ai/llama_cpp/**' --exclude-re 'llamacpp_runtime')
fi
if [[ -n "${RECHECK:-}" ]]; then
  # One anchored --re per missed mutant; the integration tests join the lib tests.
  # --re does not filter struct-field deletions (cargo-mutants 27), so the
  # missed mutants' files are passed with --file as well.
  missed="$(find "$REPORTS/$RECHECK" -name missed.txt -exec cat {} +)"
  while IFS= read -r re; do
    args+=(--re "$re")
  done < <(sed -e 's/[][\\.+*?(){}|^$]/\\&/g' -e 's/^/^/' -e 's/$/$/' <<< "$missed")
  while IFS= read -r file; do
    args+=(--file "$file")
  done < <(cut -d: -f1 <<< "$missed" | sort -u)
  args+=(--cargo-test-arg=--tests)
fi

# Diff paths are resolved from the main checkout, the run happens in a worktree.
user=()
while (( $# )); do
  case "$1" in
    -D|--in-diff) user+=("$1" "$(cd "$(dirname "$2")" && pwd -P)/$(basename "$2")"); shift 2 ;;
    *) user+=("$1"); shift ;;
  esac
done

OUT="$REPORTS/${LABEL:-$(date +%Y%m%d-%H%M%S)}"
pids=()
for (( i = 1; i <= SHARDS; i++ )); do
  wt="$ROOT/.claude/worktrees/mutants"
  out="$OUT"
  shard=()
  if (( SHARDS > 1 )); then
    (( i == 1 )) || wt="$wt-$i"
    out="$OUT/shard-$i"
    shard=(--shard "$((i - 1))/$SHARDS")
  fi
  prepare_worktree "$wt"
  mkdir -p "$out"
  run=(cargo mutants "${args[@]}" "${shard[@]+"${shard[@]}"}" --output "$out" "${user[@]+"${user[@]}"}")
  if (( SHARDS == 1 )); then
    # A single run also streams to the terminal, so --list and progress are visible.
    (cd "$wt/src-tauri" && "${run[@]}") 2>&1 | tee "$out/console.txt" &
  else
    (cd "$wt/src-tauri" && "${run[@]}" > "$out/console.txt" 2>&1) &
  fi
  pids+=("$!")
done

status=0
for (( i = 1; i <= SHARDS; i++ )); do
  code=0
  wait "${pids[$((i - 1))]}" || code=$?
  wt="$ROOT/.claude/worktrees/mutants"
  out="$OUT"
  if (( SHARDS > 1 )); then
    (( i == 1 )) || wt="$wt-$i"
    out="$OUT/shard-$i"
  fi
  # cargo-mutants restores every file after each mutant; a dirty tree means it was killed mid-run.
  if [[ -n "$(git -C "$wt" status --porcelain)" ]]; then
    echo "mutants: $wt left modified — restore with: git -C $wt checkout -- ." >&2
  fi
  echo "shard $i: $(grep -a 'mutants tested' "$out/console.txt" || tail -n 1 "$out/console.txt") (exit $code)"
  if (( code > status )); then status=$code; fi
done
echo "results: $OUT (exit codes: 0 all caught, 2 missed, 3 timeout, 4 baseline failed)"
exit "$status"
