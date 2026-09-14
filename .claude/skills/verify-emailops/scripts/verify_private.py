#!/usr/bin/env python3
"""Private verification run: the eval suites that need the developer's real
mailbox, run against a snapshot of the production DB, one report per run.

Usage:
    verify_private.py [--tier smoke|full|lab|all] [--skip layer,...] [--only layer,...]

Layers: git, chat (private chat cases through `emailops-cli eval --judge`,
same harness and judge as the public report), junk (the private golden set,
deterministic). Reads VERIFY_EVAL_MODEL / VERIFY_JUDGE_MODEL like verify_all.py
and EVAL_SNAPSHOT_DIR for the snapshot (default $TMPDIR/eval-snapshot, made by
`make eval-snapshot`). Never touches the production data dir: the snapshot dir
carries a `models` symlink so the CLI finds the GGUF files there.

Writes <run>/results.json in the same shape as verify_all.py so report_all.py
renders it. The report contains real senders, subjects and answers: it stays
under the gitignored reports tree and must not be published.
"""
import argparse, datetime, json, os, pathlib, re, sqlite3, subprocess, sys, time

REPO = pathlib.Path(__file__).resolve().parents[4]
SKILL = REPO / ".claude/skills/verify-emailops"
NODE_BIN = pathlib.Path.home() / ".nvm/versions/node/v22.23.1/bin"
ENV = dict(os.environ, PATH=f"{NODE_BIN}:{os.environ['PATH']}")
MANIFEST = json.loads((SKILL / "features.json").read_text())
FEATURES = MANIFEST["features"]

ap = argparse.ArgumentParser()
ap.add_argument("--tier", default="all", choices=["smoke", "full", "lab", "all"])
ap.add_argument("--skip", default="")
ap.add_argument("--only", default="")
args = ap.parse_args()
skip = {s for s in args.skip.split(",") if s}
only = {s for s in args.only.split(",") if s}

SNAP = pathlib.Path(os.environ.get("EVAL_SNAPSHOT_DIR") or (pathlib.Path(os.environ.get("TMPDIR", "/tmp")) / "eval-snapshot"))
stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
RUN = REPO / "src-tauri/reports/verify-private" / f"{stamp}-private"
LAYERS = RUN / "layers"; LAYERS.mkdir(parents=True)
(REPO / "src-tauri/reports/verify-private/current-private").unlink(missing_ok=True)
(REPO / "src-tauri/reports/verify-private/current-private").symlink_to(RUN)

records, layers = [], []
meta = {"started": datetime.datetime.now().isoformat(timespec="seconds"), "tier": args.tier, "run_dir": str(RUN), "private": True}

def log(msg): print(f"[{datetime.datetime.now():%H:%M:%S}] {msg}", flush=True)
def sh(cmd, cwd=REPO, timeout=3600, env=None):
    p = subprocess.run(cmd, cwd=cwd, shell=isinstance(cmd, str), capture_output=True, text=True, timeout=timeout, env=env or ENV)
    return p.returncode, p.stdout, p.stderr
def add(feature, typ, name, status, detail="", duration_ms=None, desc="", **evidence):
    records.append({"feature": feature, "type": typ, "name": name, "status": status, "detail": detail[:2000], "duration_ms": duration_ms, "desc": desc[:400], "evidence": evidence})
def enabled(layer): return layer not in skip and (not only or layer in only)
def layer_run(name, fn):
    if not enabled(name):
        layers.append({"layer": name, "status": "skipped"}); log(f"skip {name}"); return
    t0 = time.time(); log(f"layer {name} …")
    try:
        fn(); layers.append({"layer": name, "status": "ok", "seconds": round(time.time() - t0, 1)})
    except Exception as e:  # a broken layer must not lose the others
        layers.append({"layer": name, "status": "error", "seconds": round(time.time() - t0, 1), "error": repr(e)[:2000]})
        add("Transversal", "eval", f"capa {name}", "fail", repr(e)[:2000])
    log(f"layer {name} done ({round(time.time() - t0)}s)")
def feature_for_eval(case_id):
    for f in FEATURES:
        for pat in f.get("evals", []):
            if re.search(pat, case_id): return f["name"]
    return "Chat con el buzón"

def snapshot_facts():
    db = SNAP / "emailops.db"
    if not db.exists(): raise RuntimeError(f"no snapshot at {db}; run `make eval-snapshot` first")
    if not (SNAP / "models").exists(): raise RuntimeError(f"{SNAP}/models missing: symlink it to the real models dir")
    c = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    q = lambda sql: c.execute(sql).fetchone()[0]
    facts = {"path": str(db), "copied_at": datetime.datetime.fromtimestamp(db.stat().st_mtime).isoformat(timespec="seconds"),
             "size_gb": round(db.stat().st_size / 1e9, 2), "accounts": q("select count(*) from accounts where enabled=1"),
             "emails": q("select count(*) from emails where is_deleted=0"), "threads": q("select count(distinct thread_id) from emails where is_deleted=0")}
    c.close(); return facts

def layer_git():
    meta["db"] = {"snapshot": snapshot_facts(),
                  "by_type": {"eval": "snapshot de la BD de producción en $TMPDIR/eval-snapshot (conversaciones desechables sobre la copia; la BD real no se toca) · junk: sin BD"}}
    rc, top, _ = sh(["git", "rev-parse", "--show-toplevel"])
    rc, br, _ = sh(["git", "branch", "--show-current"])
    rc, c, _ = sh(["git", "log", "-1", "--format=%h %s (%ci)"])
    rc, st, _ = sh(["git", "status", "--short"])
    meta.update(worktree=top.strip(), branch=br.strip(), commit=c.strip(), dirty=[l for l in st.splitlines() if l.strip()])

