# /// script
# dependencies = ["beautifulsoup4"]
# ///
"""Compare two eval-all runs side by side.

Reads timestamped reports under `src-tauri/reports/evaluations/` and writes a
markdown comparison table to stdout (or a file with `--out`).

Usage:
  uv run scripts/compare_evals.py \\
    --a-stamp 20260528_053707 --a-label "qwen3.5-4b" \\
    --b-stamp 20260528_064652 --b-label "qwen3.5-9b" \\
    --out reports/evaluations/comparison_4b_vs_9b.md

This is a one-off comparison helper; we intentionally avoid baking it into the
runner because we want the canonical eval outputs (HTML + JSON) to stay the
single source of truth.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Optional

from bs4 import BeautifulSoup

REPORTS_DIR = Path(__file__).resolve().parents[1] / "src-tauri" / "reports" / "evaluations"


def _find_report(subdir: str, glob: str, stamp_prefix: str) -> Optional[Path]:
    """Pick the report whose filename contains a stamp that starts with the
    requested prefix. Stamps may differ by a few seconds across suites in the
    same run because each eval finishes at its own time, so we pick the closest
    match by string comparison within the same date."""
    candidates = sorted((REPORTS_DIR / subdir).glob(glob))
    if not candidates:
        return None
    # Exact prefix wins; else nearest match by sortable string distance.
    same_day = [c for c in candidates if c.name.startswith(c.name.split("_")[0])]
    date_part = stamp_prefix.split("_")[0]
    same_date = [c for c in candidates if date_part in c.name]
    pool = same_date or candidates
    # Order by absolute "distance" of the stamp inside the filename.
    def score(p: Path) -> tuple:
        m = re.search(r"(\d{8}[_T]\d{6})", p.name)
        if not m:
            return (10**18,)
        return (abs(_stamp_key(m.group(1)) - _stamp_key(stamp_prefix)),)
    return min(pool, key=score)


def _stamp_key(s: str) -> int:
    return int(re.sub(r"[^0-9]", "", s))


def _classification_metrics(html_path: Path) -> dict:
    soup = BeautifulSoup(html_path.read_text(encoding="utf-8"), "html.parser")
    pills = [p.get_text(strip=True) for p in soup.select(".stats .pill")]
    total = ok = 0
    mean_ms = p95_ms = None
    for p in pills:
        if m := re.match(r"(\d+) total", p):
            total = int(m.group(1))
        elif m := re.match(r"(\d+) ok", p):
            ok = int(m.group(1))
        elif m := re.match(r"mean (\d+) ms", p):
            mean_ms = int(m.group(1))
        elif m := re.match(r"p95 (\d+) ms", p):
            p95_ms = int(m.group(1))
    return {"total": total, "ok": ok, "mean_ms": mean_ms, "p95_ms": p95_ms}


def _extraction_metrics(json_path: Path) -> dict:
    j = json.loads(json_path.read_text(encoding="utf-8"))
    total = j.get("total", 0)
    items = j.get("per_item_results", [])
    passed = sum(1 for it in items if it.get("passed"))
    skipped = sum(1 for it in items if (it.get("detail") or "").startswith("skipped"))
    errored = sum(1 for it in items if (it.get("detail") or "").startswith("error"))
    return {
        "total": total,
        "passed": passed,
        "skipped": skipped,
        "errored": errored,
        "pass_pct": (passed / total * 100) if total else 0.0,
    }


def _lens_metrics(json_path: Path) -> dict:
    j = json.loads(json_path.read_text(encoding="utf-8"))
    total = j.get("total", 0)
    succeeded = j.get("succeeded", 0)
    elapsed_ms = j.get("elapsed_ms", 0)
    # Structural completeness: how many cases had at least one non-null field?
    non_empty = 0
    for c in j.get("cases", []):
        data = c.get("data") or {}
        if any(v not in (None, "", [], {}) for v in data.values()):
            non_empty += 1
    return {
        "total": total,
        "succeeded": succeeded,
        "non_empty_data": non_empty,
        "elapsed_ms": elapsed_ms,
        "per_case_ms": elapsed_ms // total if total else 0,
    }


def _draft_metrics(json_path: Path) -> dict:
    cases = json.loads(json_path.read_text(encoding="utf-8"))
    if not cases:
        return {"n": 0}
    keys = ("style_match", "completeness", "tone_fit", "length_fit")
    sums = {k: 0.0 for k in keys}
    n = 0
    elapsed_ms = 0
    for c in cases:
        scores = c.get("scores") or {}
        if not scores:
            continue
        n += 1
        for k in keys:
            v = scores.get(k)
            if isinstance(v, (int, float)):
                sums[k] += v
        elapsed_ms += int(c.get("elapsed_ms") or 0)
    avg = {k: (sums[k] / n if n else 0.0) for k in keys}
    return {
        "n": n,
        "avg_style": avg["style_match"],
        "avg_completeness": avg["completeness"],
        "avg_tone": avg["tone_fit"],
        "avg_length": avg["length_fit"],
        "per_case_ms": (elapsed_ms // n) if n else 0,
    }


def _agent_search_metrics(html_path: Path) -> dict:
    """Aggregate row for each mode lives in the first <table class='summary'>."""
    soup = BeautifulSoup(html_path.read_text(encoding="utf-8"), "html.parser")
    table = soup.select_one("table.summary")
    out: dict[str, dict] = {}
    if not table:
        return out
    for row in table.select("tbody tr"):
        cells = [c.get_text(strip=True) for c in row.select("td")]
        if len(cells) < 5:
            continue
        mode = cells[0]
        out[mode] = {
            "p_at_15": cells[1],
            "r_at_15": cells[2],
            "f1_at_15": cells[3],
            "mrr": cells[4],
            "avg_latency_ms": int(re.sub(r"\D", "", cells[5])) if len(cells) > 5 else 0,
        }
    return out


def _chat_metrics(html_path: Path) -> dict:
    """Chat HTML lists each case as PASS / FAIL pills. We count those."""
    text = html_path.read_text(encoding="utf-8")
    soup = BeautifulSoup(text, "html.parser")
    statuses = [el.get_text(strip=True).upper() for el in soup.select(".status, .verdict, .pill")]
    passes = sum(1 for s in statuses if "PASS" in s or s == "OK")
    fails = sum(1 for s in statuses if "FAIL" in s)
    # Fall back to plain regex scan when the markup differs.
    if not passes and not fails:
        passes = len(re.findall(r"\bPASS\b", text))
        fails = len(re.findall(r"\bFAIL\b", text))
    return {"pass": passes, "fail": fails}


def _row(label: str, a, b):
    return f"| {label} | {a} | {b} |"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--a-stamp", required=True, help="Run-A stamp prefix (YYYYMMDD_HHMMSS).")
    ap.add_argument("--a-label", required=True)
    ap.add_argument("--b-stamp", required=True)
    ap.add_argument("--b-label", required=True)
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()

    out_lines: list[str] = []
    def emit(s: str = ""):
        out_lines.append(s)

    emit(f"# Eval comparison — {args.a_label} vs {args.b_label}")
    emit()
    emit(f"Run A stamp: `{args.a_stamp}`  •  Run B stamp: `{args.b_stamp}`")
    emit("")

    # ── classification ────────────────────────────────────────────────────
    a_path = _find_report("email_classification", "*.html", args.a_stamp)
    b_path = _find_report("email_classification", "*.html", args.b_stamp)
    if a_path and b_path:
        a = _classification_metrics(a_path)
        b = _classification_metrics(b_path)
        emit("## Email classification")
        emit("")
        emit("| Metric | " + args.a_label + " | " + args.b_label + " |")
        emit("|---|---|---|")
        emit(_row("Total / OK", f"{a['total']} / {a['ok']}", f"{b['total']} / {b['ok']}"))
        emit(_row("Mean latency (ms)", a["mean_ms"], b["mean_ms"]))
        emit(_row("P95 latency (ms)", a["p95_ms"], b["p95_ms"]))
        emit("")

    # ── extraction (tasks / facts) ────────────────────────────────────────
    for kind, prefix in (("tasks", "tasks_eval"), ("facts", "facts_eval")):
        a_path = _find_report("extraction", f"{prefix}_*.json", args.a_stamp)
        b_path = _find_report("extraction", f"{prefix}_*.json", args.b_stamp)
        if a_path and b_path:
            a = _extraction_metrics(a_path)
            b = _extraction_metrics(b_path)
            label = "Task extraction" if kind == "tasks" else "Memory (facts) extraction"
            emit(f"## {label}")
            emit("")
            emit("| Metric | " + args.a_label + " | " + args.b_label + " |")
            emit("|---|---|---|")
            emit(_row("Total", a["total"], b["total"]))
            emit(_row("Passed", a["passed"], b["passed"]))
            emit(_row("Skipped", a["skipped"], b["skipped"]))
            emit(_row("Errored", a["errored"], b["errored"]))
            emit(_row("Pass %", f"{a['pass_pct']:.1f}%", f"{b['pass_pct']:.1f}%"))
            emit("")

    # ── lens / invoices ───────────────────────────────────────────────────
    a_path = _find_report("lenses", "invoices_received_*.json", args.a_stamp)
    b_path = _find_report("lenses", "invoices_received_*.json", args.b_stamp)
    if a_path and b_path:
        a = _lens_metrics(a_path)
        b = _lens_metrics(b_path)
        emit("## Invoices received (lens extract)")
        emit("")
        emit("| Metric | " + args.a_label + " | " + args.b_label + " |")
        emit("|---|---|---|")
        emit(_row("Total", a["total"], b["total"]))
        emit(_row("Succeeded (no error)", a["succeeded"], b["succeeded"]))
        emit(_row("Cases with any non-null field", a["non_empty_data"], b["non_empty_data"]))
        emit(_row("Per-case latency (ms)", a["per_case_ms"], b["per_case_ms"]))
        emit("")

    # ── drafts ─────────────────────────────────────────────────────────────
    a_path = _find_report("drafts", "draft_eval_*.json", args.a_stamp)
    b_path = _find_report("drafts", "draft_eval_*.json", args.b_stamp)
    if a_path and b_path:
        a = _draft_metrics(a_path)
        b = _draft_metrics(b_path)
        emit("## Draft generation (judged 1–5)")
        emit("")
        emit("| Metric | " + args.a_label + " | " + args.b_label + " |")
        emit("|---|---|---|")
        emit(_row("Cases scored", a["n"], b["n"]))
        emit(_row("Avg style_match", f"{a['avg_style']:.2f}", f"{b['avg_style']:.2f}"))
        emit(_row("Avg completeness", f"{a['avg_completeness']:.2f}", f"{b['avg_completeness']:.2f}"))
        emit(_row("Avg tone_fit", f"{a['avg_tone']:.2f}", f"{b['avg_tone']:.2f}"))
        emit(_row("Avg length_fit", f"{a['avg_length']:.2f}", f"{b['avg_length']:.2f}"))
        emit(_row("Per-case latency (ms)", a["per_case_ms"], b["per_case_ms"]))
        emit("")

    # ── agent search ──────────────────────────────────────────────────────
    a_path = _find_report("private/agent_search", "*.html", args.a_stamp)
    b_path = _find_report("private/agent_search", "*.html", args.b_stamp)
    if a_path and b_path:
        a = _agent_search_metrics(a_path)
        b = _agent_search_metrics(b_path)
        emit("## Agent search (mean across cases, judge disabled)")
        emit("")
        emit("| Mode | Metric | " + args.a_label + " | " + args.b_label + " |")
        emit("|---|---|---|---|")
        for mode in sorted(set(a.keys()) | set(b.keys())):
            am = a.get(mode, {})
            bm = b.get(mode, {})
            for metric in ("p_at_15", "r_at_15", "f1_at_15", "mrr", "avg_latency_ms"):
                label = mode if metric == "p_at_15" else ""
                emit(f"| {label} | {metric} | {am.get(metric, '—')} | {bm.get(metric, '—')} |")
        emit("")

    # ── chat ──────────────────────────────────────────────────────────────
    a_path = _find_report("private/chat", "*.html", args.a_stamp)
    b_path = _find_report("private/chat", "*.html", args.b_stamp)
    if a_path and b_path:
        a = _chat_metrics(a_path)
        b = _chat_metrics(b_path)
        emit("## Chat (heuristic pass/fail per case)")
        emit("")
        emit("| Metric | " + args.a_label + " | " + args.b_label + " |")
        emit("|---|---|---|")
        emit(_row("PASS", a["pass"], b["pass"]))
        emit(_row("FAIL", a["fail"], b["fail"]))
        emit("")

    text = "\n".join(out_lines)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text, encoding="utf-8")
        print(f"[compare_evals] wrote {args.out}", file=sys.stderr)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
