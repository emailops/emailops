#!/usr/bin/env python3
"""Interleaved prefill-speed A/B between two emailops-cli builds on the demo DB.

Runs control, candidate, control, candidate, ..., control. Each run is one
process asking the same panel of questions in fresh conversations. Every
candidate run is compared with the mean of the two control runs around it, so
drift over the session (thermals, background load) cancels out. Three
candidate runs settle a clear difference; when they cannot, three more run.

Usage:
  bench_ab.py --control NAME=CLI --candidate NAME=CLI --model ID

  CLI is an emailops-cli binary. To A/B a code change, build the CLI before and
  after it and copy each binary out of src-tauri/target, since the next build
  overwrites it:
    cargo build --manifest-path src-tauri/Cargo.toml --features cli --bin emailops-cli
    cp src-tauri/target/debug/emailops-cli <scratch>/cli-before

Per run, from `chat --fresh --json --trace` without prewarm, so the first
answer prefills the system prefix cold:
  long    median prefill tok/s of calls with >= 4096 uncached tokens (cold prefix)
  short   median prefill tok/s of calls with 512-4095 (one question's sources)
  peak    max RSS of the process, in GB
  wall    wall-clock seconds
Verdict per prefill metric: better (worse) when every candidate run beats
(trails) both of its neighbours and the median delta clears 3%; noise otherwise.

Refuses to start, and stops mid-way, when scripts/bench_preflight.sh fails.
Writes src-tauri/reports/bench/ab_<stamp>.json.
"""

from __future__ import annotations  # list[...] / dict[...] hints on the system python3 `make` runs

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MIN_UNCACHED = 512  # smaller prefills are dominated by fixed per-call overhead
LONG_UNCACHED = 4096
BAR = 0.03
FIRST_RUNS = 3
MAX_RUNS = 6
PANEL = (
    "What invoices have I received recently?",
    "Summarize what my unread emails are about",
    "Which emails still need a reply from me?",
)


def prefill_rates(envelope: dict) -> list[tuple[int, float]]:
    """(uncached tokens, prefill tok/s) of each LLM call that prefilled >= MIN_UNCACHED new tokens."""
    rates = []
    for turn in envelope["data"]["turns"]:
        for call in turn["trace"]["llmCalls"]:
            uncached = (call.get("promptTokens") or 0) - (call.get("cachedPromptTokens") or 0)
            ms = call.get("prefillMs")
            if call.get("failed") or not ms or uncached < MIN_UNCACHED:
                continue
            rates.append((uncached, uncached * 1000 / ms))
    return rates


def bucket_medians(rates: list[tuple[int, float]]) -> dict[str, float]:
    """Median tok/s of the long (>= LONG_UNCACHED uncached tokens) and short prefills."""
    buckets = {
        "long": [r for n, r in rates if n >= LONG_UNCACHED],
        "short": [r for n, r in rates if n < LONG_UNCACHED],
    }
    return {name: statistics.median(values) for name, values in buckets.items() if values}


def verdict(controls: list[float], candidates: list[float], bar: float = BAR) -> tuple[str, list[float]]:
    """Judge candidate runs K1..Kn against controls C0..Cn, run as C0 K1 C1 ... Kn Cn."""
    deltas = [k / ((controls[i] + controls[i + 1]) / 2) - 1 for i, k in enumerate(candidates)]
    median = statistics.median(deltas)
    if all(d > 0 for d in deltas) and median > bar:
        return "better", deltas
    if all(d < 0 for d in deltas) and median < -bar:
        return "worse", deltas
    return "noise", deltas


def judge(controls: list[dict], candidates: list[dict]) -> dict[str, tuple[str, list[float]]]:
    """Verdict for each prefill metric every run measured."""
    runs = controls + candidates
    return {
        metric: verdict([r[metric] for r in controls], [r[metric] for r in candidates])
        for metric in ("long", "short")
        if all(metric in r for r in runs)
    }


def preflight() -> None:
    if subprocess.run(["bash", str(REPO / "scripts" / "bench_preflight.sh")]).returncode != 0:
        sys.exit("[bench-ab] preflight refused the run; fix the above and start again")


