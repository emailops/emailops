#!/usr/bin/env bash
#
# Run the one-shot vs chat KV bench in one process, sampling the memory the
# Rust side cannot see: the harness reports per-scenario prefill and cache
# hits on stdout, while this wrapper samples RSS and picks llama.cpp's own
# buffer sizes out of stderr.
#
# Usage: scripts/oneshot_kv_bench.sh [extra args passed to the example]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="${EMAILOPS_DEMO_DIR:-$REPO_ROOT/.emailops-demo-data}"
OUT_DIR="$REPO_ROOT/reports/bench"
STAMP="$(date +%Y%m%d_%H%M%S)"
STDOUT_FILE="$OUT_DIR/oneshot_kv_${STAMP}.json"
STDERR_FILE="$OUT_DIR/oneshot_kv_${STAMP}.stderr.log"
RSS_FILE="$OUT_DIR/oneshot_kv_${STAMP}.rss.txt"
REPORT_FILE="$OUT_DIR/oneshot_kv_${STAMP}.report.json"

mkdir -p "$OUT_DIR"

# LLAMA_VERBOSE lets llama.cpp's INFO lines (KV / recurrent / compute buffer
# sizes) reach stderr, which is the only place they are ever printed.
LLAMA_VERBOSE=1 EMAILOPS_DATA_DIR="$DEMO_DIR" cargo run --manifest-path "$REPO_ROOT/src-tauri/Cargo.toml" \
  --features eval --example oneshot_kv_bench -- \
  --prod-db "$DEMO_DIR/emailops.db" "$@" \
  > "$STDOUT_FILE" 2> "$STDERR_FILE" &
BENCH_PID=$!

# Sample the resident set every second for as long as the bench runs. `ps`
# reports KiB; the summary converts to MiB.
while kill -0 "$BENCH_PID" 2>/dev/null; do
  ps -o rss= -p "$BENCH_PID" 2>/dev/null | tr -d ' ' >> "$RSS_FILE" || true
  sleep 1
done
wait "$BENCH_PID" || BENCH_STATUS=$?
BENCH_STATUS="${BENCH_STATUS:-0}"

python3 - "$STDOUT_FILE" "$STDERR_FILE" "$RSS_FILE" "$REPORT_FILE" <<'PY'
import json, re, sys

stdout_file, stderr_file, rss_file, report_file = sys.argv[1:5]

try:
    report = json.load(open(stdout_file, encoding="utf-8"))
except Exception as exc:  # the bench failed; keep the raw files for triage
    report = {"error": f"no JSON on stdout: {exc}"}

rss = [int(line) for line in open(rss_file, encoding="utf-8").read().split() if line.isdigit()] \
    if __import__("os").path.exists(rss_file) else []
report["rssMib"] = {
    "samples": len(rss),
    "peak": round(max(rss) / 1024, 1) if rss else None,
    "final": round(rss[-1] / 1024, 1) if rss else None,
}

# llama.cpp prints its buffer sizes once per model/context load.
buffers = []
pattern = re.compile(r"([\w .]*?(?:KV|kv_cache|recurrent|compute buffer|model buffer)[\w .]*?)\s*[:=]\s*([\d.]+)\s*MiB", re.I)
for line in open(stderr_file, encoding="utf-8", errors="replace"):
    m = pattern.search(line)
    if m:
        buffers.append({"what": m.group(1).strip(), "mib": float(m.group(2)), "line": line.strip()})
report["llamaBuffers"] = buffers

json.dump(report, open(report_file, "w", encoding="utf-8"), indent=2)
print(json.dumps({k: report[k] for k in ("rssMib", "llamaBuffers") if k in report}, indent=2))
for probe in report.get("probes", []):
    print(
        f"[kv-bench] {probe['scenario']:<28} prefill {probe.get('prefillMs')} ms · "
        f"cached {probe.get('cachedPromptTokens')} / {probe.get('promptTokens')} tokens · "
        f"plan {probe.get('prefixPlan')} · {probe['latencyMs']} ms"
    )
PY

echo "[kv-bench] report  → $REPORT_FILE"
echo "[kv-bench] stderr  → $STDERR_FILE"
exit "$BENCH_STATUS"
