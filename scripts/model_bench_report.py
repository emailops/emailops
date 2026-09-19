#!/usr/bin/env python3
"""Render the model benchmark collected by scripts/model_bench.sh.

Reads the per-model, per-eval artefacts from the work directory and writes a
plain-text table and an HTML page beside it. Every number comes from a file
the harness produced; anything missing prints as `n/a` rather than being
guessed.
"""

import json
import os
import re
import sys


def load(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:
        return None


def first_json_object(path):
    """The JSON object a harness printed on stdout, ignoring any prose."""
    try:
        text = open(path, encoding="utf-8").read()
    except OSError:
        return None
    start = text.find("{")
    if start < 0:
        return None
    try:
        return json.loads(text[start:])
    except json.JSONDecodeError:
        return None


def pct(value):
    return "n/a" if value is None else "{:.1f}%".format(value * 100)


def num(value, unit="", digits=0):
    return "n/a" if value is None else "{:.{}f}{}".format(value, digits, unit)


def mib(kib):
    return "n/a" if not kib else "{:.0f} MiB".format(kib / 1024)


def ratio(hit, total):
    return None if not total else hit / total


def collect(work, model):
    safe = re.sub(r"[^a-zA-Z0-9_.-]", "_", model)
    out = {"model": model}

    classify = first_json_object(os.path.join(work, f"{safe}_classify.out"))
    meta = load(os.path.join(work, f"{safe}_classify.meta.json")) or {}
    if classify:
        out["classify"] = {
            axis: {
                "strict": ratio(classify[axis]["strict"], classify[axis]["total"]),
                "accepted": ratio(classify[axis]["accepted"], classify[axis]["total"]),
                "macroF1": classify[axis]["macroF1"],
            }
            for axis in ("intent", "topic", "urgency")
        }
        out["classify"]["all_axes"] = ratio(classify["allAxesAccepted"], classify["totalCases"])
        out["classify"]["latency_ms"] = classify["latencyMsMean"]
        out["classify"]["emails_per_min"] = classify["emailsPerMinute"]
        out["classify"]["cases"] = classify["totalCases"]
    out["classify_meta"] = meta

    plan = first_json_object(os.path.join(work, f"{safe}_plan.out"))
    meta = load(os.path.join(work, f"{safe}_plan.meta.json")) or {}
    if plan:
        out["plan"] = {
            "passed": ratio(plan["passed"], plan["totalCases"]),
            "cases": plan["totalCases"],
            "unparseable": plan["unparseable"],
            "latency_ms": plan["latencyMsMean"],
            "p95_ms": plan["latencyMsP95"],
            "prefill_ms": plan["prefillMsMean"],
        }
    out["plan_meta"] = meta

    # chat_eval writes an HTML report and per-case lines on stderr, no JSON —
    # so the pass rate is counted from those lines rather than invented.
    meta = load(os.path.join(work, f"{safe}_chat.meta.json")) or {}
    try:
        chat_log = open(os.path.join(work, f"{safe}_chat.err"), encoding="utf-8", errors="replace").read()
    except OSError:
        chat_log = ""
    verdicts = re.findall(r"^\[eval\]\s+(OK|FAIL)\s", chat_log, re.M)
    if verdicts:
        out["chat"] = {
            "passed": ratio(verdicts.count("OK"), len(verdicts)),
            "cases": len(verdicts),
        }
    out["chat_meta"] = meta

    drafts = load(os.path.join(work, f"{safe}_drafts.report.json"))
    meta = load(os.path.join(work, f"{safe}_drafts.meta.json")) or {}
    if drafts:
        rows = drafts if isinstance(drafts, list) else drafts.get("cases", [])
        done = [r for r in rows if not r.get("error")]
        overlap = [r["word_overlap"] for r in done if r.get("word_overlap") is not None]
        elapsed = [r["elapsed_ms"] for r in done if r.get("elapsed_ms") is not None]
        out["drafts"] = {
            "cases": len(rows),
            "errors": len(rows) - len(done),
            "word_overlap": sum(overlap) / len(overlap) if overlap else None,
            "latency_ms": sum(elapsed) / len(elapsed) if elapsed else None,
        }
    out["drafts_meta"] = meta
    return out


ROWS = [
    ("Clasificador — intent estricto", lambda d: pct(d.get("classify", {}).get("intent", {}).get("strict"))),
    ("Clasificador — intent aceptado", lambda d: pct(d.get("classify", {}).get("intent", {}).get("accepted"))),
    ("Clasificador — topic estricto", lambda d: pct(d.get("classify", {}).get("topic", {}).get("strict"))),
    ("Clasificador — urgency estricto", lambda d: pct(d.get("classify", {}).get("urgency", {}).get("strict"))),
    ("Clasificador — 3 ejes correctos", lambda d: pct(d.get("classify", {}).get("all_axes"))),
    ("Clasificador — ms/correo", lambda d: num(d.get("classify", {}).get("latency_ms"), " ms")),
    ("Clasificador — correos/min", lambda d: num(d.get("classify", {}).get("emails_per_min"), "", 1)),
    ("Clasificador — RSS pico", lambda d: mib(d.get("classify_meta", {}).get("rssKib"))),
    ("Planner — casos que pasan", lambda d: pct(d.get("plan", {}).get("passed"))),
    ("Planner — no parseables", lambda d: str(d.get("plan", {}).get("unparseable", "n/a"))),
    ("Planner — latencia media", lambda d: num(d.get("plan", {}).get("latency_ms"), " ms")),
    ("Planner — p95", lambda d: num(d.get("plan", {}).get("p95_ms"), " ms")),
    ("Planner — prefill medio", lambda d: num(d.get("plan", {}).get("prefill_ms"), " ms")),
    ("Planner — RSS pico", lambda d: mib(d.get("plan_meta", {}).get("rssKib"))),
    ("Chat — casos que pasan", lambda d: pct(d.get("chat", {}).get("passed"))),
    ("Chat — casos ejecutados", lambda d: str(d.get("chat", {}).get("cases", "n/a"))),
    ("Chat — tiempo total", lambda d: num(d.get("chat_meta", {}).get("wallMs"), " ms")),
    ("Chat — RSS pico", lambda d: mib(d.get("chat_meta", {}).get("rssKib"))),
    ("Drafts — solape de palabras", lambda d: num(d.get("drafts", {}).get("word_overlap"), "", 3)),
    ("Drafts — ms/borrador", lambda d: num(d.get("drafts", {}).get("latency_ms"), " ms")),
    ("Drafts — errores", lambda d: str(d.get("drafts", {}).get("errors", "n/a"))),
    ("Drafts — RSS pico", lambda d: mib(d.get("drafts_meta", {}).get("rssKib"))),
]

CAVEAT = (
    "draft_eval juzga con el mismo modelo que genera, asi que sus notas del juez no son "
    "comparables entre modelos y no se incluyen: solo van las metricas deterministas."
)


def main():
    work, models_arg, tier, repeats = sys.argv[1:5]
    models = [m for m in models_arg.split(",") if m]
    data = [collect(work, m) for m in models]

    width = max(34, *(len(r[0]) for r in ROWS))
    col = max(22, *(len(m) for m in models))

    lines = [
        "Benchmark de modelos — EmailOps",
        "",
        f"tier de chat: {tier} · repeticiones del clasificador: {repeats}",
        "",
        "| " + "metrica".ljust(width) + " | " + " | ".join(m.ljust(col) for m in models) + " |",
        "|" + "-" * (width + 2) + "|" + "|".join("-" * (col + 2) for _ in models) + "|",
    ]
    for label, fn in ROWS:
        lines.append(
            "| " + label.ljust(width) + " | " + " | ".join(fn(d).ljust(col) for d in data) + " |"
        )
    lines += ["", CAVEAT, ""]
    text = "\n".join(lines)

    base = work.rstrip("/")
    open(base + ".txt", "w", encoding="utf-8").write(text)
    print(text)

    def cells(fn):
        return "".join(f"<td>{fn(d)}</td>" for d in data)

    html_rows = "".join(f"<tr><th>{label}</th>{cells(fn)}</tr>" for label, fn in ROWS)
    heads = "".join(f"<th>{m}</th>" for m in models)
    html = f"""<!doctype html>
<html lang="es"><head><meta charset="utf-8">
<title>Benchmark de modelos — EmailOps</title>
<style>
 body{{background:#111418;color:#e6e6e6;font:14px/1.5 -apple-system,Segoe UI,sans-serif;margin:0;padding:24px}}
 h1{{font-size:20px;margin:0 0 4px}} .sub{{color:#8a94a0;margin-bottom:20px}}
 table{{border-collapse:collapse;width:100%;max-width:1100px}}
 th,td{{border-bottom:1px solid #222a33;padding:7px 10px;text-align:left}}
 thead th{{color:#8a94a0;font-size:12px;text-transform:uppercase;letter-spacing:.04em}}
 tbody th{{font-weight:500;color:#c7ced6}}
 td{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace}}
 .note{{color:#8a94a0;margin-top:20px;max-width:1100px}}
</style></head><body>
<h1>Benchmark de modelos — EmailOps</h1>
<div class="sub">tier de chat: {tier} · repeticiones del clasificador: {repeats}</div>
<table><thead><tr><th>métrica</th>{heads}</tr></thead><tbody>{html_rows}</tbody></table>
<p class="note">{CAVEAT}</p>
</body></html>"""
    open(base + ".html", "w", encoding="utf-8").write(html)


if __name__ == "__main__":
    main()