def run_once(cli: str, model: str, demo_dir: Path, models_dir: Path) -> dict:
    account = os.environ.get("EMAILOPS_BENCH_ACCOUNT", "ulises@emailopslabs.dev")
    with tempfile.TemporaryDirectory() as tmp:
        data = Path(tmp)
        for f in demo_dir.glob("emailops.db*"):
            shutil.copy(f, data)
        (data / "models").symlink_to(models_dir)
        cmd = ["/usr/bin/time", "-l", cli, "--data-dir", str(data), "--account", account,
               "--model", model, "--json", "chat", "--trace", "--fresh", *PANEL]
        start = time.monotonic()
        proc = subprocess.run(cmd, capture_output=True, text=True)
        wall = time.monotonic() - start
    try:
        envelope = json.loads(proc.stdout)
    except json.JSONDecodeError:
        sys.exit(f"[bench-ab] {cli} printed no JSON envelope (exit {proc.returncode}):\n{proc.stderr[-2000:]}")
    if not envelope.get("ok"):
        sys.exit(f"[bench-ab] {cli} failed: {json.dumps(envelope.get('error'))}")
    rss = [line for line in proc.stderr.splitlines() if "maximum resident set size" in line]
    peak = int(rss[-1].split()[0]) / 1e9 if rss else None
    return {"wall_s": wall, "peak_gb": peak, **bucket_medians(prefill_rates(envelope))}


def arm(value: str) -> tuple[str, str]:
    name, sep, cli = value.partition("=")
    if not sep or not os.access(cli, os.X_OK):
        raise argparse.ArgumentTypeError(f"expected NAME=CLI with an executable CLI, got {value!r}")
    return name, cli


def median_of(runs: list[dict], key: str) -> float | None:
    values = [r[key] for r in runs if r.get(key) is not None]
    return statistics.median(values) if values else None


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--control", required=True, type=arm, metavar="NAME=CLI")
    ap.add_argument("--candidate", required=True, type=arm, metavar="NAME=CLI")
    ap.add_argument("--model", required=True)
    args = ap.parse_args()

    demo_dir = Path(os.environ.get("EMAILOPS_DEMO_DIR", REPO / ".emailops-demo-data"))
    default_models = Path.home() / "Library/Application Support/com.emailops.app/models"
    models_dir = Path(os.environ.get("MODELS_DIR", default_models))
    if not (demo_dir / "emailops.db").is_file():
        sys.exit(f"[bench-ab] no demo DB at {demo_dir}: run make demo-db")

    (ctl_name, ctl_cli), (cand_name, cand_cli) = args.control, args.candidate
    controls: list[dict] = []
    candidates: list[dict] = []
    sequence: list[str] = []

    def run(name: str, cli: str, into: list[dict]) -> None:
        preflight()
        print(f"[bench-ab] run {len(sequence) + 1}: {name}", file=sys.stderr, flush=True)
        into.append(run_once(cli, args.model, demo_dir, models_dir))
        sequence.append(name)

    run(ctl_name, ctl_cli, controls)
    target = FIRST_RUNS
    while True:
        while len(candidates) < target:
            run(cand_name, cand_cli, candidates)
            run(ctl_name, ctl_cli, controls)
        verdicts = judge(controls, candidates)
        if target == MAX_RUNS or all(v != "noise" for v, _ in verdicts.values()):
            break
        target = MAX_RUNS

    print(f"{'metric':<16}{ctl_name:>12}{cand_name:>12}{'delta':>10}  verdict")
    for metric, (result, deltas) in verdicts.items():
        ctl_v, cand_v = median_of(controls, metric), median_of(candidates, metric)
        print(f"{metric + ' tok/s':<16}{ctl_v:>12.0f}{cand_v:>12.0f}{statistics.median(deltas):>+10.1%}  {result}")
    for key, label in (("peak_gb", "peak GB"), ("wall_s", "wall s")):
        ctl_v, cand_v = median_of(controls, key), median_of(candidates, key)
        if ctl_v is not None and cand_v is not None:
            print(f"{label:<16}{ctl_v:>12.2f}{cand_v:>12.2f}{cand_v - ctl_v:>+10.2f}")

    out = REPO / "src-tauri" / "reports" / "bench" / f"ab_{time.strftime('%Y%m%d_%H%M%S')}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    report = {
        "model": args.model,
        "panel": PANEL,
        "control": {"name": ctl_name, "cli": ctl_cli, "runs": controls},
        "candidate": {"name": cand_name, "cli": cand_cli, "runs": candidates},
        "sequence": sequence,
        "verdicts": {m: {"verdict": v, "deltas": d} for m, (v, d) in verdicts.items()},
    }
    out.write_text(json.dumps(report, indent=2) + "\n")
    print(f"[bench-ab] report: {out}", file=sys.stderr)


if __name__ == "__main__":
    main()
