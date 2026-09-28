#!/usr/bin/env bash
# Run the repo's quality gates and print a one-line summary per gate.
#
# Each gate's full output goes to <out_dir>/<gate>.txt; stdout carries only a
# summary table (and, for failing gates, a short excerpt), so an agent or a
# human sees the result without scrolling through thousands of test lines.
# Exit code: 0 when every requested gate passed, 1 otherwise (a skipped gate
# counts as not passed — it did not run).
#
# Usage: scripts/gates.sh <set|gate> [out_dir]      (make gates SET=... OUT=...)
#   sets:  commit   clippy clippy-desktop fmt biome tsc   (pre-commit checks)
#          push     rust-test clippy clippy-desktop outdated  (pre-push checks)
#          rust     rust-test clippy clippy-desktop fmt
#          frontend biome tsc vitest
#          all      every gate
#   or a single gate id from the lists above.
#
# Frontend gates run under the Node major pinned in .nvmrc, found on PATH or
# in nvm/fnm; without it they are skipped (and the run fails). In a
# .claude/worktrees/ checkout, cargo reuses the main checkout's target dir
# unless CARGO_TARGET_DIR is already set, to avoid a cold build.
set -uo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

case "${1:-}" in
  commit)   GATES=(clippy clippy-desktop fmt biome tsc) ;;
  push)     GATES=(rust-test clippy clippy-desktop outdated) ;;
  rust)     GATES=(rust-test clippy clippy-desktop fmt) ;;
  frontend) GATES=(biome tsc vitest) ;;
  all)      GATES=(rust-test clippy clippy-desktop fmt outdated biome tsc vitest) ;;
  rust-test|clippy|clippy-desktop|fmt|outdated|biome|tsc|vitest) GATES=("$1") ;;
  *) echo "usage: $0 <commit|push|rust|frontend|all|<gate>> [out_dir]" >&2; exit 2 ;;
esac

OUT="${2:-${TMPDIR:-/tmp}/emailops-gates/$(basename "$ROOT")}"
mkdir -p "$OUT"

# Worktrees: share the main checkout's build output.
if [[ -z "${CARGO_TARGET_DIR:-}" && "$ROOT" == */.claude/worktrees/* ]]; then
  main_root="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"
  if [[ -d "$main_root/src-tauri/target" ]]; then
    export CARGO_TARGET_DIR
    CARGO_TARGET_DIR="$(cd "$main_root/src-tauri/target" && pwd -P)"
  fi
fi

# Put the .nvmrc Node major first on PATH. Prints nothing and returns 1 when
# no matching install exists.
node_bin_for_nvmrc() {
  local want dir
  want="$(tr -d ' v\n' < "$ROOT/.nvmrc")"
  want="${want%%.*}"
  if command -v node >/dev/null && [[ "$(node --version)" == "v$want."* ]]; then
    dirname "$(command -v node)"
    return 0
  fi
  for dir in "$HOME"/.nvm/versions/node/v"$want".*/bin \
             "$HOME"/.local/share/fnm/node-versions/v"$want".*/installation/bin; do
    if [[ -x "$dir/node" ]]; then echo "$dir"; fi
  done | sort -V | tail -n 1 | grep .
}

NODE_WANT="$(tr -d ' v\n' < "$ROOT/.nvmrc")"
NODE_BIN="$(node_bin_for_nvmrc)" || NODE_BIN=""

run_gate() {
  local gate="$1" file="$OUT/$1.txt"
  case "$gate" in
    rust-test)      cargo test --manifest-path src-tauri/Cargo.toml ;;
    clippy)         cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --tests -- -D warnings ;;
    clippy-desktop) cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --features desktop --tests -- -D warnings ;;
    fmt)            cargo fmt --manifest-path src-tauri/Cargo.toml -- --check ;;
    outdated)       cargo outdated --manifest-path src-tauri/Cargo.toml --root-deps-only --exit-code 1 --exclude rusqlite --ignore reqwest ;;
    biome)          PATH="$NODE_BIN:$PATH" ./node_modules/.bin/biome check src/ ;;
    tsc)            PATH="$NODE_BIN:$PATH" ./node_modules/.bin/tsc --noEmit ;;
    vitest)         PATH="$NODE_BIN:$PATH" ./node_modules/.bin/vitest run ;;
  esac > "$file" 2>&1
}

summary() {
  local gate="$1" f="$OUT/$1.txt"
  case "$gate" in
    rust-test) awk '/^test result:/{n++; p+=$4; f+=$6} END{print p " passed; " f " failed (" n " test binaries)"}' "$f" ;;
    clippy|clippy-desktop) awk '/^error: could not compile|Finished /' "$f" ;;
    fmt|tsc)   awk 'END{print NR " lines of output"}' "$f" ;;
    outdated)  awk 'NR>2{n++} END{print n+0 " outdated root dependencies"}' "$f" ;;
    biome)     awk '/Checked /' "$f" ;;
    vitest)    awk '/Tests /' "$f" ;;
  esac
}

excerpt() {
  awk '/^---- |panicked at|^error|^warning: unused|Diff in|error TS|FAIL |✖|×/' "$1" | head -n 20
}

failed=0
failing=()
printf '%-15s %-6s %-4s %s\n' gate status exit summary
for gate in "${GATES[@]}"; do
  reason=""
  case "$gate" in
    biome|tsc|vitest)
      if [[ ! -d node_modules ]]; then
        reason="node_modules missing (run npm ci)"
      elif [[ -z "$NODE_BIN" ]]; then
        reason="Node $NODE_WANT (.nvmrc) not found on PATH, nvm or fnm"
      fi ;;
    outdated)
      command -v cargo-outdated >/dev/null || reason="cargo-outdated not installed" ;;
  esac
  if [[ -n "$reason" ]]; then
    printf '%-15s %-6s %-4s %s\n' "$gate" skip - "$reason"
    failed=1
    continue
  fi
  run_gate "$gate"
  code=$?
  status=ok
  if [[ $code -ne 0 ]]; then
    status=FAIL
    failed=1
    failing+=("$gate")
  fi
  printf '%-15s %-6s %-4s %s\n' "$gate" "$status" "$code" "$(summary "$gate" | tr '\n' ' ')"
done

echo "output: $OUT"
for gate in "${failing[@]+"${failing[@]}"}"; do
  echo "--- $gate: first lines of interest ($OUT/$gate.txt) ---"
  excerpt "$OUT/$gate.txt"
done
exit "$failed"
