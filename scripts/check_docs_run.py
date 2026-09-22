#!/usr/bin/env python3
"""Verify every claim the published docs make and write results.json.

Driven by scripts/check_docs.sh, which renders the JSON with the verify report
renderer — the same record shape as `make verify`, so the docs report gets the
per-page grouping, inline failures, screenshots and previous-run delta for free.

One record per claim (docs/site/claims.toml), grouped by page, typed by how
it was proven: against the running app, by existing tests, against source,
or not provable automatically (MANUAL — never counted as OK). Plus the
structural guards, which vouch for the claims as a set: parity across the four
languages, quoted labels, quoted paths, and completeness (every block of every
page is a catalogued claim).
"""

import json
import os
import pathlib
import subprocess
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from docs_claims_lib import pages  # noqa: E402
from docs_claims_verify import evaluate, page_title  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUN = pathlib.Path(os.environ["CHECK_DOCS_RUN"])
WITH_APP = os.environ.get("CHECK_DOCS_WITH_APP") == "1"
STRUCTURE = "Estructura de la doc"
records = []


def sh(cmd, timeout=1800):
    p = subprocess.run(cmd, shell=True, cwd=ROOT, capture_output=True, text=True, timeout=timeout)
    return p.returncode, (p.stdout + p.stderr).strip()


def first_problem(out):
    """The summary row wants the problem, not the closing advice.

    Every guard prints its findings as `  - <what is wrong>` and then a line
    telling you how to fix the class of thing. Taking the last line puts that
    generic advice in the table and leaves the reader opening each failure to
    learn which file it was actually about.
    """
    for line in out.splitlines():
        if line.lstrip().startswith("- "):
            return line.strip()[2:][:200]
    return out.splitlines()[-1][:200] if out.splitlines() else ""


def add(feature, typ, name, status, detail="", ms=None, desc="", **evidence):
    records.append(
        {
            "feature": feature,
            "type": typ,
            "name": name,
            "status": status,
            "detail": detail,
            "duration_ms": ms,
            "desc": desc,
            "evidence": {k: v for k, v in evidence.items() if v},
        }
    )


# ── structure: the guards that vouch for the claims as a set ────────────────
STATIC = [
    ("estructura en los 4 idiomas", "bash scripts/check-docs-parity.sh",
     "Mismas páginas, mismos weights de sidebar y mismas anclas {#id} en en/es/fr/de",
     "alinear páginas, weight o anclas entre idiomas"),
    ("etiquetas de UI citadas", "bash scripts/check-docs-labels.sh",
     "Cada etiqueta que los docs mandan pulsar existe literal en src/locales/<lang>/",
     "citar la cadena real de src/locales/<lang>/ en lugar de parafrasearla"),
    ("rutas de fichero citadas", "uv run --no-project scripts/check-docs-paths.py",
     "Cada ruta del repo entre backticks resuelve a un fichero que existe",
     "corregir la ruta, o añadirla a ALLOWED_UNRESOLVED si vive en otro repo"),
    ("cobertura completa de la doc", "uv run --no-project scripts/check-docs-claims.py",
     "Cada párrafo, viñeta, tabla y bloque de código lleva un marcador de afirmación, igual en los 4 idiomas, y cada marcador tiene su comprobación en claims.toml",
     "marcar el bloque nuevo y catalogarlo en docs/site/claims.toml"),
]
for name, cmd, desc, fix in STATIC:
    t0 = time.time()
    rc, out = sh(cmd)
    add(STRUCTURE, "static", name, "ok" if rc == 0 else "fail",
        "" if rc == 0 else first_problem(out), int((time.time() - t0) * 1000),
        desc=f"{desc} · Comando: {cmd}",
        claim=desc, trace="" if rc == 0 else out[-4000:], proposed_fix="" if rc == 0 else fix)

# ── the app, when asked: docClaim() cases ride the verify sweep ─────────────
app_results = None
shots_dir = None
if WITH_APP:
    rc, out = sh("bash scripts/verify_all.sh --only e2e")
    current = ROOT / "src-tauri/reports/verify/current-full/app/sweep"
    sweep = current / "results.json"
    app_results = {}
    if sweep.exists():
        shots_dir = current
        for r in json.loads(sweep.read_text()):
            if r.get("claim"):
                app_results[r["claim"]] = r
    else:
        add(STRUCTURE, "doc", "barrida de la app", "fail", "la barrida no dejó results.json",
            trace=out[-4000:])

# ── every claim ─────────────────────────────────────────────────────────────
t0 = time.time()
claim_records, test_log = evaluate(app_results)
MARK = {"ok": "✓", "fail": "✗", "manual": "○ manual", "none": "— sin afirmación", "pending": "… pendiente"}
for r in claim_records:
    lines = [f"{MARK.get(s, s)}  {why}" for _, s, why in r["results"]]
    fix = ""
    if r["status"] == "fail":
        fix = r.get("fix") or (
            "Corregir el párrafo en los 4 idiomas para que diga lo que hace el código, "
            "o actualizar su entrada en docs/site/claims.toml si el código cambió a propósito."
        )
    shots = []
    app = (app_results or {}).get(r["name"].rsplit(" · ", 1)[-1])
    if app and app.get("shot") and shots_dir:
        shot = shots_dir / app["shot"]
        if shot.exists():
            shots = [str(shot)]
    add(r["feature"], r["type"], r["name"], r["status"], r["detail"],
        desc=f"{r['page']}:{r['line']}", claim=r["claim"], page=r["page"],
        trace="\n".join(lines), proposed_fix=fix, shots=shots)
elapsed = int((time.time() - t0) * 1000)

feature_order = [STRUCTURE] + [page_title(p) for p in pages("en")]
data = {
    "meta": {
        "title": "Verificación de documentación de EmailOps",
        "eyebrow": "Documentación",
        "info_label": "Manual",
        "commit": sh("git rev-parse --short HEAD")[1],
        "branch": sh("git rev-parse --abbrev-ref HEAD")[1],
        "tier": "con la app" if WITH_APP else "sin la app",
    },
    "layers": [{"layer": "docs", "status": "ok", "seconds": round(elapsed / 1000, 1)}],
    "types": ["static", "doc", "tests", "source", "manual"],
    "features": feature_order,
    "records": records,
}
(RUN / "results.json").write_text(json.dumps(data, ensure_ascii=False, indent=1))
(RUN / "tests.log").write_text(test_log)

fails = [r for r in records if r["status"] == "fail"]
by = {s: sum(1 for r in records if r["status"] == s) for s in ("ok", "fail", "info", "skip")}
print(f"{len(records)} casos — {by['ok']} ok, {by['fail']} fallos, {by['info']} manuales, {by['skip']} sin afirmación")
for r in fails:
    print(f"  FALLO  {r['name']}: {r['detail'][:160]}")
sys.exit(1 if fails else 0)
