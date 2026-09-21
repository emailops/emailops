#!/usr/bin/env python3
"""Run the documentation checks and write results.json in the verify schema.

Driven by scripts/check_docs.sh, which renders the JSON with the verify
report renderer. Writing the same record shape means the docs report gets the
per-feature grouping, inline failure expansion, embedded screenshots and
previous-run delta for free, and a docs run sitting inside `make verify` reads
identically to one run on its own.
"""

import json
import os
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUN = pathlib.Path(os.environ["CHECK_DOCS_RUN"])
WITH_APP = os.environ.get("CHECK_DOCS_WITH_APP") == "1"

FEATURE = "Documentación publicada"
records = []


def sh(cmd, timeout=1800):
    p = subprocess.run(cmd, shell=True, cwd=ROOT, capture_output=True, text=True, timeout=timeout)
    return p.returncode, (p.stdout + p.stderr).strip()


def add(typ, name, status, detail="", ms=None, desc="", **evidence):
    records.append(
        {
            "feature": FEATURE,
            "type": typ,
            "name": name,
            "status": status,
            "detail": detail,
            "ms": ms,
            "desc": desc,
            "evidence": {k: v for k, v in evidence.items() if v},
        }
    )


# ── static: the cheap guards ────────────────────────────────────────────────
STATIC = [
    (
        "estructura en los 4 idiomas",
        "bash scripts/check-docs-parity.sh",
        "Mismas páginas, mismos weights de sidebar y mismas anclas {#id} en en/es/fr/de",
        "alinear páginas, weight o anclas entre idiomas",
    ),
    (
        "etiquetas de UI citadas",
        "bash scripts/check-docs-labels.sh",
        "Cada etiqueta que los docs mandan pulsar existe literal en src/locales/<lang>/",
        "citar la cadena real de src/locales/<lang>/ en lugar de parafrasearla",
    ),
    (
        "rutas de fichero citadas",
        "uv run --no-project scripts/check-docs-paths.py",
        "Cada ruta del repo entre backticks resuelve a un fichero que existe",
        "corregir la ruta, o añadirla a ALLOWED_UNRESOLVED si vive en otro repo",
    ),
    (
        "claims emparejados con sus tests",
        "uv run --no-project scripts/check-docs-claims.py",
        "Cada <!-- claim:id --> tiene un docClaim() que lo prueba, y al revés, en los 4 idiomas",
        "añadir el marcador o el caso que falta, o retirar ambos",
    ),
]

for name, cmd, desc, fix in STATIC:
    t0 = time.time()
    rc, out = sh(cmd)
    add(
        "static",
        name,
        "ok" if rc == 0 else "fail",
        "" if rc == 0 else out.splitlines()[-1][:200],
        int((time.time() - t0) * 1000),
        desc=f"{desc} · Comando: {cmd}",
        trace="" if rc == 0 else out[-4000:],
        proposed_fix="" if rc == 0 else fix,
    )

# ── contract: the docs quote the catalog ────────────────────────────────────
t0 = time.time()
rc, out = sh(
    "cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib "
    "-- --exact ai::model_catalog::recommendation_tests::"
    "published_model_table_matches_the_catalog_in_every_language "
    "ai::model_catalog::recommendation_tests::"
    "getting_started_anchors_the_recommendation_to_a_machine_size"
)
add(
    "contract",
    "tablas y cifras citadas vs model_catalog.rs",
    "ok" if rc == 0 else "fail",
    "" if rc == 0 else "los docs publicados contradicen el catálogo",
    int((time.time() - t0) * 1000),
    desc="ai-features.md y getting-started.md se comparan contra las constantes reales del catálogo, en los 4 idiomas",
    trace="" if rc == 0 else out[-6000:],
    proposed_fix="" if rc == 0 else "actualizar la tabla de modelos o la frase de recomendación en los 4 idiomas",
)

# ── doc: claims driven against the running app ──────────────────────────────
if WITH_APP:
    t0 = time.time()
    rc, out = sh("bash scripts/verify_all.sh --only e2e")
    current = ROOT / "src-tauri/reports/verify/current-full"
    sweep = current / "app/sweep/results.json"
    if not sweep.exists():
        add(
            "doc",
            "claims contra la app",
            "fail",
            "la barrida no dejó results.json",
            int((time.time() - t0) * 1000),
            trace=out[-4000:],
        )
    else:
        for r in json.loads(sweep.read_text()):
            if not r["step"].startswith("doc:"):
                continue
            shot = current / "app/sweep" / (r.get("shot") or "")
            add(
                "doc",
                f"{r['feature']} › {r['step']}",
                r["status"],
                r["detail"],
                desc=r.get("expect", ""),
                expect=r.get("expect", ""),
                proposed_fix=r.get("fix", ""),
                page=r.get("page", ""),
                shots=[str(shot)] if r.get("shot") and shot.exists() else [],
            )
else:
    add(
        "doc",
        "claims contra la app",
        "skip",
        "no ejecutados: relanza con --with-app",
        desc="Los docClaim() de sweep.mjs arrancan la instancia de verificación; se omiten en la pasada rápida para que ésta dure segundos",
    )

data = {
    "meta": {
        "commit": sh("git rev-parse --short HEAD")[1],
        "branch": sh("git rev-parse --abbrev-ref HEAD")[1],
        "tier": "with-app" if WITH_APP else "rápido",
    },
    "layers": [{"layer": "docs", "status": "ok"}],
    "types": ["static", "contract", "doc"],
    "features": [FEATURE],
    "records": records,
}
(RUN / "results.json").write_text(json.dumps(data, ensure_ascii=False, indent=1))

fails = [r for r in records if r["status"] == "fail"]
print(f"{len(records)} casos, {len(fails)} fallos")
for r in fails:
    print(f"  FALLO  {r['name']}: {r['detail']}")
sys.exit(1 if fails else 0)
