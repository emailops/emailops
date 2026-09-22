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

# ── the app, when asked: the ground truth ───────────────────────────────────
# Four phases (fresh install, locked relaunch, demo mailbox, CLI); a claim
# checked in several passes only if it passes in all of them.
app_results = None
if WITH_APP:
    app_dir = RUN / "app"
    rc, out = sh(f"bash scripts/check_docs_app.sh {app_dir}", timeout=3600)
    (RUN / "app.log").write_text(out)
    app_results = {}
    for phase in ("fresh", "locked", "demo", "cli"):
        f = app_dir / f"{phase}.json"
        if not f.exists():
            add(STRUCTURE, "doc", f"fase {phase} de la app", "fail", "la fase no dejó resultados", trace=out[-4000:])
            continue
        for r in json.loads(f.read_text()):
            if r["claim"] == "__launch__":
                add(STRUCTURE, "doc", f"fase {phase} de la app", "fail", r["detail"], trace=out[-4000:])
                continue
            prev = app_results.get(r["claim"])
            shots = [str(app_dir / s) for s in r.get("shots", [])]
            if prev is None:
                app_results[r["claim"]] = {**r, "shots": shots}
            else:
                order = {"fail": 2, "ok": 1, "skip": 0}
                worst = max(prev["status"], r["status"], key=lambda s: order[s])
                app_results[r["claim"]] = {
                    "claim": r["claim"], "status": worst,
                    "detail": f"{prev['detail']} · {phase}: {r['detail']}",
                    "shots": prev["shots"] + shots,
                    "fix": prev.get("fix") or r.get("fix", ""),
                }

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
    # Evidence for a failure or a manual call: the screens the app showed.
    shots = [s for s in r.get("shots", []) if pathlib.Path(s).exists()] if r["status"] in ("fail", "info") else []
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
    "types": ["static", "doc", "release", "tests", "source", "manual"],
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
