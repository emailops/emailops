#!/usr/bin/env bash
#
# Before/after report for the one-shot prefix slot.
#
# Runs the query-planner eval twice from the SAME build — once with the slot
# off (`EMAILOPS_AUX_PREFIX=0`, which is the old path exactly) and once with it
# on — plus the classifier eval, and writes a markdown table. Running both
# halves from one build is the point: it isolates the slot from model, machine
# and corpus drift.
#
# Usage: scripts/oneshot_report.sh [--repeats N]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="${EMAILOPS_DEMO_DIR:-$REPO_ROOT/.emailops-demo-data}"
OUT_DIR="$REPO_ROOT/reports/bench"
STAMP="$(date +%Y%m%d_%H%M%S)"
WORK="$OUT_DIR/oneshot_report_${STAMP}"
REPORT="$WORK.md"
REPEATS=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repeats) REPEATS="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT_DIR"

plan_eval() {
  # $1 = output file, rest = environment assignments
  local out="$1"; shift
  env "$@" EMAILOPS_DATA_DIR="$DEMO_DIR" \
    cargo run --quiet --manifest-path "$REPO_ROOT/src-tauri/Cargo.toml" \
    --features eval --example query_plan_eval -- \
    --prod-db "$DEMO_DIR/emailops.db" --account ulises@emailopslabs.dev --json \
    > "$out"
}

echo "[report] planner, prefix slot OFF"
plan_eval "${WORK}_plan_before.json" EMAILOPS_AUX_PREFIX=0
echo "[report] planner, prefix slot ON"
plan_eval "${WORK}_plan_after.json" EMAILOPS_AUX_PREFIX=1

echo "[report] classifier"
EMAILOPS_DATA_DIR="$DEMO_DIR" cargo run --quiet --manifest-path "$REPO_ROOT/src-tauri/Cargo.toml" \
  --features eval --example tag_classification_eval -- \
  --prod-db "$DEMO_DIR/emailops.db" --json --repeats "$REPEATS" \
  > "${WORK}_classify.json"

python3 - "${WORK}_plan_before.json" "${WORK}_plan_after.json" "${WORK}_classify.json" "$REPORT" <<'PY'
import json, sys

before_path, after_path, classify_path, out_path = sys.argv[1:5]
before = json.load(open(before_path, encoding="utf-8"))
after = json.load(open(after_path, encoding="utf-8"))
classify = json.load(open(classify_path, encoding="utf-8"))


def pct(value):
    return "n/a" if value is None else "{:.1f}%".format(value * 100)


def num(value, unit=""):
    return "n/a" if value is None else "{:.0f}{}".format(value, unit)


def one(value, digits=3):
    return "n/a" if value is None else "{:.{}f}".format(value, digits)


def row(label, key, unit=""):
    return "| {} | {} | {} |".format(label, num(before[key], unit), num(after[key], unit))


lines = [
    "# One-shot prefix slot - before / after",
    "",
    "Model: `{}` - planner cases: {} - classifier cases: {} x {} repeat(s)".format(
        after["model"], after["totalCases"], classify["totalCases"], classify["repeats"]
    ),
    "",
    "Both planner columns come from the same build; the left one runs with",
    "`EMAILOPS_AUX_PREFIX=0`, which is the pre-change path exactly.",
    "",
    "## Query planner",
    "",
    "| | slot off | slot on |",
    "|---|---|---|",
    "| cases passed | {}/{} | {}/{} |".format(
        before["passed"], before["totalCases"], after["passed"], after["totalCases"]
    ),
    "| search / defer | {} / {} | {} / {} |".format(
        before["searched"], before["deferred"], after["searched"], after["deferred"]
    ),
    "| unparseable | {} | {} |".format(before["unparseable"], after["unparseable"]),
    row("latency mean", "latencyMsMean", " ms"),
    row("latency p50", "latencyMsP50", " ms"),
    row("latency p95", "latencyMsP95", " ms"),
    row("prefill mean", "prefillMsMean", " ms"),
    row("prompt tokens", "promptTokensMean"),
    "| prefix reused / reseeded / bypassed | {} / {} / {} | {} / {} / {} |".format(
        before["prefixReused"], before["prefixReseeded"], before["prefixBypassed"],
        after["prefixReused"], after["prefixReseeded"], after["prefixBypassed"],
    ),
    "",
    "## Classifier",
    "",
    "Unaffected by the slot - its prefill is milliseconds. Listed so a prompt or",
    "model change shows up against the same corpus.",
    "",
    "| axis | strict | accepted | macro-F1 |",
    "|---|---|---|---|",
]

for axis in ("intent", "topic", "urgency"):
    scores = classify[axis]
    total = scores["total"] or None
    strict = scores["strict"] / total if total else None
    accepted = scores["accepted"] / total if total else None
    lines.append("| {} | {} | {} | {} |".format(axis, pct(strict), pct(accepted), one(scores["macroF1"])))

lines += [
    "",
    "All three axes accepted on {}/{} emails - repaired {} - silent fallbacks {} - unparseable {}".format(
        classify["allAxesAccepted"], classify["totalCases"], classify["repairedCases"],
        classify["fallbackCases"], classify["unparseableReplies"],
    ),
    "",
    "Latency mean {} - p50 {} - p95 {} - {} emails/min".format(
        num(classify["latencyMsMean"], " ms"),
        num(classify["latencyMsP50"], " ms"),
        num(classify["latencyMsP95"], " ms"),
        one(classify["emailsPerMinute"], 1),
    ),
    "",
]

open(out_path, "w", encoding="utf-8").write("\n".join(lines) + "\n")
print("\n".join(lines))
PY

echo "[report] written → $REPORT"