def layer_chat():
    model = os.environ.get("VERIFY_EVAL_MODEL", "")
    judge_model = os.environ.get("VERIFY_JUDGE_MODEL", model)
    flags = "--json --judge" + (f" --model {model}" if model else "") + (f" --judge-model {judge_model}" if judge_model else "") + (f" --tier {args.tier}" if args.tier != "all" else "")
    meta["evals"] = {"flags": flags, "model": model or "(preferencia ai_model de la BD)", "judge_model": judge_model or model or "(preferencia ai_model)", "cases_dir": "private-evals/chat/cases"}
    env = dict(ENV, EMAILOPS_DATA_DIR=str(SNAP))
    rc, out, err = sh(f'cargo run --manifest-path src-tauri/Cargo.toml --features cli,eval --bin emailops-cli -- eval --cases-dir private-evals/chat/cases {flags}', timeout=7200, env=env)
    body = out[out.find("{"):] if "{" in out else ""
    (LAYERS / "chat.raw.json").write_text(body or out + err)
    try: d = json.loads(body)
    except Exception:
        add("Chat con el buzón", "eval", "eval de chat privado (CLI)", "fail", (out + err)[-3000:]); return
    if not d.get("ok"):
        e = d.get("error") or {}
        add("Chat con el buzón", "eval", "eval de chat privado (CLI)", "fail", f"{e.get('code')}: {e.get('message')}", trace=json.dumps(e, ensure_ascii=False)[:4000]); return
    for c in d["data"]["cases"]:
        failing = [ck for ck in c["checks"] if not ck["passed"]]
        detail = "; ".join(f"{ck['name']}: esperado {ck['expected']!r}, obtenido {ck['actual']!r}" for ck in failing)[:1500]
        case_model = (c.get("trace") or {}).get("model") or model
        j = c.get("judge")
        if j and not j.get("passed"):
            sc = j.get("scores") or {}
            detail = (detail + "; " if detail else "") + "juez: " + (sc.get("error") or ", ".join(f"{k} {v:.2f}" for k, v in sc.items() if isinstance(v, (int, float))) + f" < {j.get('threshold')}")
        judge_desc = (f"juez {j['model']} (umbral {j['threshold']}) sobre {', '.join(j['metrics']) or 'sin métricas'}" if j else "sin juez: solo métricas heurísticas")
        add(feature_for_eval(c["id"]), "eval", f"{c['id']} ({c['tier']})", "ok" if c["passed"] else "fail",
            detail or f"{c['checksPassed']}/{c['checksTotal']} checks" + (" · juez ok" if j else ""), c.get("latencyMs"),
            desc=f"Caso privado · Pregunta: {c.get('question', '')} · Checks: {', '.join(ck['name'] for ck in c['checks'])} · {judge_desc}",
            question=c.get("question", ""), answer=c.get("answer", ""), expected_output=c.get("expectedOutput"), ai_trace=c.get("trace"), checks=c["checks"], model=case_model, judge=judge_desc, judge_report=j)

def layer_junk():
    out = LAYERS / "junk"; out.mkdir(exist_ok=True)
    rc, o, e = sh(f'make eval-junk ARGS="--cases-dir private-evals/junk/cases --out {out}"', timeout=1800)
    files = sorted(f for f in out.glob("*.json") if not f.name.endswith("_metrics.json"))
    if not files:
        add("Correo no deseado", "eval", "junk_eval privado (harness)", "fail", (o + e)[-3000:]); return
    rep = json.loads(files[-1].read_text())
    harness = "junk_eval sobre el golden set privado (private-evals/junk/cases): veredictos etiquetados a mano sobre correo real, sin modelo ni BD; las puertas globales limitan los falsos positivos sobre correo legítimo"
    for it in rep["per_item_results"]:
        add("Correo no deseado", "eval", it["id"], "ok" if it["passed"] else "fail", it.get("detail") or "", None,
            desc=f"Caso privado de junk_eval; puntuación {it.get('score')}", model="determinista (Naive Bayes local)", judge="sin juez: veredicto etiquetado", harness=harness, checks=[])
    metrics = sorted(out.glob("*_metrics.json"))
    if metrics:
        for g in json.loads(metrics[-1].read_text()).get("gates", []):
            add("Correo no deseado", "eval", f"puerta {g['name']}", "ok" if g["passed"] else "fail", f"actual {g['actual']:.4f} · límite {g['limit']:.4f}", None,
                desc="Puerta global del detector sobre el golden set privado", model="determinista (Naive Bayes local)", judge="sin juez: umbral fijo", harness=harness, checks=[])

try:
    for name, fn in [("git", layer_git), ("chat", layer_chat), ("junk", layer_junk)]:
        layer_run(name, fn)
finally:
    meta["finished"] = datetime.datetime.now().isoformat(timespec="seconds")
    (RUN / "results.json").write_text(json.dumps({"meta": meta, "layers": layers, "records": records, "features": [f["name"] for f in FEATURES] + ["Transversal"], "types": MANIFEST["types"]}, ensure_ascii=False, indent=1))
    counts = {s: sum(r["status"] == s for r in records) for s in ("ok", "fail", "skip", "info")}
    log(f"done: {counts} → {RUN}/results.json")
