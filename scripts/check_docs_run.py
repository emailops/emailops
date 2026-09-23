#!/usr/bin/env python3
"""Verify every claim the published docs make and write results.json.

Driven by scripts/check_docs.sh; scripts/docs_report.py renders the JSON as
the docs themselves, each fragment coloured by what validated it.

Three sources of validation, merged per fragment (docs_claims_verify.py):
the structural guards (parity, labels, paths, completeness — they vouch for
the docs as a set and head the report), the catalog checks in
docs/site/claims.toml, and the app itself — four phases driven by
scripts/check_docs_app.sh, one record per case. The agent's judgments
(judge/judgments.json in the run dir) are merged in when present.

CHECK_DOCS_RENDER_ONLY=1 re-evaluates an existing run dir without driving the
app again: the app records under <run>/app are reused, so the judgments the
agent adds after a run can be folded into the report.
"""

import datetime
import json
import os
import pathlib
import subprocess
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from docs_claims_verify import evaluate  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUN = pathlib.Path(os.environ["CHECK_DOCS_RUN"])
WITH_APP = os.environ.get("CHECK_DOCS_WITH_APP") == "1"
RENDER_ONLY = os.environ.get("CHECK_DOCS_RENDER_ONLY") == "1"
PHASES = tuple(os.environ.get("DOCS_PHASES", "fresh locked demo cli").split())
structure = []


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


# ── structure: the guards that vouch for the docs as a set ──────────────────
STATIC = [
    ("Estructura en los 4 idiomas", "bash scripts/check-docs-parity.sh",
     "Mismas páginas, mismos weights de sidebar y mismas anclas {#id} en en/es/fr/de.",
     "Alinear páginas, weight o anclas entre idiomas."),
    ("Etiquetas de UI citadas", "bash scripts/check-docs-labels.sh",
     "Cada etiqueta que la doc manda pulsar existe literal en src/locales/<lang>/.",
     "Citar la cadena real de src/locales/<lang>/ en lugar de parafrasearla."),
    ("Rutas de fichero citadas", "uv run --no-project scripts/check-docs-paths.py",
     "Cada ruta del repo entre backticks resuelve a un fichero que existe.",
     "Corregir la ruta, o añadirla a ALLOWED_UNRESOLVED si vive en otro repo."),
    ("Cobertura completa de la doc", "uv run --no-project scripts/check-docs-claims.py",
     "Cada párrafo, viñeta, tabla y bloque de código lleva un marcador, igual en los 4 idiomas, y cada marcador tiene su entrada en claims.toml.",
     "Marcar el bloque nuevo y catalogarlo en docs/site/claims.toml."),
]
for name, cmd, desc, fix in STATIC:
    t0 = time.time()
    rc, out = sh(cmd)
    structure.append({"name": name, "status": "ok" if rc == 0 else "fail", "how": desc, "cmd": cmd,
                      "detail": "" if rc == 0 else first_problem(out), "fix": "" if rc == 0 else fix,
                      "trace": "" if rc == 0 else out[-4000:], "ms": int((time.time() - t0) * 1000)})

# ── the app: the ground truth ───────────────────────────────────────────────
app_dir = RUN / "app"
ran = RENDER_ONLY and app_dir.exists() or WITH_APP
if WITH_APP and not RENDER_ONLY:
    rc, out = sh(f"bash scripts/check_docs_app.sh {app_dir}", timeout=3600)
    (RUN / "app.log").write_text(out)
app_parts = {}
if ran:
    log = (RUN / "app.log").read_text() if (RUN / "app.log").exists() else ""
    for phase in PHASES:
        f = app_dir / f"{phase}.json"
        if not f.exists():
            structure.append({"name": f"Fase «{phase}» de la app", "status": "fail", "how": "La fase debe dejar sus resultados.",
                              "detail": "la fase no dejó resultados", "fix": "Ver app.log.", "trace": log[-4000:], "cmd": ""})
            continue
        for r in json.loads(f.read_text()):
            if r["claim"] == "__launch__":
                structure.append({"name": f"Fase «{phase}» de la app", "status": "fail", "how": "La instancia debe arrancar.",
                                  "detail": r["detail"], "fix": "Ver app.log.", "trace": log[-4000:], "cmd": ""})
                continue
            r["shots"] = [f"app/{s}" for s in r.get("shots", []) if (app_dir / s).exists()]
            r["phase"] = phase
            app_parts.setdefault(r["claim"], []).append(r)

judgments = []
jf = RUN / "judge" / "judgments.json"
if jf.exists():
    judgments = json.loads(jf.read_text())

complete = set(PHASES) >= {"fresh", "locked", "demo", "cli"}
pages, test_log = evaluate(app_parts, ran, judgments, complete)
data = {
    "meta": {
        "title": "Verificación de documentación de EmailOps",
        "commit": sh("git rev-parse --short HEAD")[1],
        "branch": sh("git rev-parse --abbrev-ref HEAD")[1],
        "date": datetime.datetime.now().strftime("%Y-%m-%d %H:%M"),
        "with_app": bool(ran),
        "judged": bool(judgments),
    },
    "structure": structure,
    "pages": pages,
}
(RUN / "results.json").write_text(json.dumps(data, ensure_ascii=False, indent=1))
(RUN / "tests.log").write_text(test_log)

frags = [f for p in pages for it in p["items"] for f in it.get("fragments", []) if f["color"] != "none"]
by = {c: sum(1 for f in frags if f["color"] == c) for c in ("green", "yellow", "red")}
bad = [s for s in structure if s["status"] == "fail"]
print(f"{len(frags)} fragmentos — {by['green']} verdes, {by['yellow']} amarillos, {by['red']} rojos"
      + (f"; {len(bad)} guardas de estructura en rojo" if bad else ""))
for p in pages:
    for it in p["items"]:
        for f in it.get("fragments", []):
            if f["color"] == "red":
                why = next((v["detail"] for v in f["validations"] if v["state"] in ("fail", "contradicted")), "")
                print(f"  ROJO  {p['page']} [{it['claim']}] {f['text'][:70]!r}: {why[:120]}")
sys.exit(1 if by["red"] or bad else 0)
