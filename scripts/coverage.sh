#!/usr/bin/env bash
# Line/region coverage for the Rust backend (cargo-llvm-cov) and the frontend
# (vitest --coverage, v8 provider), plus a per-feature ranking.
#
# Usage: scripts/coverage.sh [rust|ts|all]        (make coverage / coverage-rust / coverage-ts)
#
# Rust is measured with `--no-default-features`, the feature set CI's Rust test
# job runs (ci.yml), so the numbers describe what CI actually tests. That build
# leaves out `commands/` (desktop) and `ai/llama_cpp/` (llamacpp), so neither
# appears in the report. Stable rustc has no branch coverage (`--branch` is
# nightly-only); region coverage is the closest stable proxy.
#
# Output (gitignored), under src-tauri/reports/coverage/:
#   rust/html/index.html  rust/summary.json  rust/lcov.info  rust/coverage.json (regions)
#   rust/test-output.txt
#   ts/index.html         ts/coverage-summary.json           ts/test-output.txt
#   by-feature.md         per-feature and per-file ranking (scripts/coverage_by_feature.py)
set -euo pipefail

cd "$(dirname "$0")/.."
WHAT="${1:-all}"
OUT="src-tauri/reports/coverage"
MANIFEST="src-tauri/Cargo.toml"
# Coverage build files belong to no test binary, the generated harness or the evals.
IGNORE='(^|/)src-tauri/(tests|examples|build\.rs)'
MIN_FREE_GB=12

rust() {
  local target free_gb
  target="$(cd src-tauri && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  free_gb="$(df -g "$target" | awk 'NR==2{print $4}')"
  if (( free_gb < MIN_FREE_GB )); then
    echo "coverage: only ${free_gb} GB free on $target (need ${MIN_FREE_GB}); the instrumented build would not fit" >&2
    exit 1
  fi
  mkdir -p "$OUT/rust"
  # Incremental state doubles the size of the instrumented target dir for no gain here.
  export CARGO_INCREMENTAL=0
  local status=0
  cargo llvm-cov --manifest-path "$MANIFEST" --no-default-features --no-report \
    > "$OUT/rust/test-output.txt" 2>&1 || status=$?
  echo "rust tests exit $status (full output: $OUT/rust/test-output.txt)"
  local report=(cargo llvm-cov report --manifest-path "$MANIFEST" --ignore-filename-regex "$IGNORE")
  "${report[@]}" --html --output-dir "$OUT/rust" > /dev/null
  "${report[@]}" --json --summary-only --output-path "$OUT/rust/summary.json"
  "${report[@]}" --lcov --output-path "$OUT/rust/lcov.info"
  "${report[@]}" --json --output-path "$OUT/rust/coverage.json"
  python3 - "$OUT/rust/summary.json" <<'PY'
import json, sys
t = json.load(open(sys.argv[1]))["data"][0]["totals"]
print("rust total: lines {:.1f}% ({}/{}), regions {:.1f}%, functions {:.1f}%".format(
    t["lines"]["percent"], t["lines"]["covered"], t["lines"]["count"],
    t["regions"]["percent"], t["functions"]["percent"]))
PY
  return "$status"
}

ts() {
  local node_bin status=0
  node_bin="$(ls -d "$HOME"/.nvm/versions/node/v"$(tr -d ' v\n' < .nvmrc | cut -d. -f1)".*/bin 2>/dev/null | sort -V | tail -n 1)"
  mkdir -p "$OUT/ts"
  NO_COLOR=1 PATH="${node_bin:+$node_bin:}$PATH" ./node_modules/.bin/vitest run --coverage \
    > "$OUT/ts-test-output.txt" 2>&1 || status=$?
  mv "$OUT/ts-test-output.txt" "$OUT/ts/test-output.txt"
  echo "vitest exit $status (full output: $OUT/ts/test-output.txt)"
  # istanbul's text-summary colours its lines even under NO_COLOR.
  perl -ne 's/\e\[[0-9;]*m//g; print if /^ *(Tests|Statements|Branches|Functions|Lines) /' "$OUT/ts/test-output.txt"
  return "$status"
}

status=0
case "$WHAT" in
  rust) rust || status=$? ;;
  ts)   ts || status=$? ;;
  all)  rust || status=$?; ts || status=$? ;;
  *) echo "usage: $0 [rust|ts|all]" >&2; exit 2 ;;
esac
python3 scripts/coverage_by_feature.py "$OUT"
echo "html: file://$PWD/$OUT/rust/html/index.html  file://$PWD/$OUT/ts/index.html"
exit "$status"
