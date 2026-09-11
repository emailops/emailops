#!/usr/bin/env python3
"""Full verification of EmailOps, one run, every test layer, results per feature.

    verify_all.py [--tier quick|full] [--skip layer,...] [--only layer,...]

Layers, in order: git, static, rust, vitest, contract, e2e, oracle, evals, perf.
`quick` skips e2e, oracle and evals. Writes <run>/results.json (normalised test
records + layer log) and <run>/layers/*.raw; report_all.py renders it.
"""
import argparse, collections, json, os, re, shutil, subprocess, sys, time, datetime, pathlib

HERE = pathlib.Path(__file__).resolve().parent
SKILL = HERE.parent
REPO = SKILL.parents[2]
MANIFEST = json.loads((SKILL / "features.json").read_text())
NODE_BIN = pathlib.Path.home() / ".nvm/versions/node/v22.23.1/bin"
ENV = dict(os.environ, PATH=f"{NODE_BIN}:{os.environ['PATH']}")

ap = argparse.ArgumentParser()
ap.add_argument("--tier", default="full", choices=["quick", "full"])
ap.add_argument("--skip", default="")
ap.add_argument("--only", default="")
args = ap.parse_args()
skip = set(filter(None, args.skip.split(",")))
only = set(filter(None, args.only.split(",")))
if args.tier == "quick":
    skip |= {"e2e", "oracle", "evals"}

stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
RUN = REPO / "src-tauri/reports/verify" / f"{stamp}-full"
LAYERS = RUN / "layers"; LAYERS.mkdir(parents=True)
(REPO / "src-tauri/reports/verify/current-full").unlink(missing_ok=True)
(REPO / "src-tauri/reports/verify/current-full").symlink_to(RUN)

records = []   # normalised tests
layers = []    # layer log
meta = {"started": datetime.datetime.now().isoformat(timespec="seconds"), "tier": args.tier, "run_dir": str(RUN)}

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
        fn(); status = "ok"; err = ""
    except Exception as e:  # a broken layer is a result, not a crash
        status = "error"; err = f"{type(e).__name__}: {e}"; log(f"layer {name} ERROR {err}")
        add("Transversal", "static", f"capa {name}", "fail", err)
    layers.append({"layer": name, "status": status, "seconds": round(time.time() - t0, 1), "error": err})
    log(f"layer {name} done ({time.time() - t0:.0f}s)")

# ---------- descriptions ----------
RUST_TEST_RE = re.compile(r"((?:^[ \t]*//[^\n]*\n)*)[ \t]*#\[(?:tokio::)?test[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub )?(?:async )?fn ([A-Za-z0-9_]+)", re.M)
def humanize(name): return name.replace("_", " ").strip().capitalize()
def rust_doc_index():
    """test fn → [(module_of_file, description)] from the comment block right above #[test]."""
    idx = collections.defaultdict(list)
    for base in (REPO / "src-tauri/src", REPO / "src-tauri/tests"):
        for f in base.rglob("*.rs"):
            rel = f.relative_to(REPO / "src-tauri")
            mod = str(rel.with_suffix("")).replace("src/", "").replace("/mod", "").replace("/", "::").replace("lib", "")
            text = f.read_text(errors="replace")
            for m in RUST_TEST_RE.finditer(text):
                comment = " ".join(l.strip().lstrip("/").strip() for l in m.group(1).splitlines() if l.strip())
                idx[m.group(2)].append((mod, (comment or humanize(m.group(2)))[:300]))
    return idx
RUST_DOCS = rust_doc_index()
def rust_desc(path):
    fn = path.split("::")[-1]; cands = RUST_DOCS.get(fn) or []
    for mod, d in cands:
        if mod and path.startswith(mod): return d
    return cands[0][1] if cands else humanize(fn)
VITEST_HEADER = {}
def vitest_desc(rel, full_name):
    if rel not in VITEST_HEADER:
        text = (REPO / rel).read_text(errors="replace")
        m = re.match(r"\s*((?://[^\n]*\n)+)", text)
        VITEST_HEADER[rel] = " ".join(l.strip().lstrip("/").strip() for l in m.group(1).splitlines()) [:300] if m else ""
    head = VITEST_HEADER[rel]
    return f"{full_name}" + (f" — {head}" if head else "")

# ---------- attribution ----------
FEATURES = MANIFEST["features"]
def feature_for_rust(path):
    for f in FEATURES:
        for pref in f.get("rust", []):
            if path.startswith(pref + "::") or path == pref: return f["name"]
    return "Transversal"
def feature_for_integration(name):
    low = name.lower()
    for f in FEATURES:
        if any(k in low for k in f.get("integration_names", [])): return f["name"]
    return "Transversal"
def feature_for_vitest(file):
    rel = os.path.relpath(file, REPO) if os.path.isabs(file) else file
    best = None
    for f in FEATURES:
        for pref in f.get("vitest", []):
            if rel.startswith(pref) and (best is None or len(pref) > len(best[0])): best = (pref, f["name"])
    return best[1] if best else "Transversal"
def feature_for_e2e(feature, step):
    key = f"{feature}/{step}"
    for f in FEATURES:
        for pat in f.get("e2e", []):
            if pat == feature or key.startswith(pat): return f["name"]
    return "Transversal"
def feature_for_eval(case_id):
    for f in FEATURES:
        for pat in f.get("evals", []):
            if re.search(pat, case_id): return f["name"]
    return "Chat con el buzón"
def is_contract(kind, name):
    return any(re.search(p, name) for p in MANIFEST["contract"].get(kind, []))

# ---------- layers ----------
def layer_git():
    rc, top, _ = sh(["git", "rev-parse", "--show-toplevel"])
    rc, br, _ = sh(["git", "branch", "--show-current"])
    rc, c, _ = sh(["git", "log", "-1", "--format=%h %s (%ci)"])
    rc, st, _ = sh(["git", "status", "--short"])
    meta.update(worktree=top.strip(), branch=br.strip(), commit=c.strip(), dirty=[l for l in st.splitlines() if l.strip()])

def layer_static():
    checks = [
        ("tsc --noEmit", "npx tsc --noEmit", "fail"),
        ("biome (errores)", "node_modules/.bin/biome check src/ --diagnostic-level=error", "fail"),
        ("literales JSX fuera de i18n", "node scripts/check-jsx-literals.mjs", "fail"),
        ("claves i18n usadas vs catálogos", "npm run -s i18n:check", "fail"),
        ("cargo fmt --check", "cargo fmt --manifest-path src-tauri/Cargo.toml -- --check", "fail"),
        ("clippy (flags de CI)", "cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --tests -- -D warnings", "fail"),
        ("cargo audit", "cargo audit --file src-tauri/Cargo.lock", "info"),
        ("npm audit (high/critical)", "npm audit --audit-level=high", "info"),
    ]
    for name, cmd, sev in checks:
        t0 = time.time(); rc, out, err = sh(cmd)
        text = (out + err).strip(); tail = "\n".join(text.splitlines()[-40:])
        status = "ok" if rc == 0 else ("info" if sev == "info" else "fail")
        (LAYERS / f"static-{re.sub('[^a-z0-9]+', '-', name.lower())}.txt").write_text(text)
        add("Transversal", "static", name, status, "" if rc == 0 else tail, int((time.time() - t0) * 1000), desc=f"Comando: {cmd}", trace=tail if rc else "")

RUST_TEST = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)")
def layer_rust():
    # Merge the streams in order: the "Running unittests/tests/…" headers go to stderr
    # and are what tells a unit test from an integration test.
    rc, out, err = sh("cargo test --manifest-path src-tauri/Cargo.toml 2>&1", timeout=3600)
    text = out; (LAYERS / "rust.raw").write_text(text)
    # failure traces: "---- name stdout ----" blocks
    traces = {}
    for m in re.finditer(r"^---- (\S+) stdout ----\n(.*?)(?=^---- \S+ stdout ----|^failures:|^test result|\Z)", text, re.S | re.M):
        traces[m.group(1)] = m.group(2).strip()[:6000]
    binary = "unit"
    for line in text.splitlines():
        if line.strip().startswith("Running unittests"): binary = "unit"
        elif line.strip().startswith("Running tests/"): binary = "integration"
        elif line.strip().startswith("Doc-tests"): binary = "doc"
        m = RUST_TEST.match(line.strip())
        if not m: continue
        name, res = m.group(1), m.group(2)
        status = {"ok": "ok", "FAILED": "fail", "ignored": "skip"}[res]
        if binary == "integration":
            typ, feature = "integration", feature_for_integration(name)
        elif binary == "doc":
            typ, feature = "unit", "Transversal"
        else:
            typ, feature = ("contract" if is_contract("rust", name) else "unit"), feature_for_rust(name)
        add(feature, typ, name, status, "" if status != "fail" else "assertion failed (ver traza)", desc=rust_desc(name), trace=traces.get(name, ""))
    if rc != 0 and not any(r["status"] == "fail" and r["type"] in ("unit", "integration", "contract") for r in records):
        add("Transversal", "static", "cargo test (compilación)", "fail", "\n".join(text.splitlines()[-40:]))

def layer_vitest():
    raw = LAYERS / "vitest.raw.json"
    rc, out, err = sh(f"npx vitest run --reporter=json --outputFile={raw}", timeout=1800)
    data = json.loads(raw.read_text())
    for f in data.get("testResults", []):
        rel = os.path.relpath(f["name"], REPO)
        for a in f.get("assertionResults", []):
            status = {"passed": "ok", "failed": "fail", "skipped": "skip", "pending": "skip", "todo": "skip"}.get(a["status"], a["status"])
            typ = "contract" if is_contract("vitest", rel) else "unit"
            add(feature_for_vitest(rel), typ, f"{rel} › {a['fullName']}", status, "" if status != "fail" else "assertion failed (ver traza)", a.get("duration"), desc=vitest_desc(rel, a["fullName"]), trace="\n".join(a.get("failureMessages", []))[:6000])
    if rc != 0 and not any(r["status"] == "fail" and r["name"].startswith("src/") for r in records):
        add("Transversal", "static", "vitest (arranque)", "fail", (out + err)[-3000:])

def layer_contract():
    # The CLI's --json envelope is the contract agents script against.
    # cli-demo: the same data dir the sweep and the evals use, so the AI config in the
    # report is the one that was actually exercised (cli-fast would read the real install).
    rc, out, err = sh('make cli-demo ARGS="doctor --json"', timeout=1800)
    body = out[out.find("{"):] if "{" in out else ""
    try:
        d = json.loads(body); okshape = set(d) == {"ok", "data", "error"} and isinstance(d["ok"], bool)
        if okshape and d.get("data"): meta["ai"] = {k: d["data"].get(k) for k in ("provider", "model", "embeddingModel", "aiEnabled")}
        add("Transversal", "contract", "emailops-cli doctor --json: envelope {ok,data,error}", "ok" if okshape else "fail", "" if okshape else f"claves: {sorted(d)}", desc="La CLI devuelve siempre el mismo sobre JSON {ok, data, error} para que un agente pueda parsear éxito y fallo con una sola forma", trace=body[:2000] if not okshape else "")
    except Exception as e:
        add("Transversal", "contract", "emailops-cli doctor --json: envelope {ok,data,error}", "fail", f"sin JSON: {e}", trace=(out + err)[-2000:])

APP = RUN / "app"
V = SKILL / "scripts/verify.sh"
def app_env(): return dict(ENV, VERIFY_RUN_DIR=str(APP))
def layer_e2e():
    APP.mkdir(exist_ok=True)
    t0 = time.time(); rc, out, err = sh([str(V), "launch"], timeout=1200, env=app_env())
    launch_s = round(time.time() - t0, 1); meta["launch_s"] = launch_s
    (LAYERS / "launch.txt").write_text(out + err)
    if rc != 0:
        add("Transversal", "e2e", "arranque de la instancia de verificación", "fail", (out + err)[-2000:]); return
    rc, out, err = sh(["node", str(SKILL / "scripts/sweep.mjs"), str(APP)], timeout=1800)
    (LAYERS / "sweep.txt").write_text(out + err)
    res = APP / "sweep/results.json"
    if not res.exists():
        add("Transversal", "e2e", "barrida UI (sweep.mjs)", "fail", (out + err)[-2000:]); return
    for r in json.loads(res.read_text()):
        typ = "ui" if re.search(r"barra de herramientas|anchura|cabecera|panel|Escape", r["step"]) else "e2e"
        shots = [str(APP / "sweep" / r["shot"])] if r.get("shot") else []
        add(feature_for_e2e(r["feature"], r["step"]), typ, f"{r['feature']} › {r['step']}", r["status"], r["detail"], None, desc=r.get("expect", ""), expect=r.get("expect", ""), shots=shots, log_tail=log_tail() if r["status"] == "fail" else "")
def log_tail():
    p = APP / "app.log"
    return "\n".join(p.read_text(errors="replace").splitlines()[-25:]) if p.exists() else ""

def layer_oracle():
    rc, out, err = sh(["node", str(SKILL / "scripts/tagboard_check.mjs"), str(APP)], timeout=1800)
    (LAYERS / "tagboard.txt").write_text(out + err)
    res = APP / "tagboard/results.json"
    if not res.exists():
        add("Tag Board y clasificación", "oracle", "oráculo del Tag Board (tagboard_check.mjs)", "fail", (out + err)[-2000:]); return
    for r in json.loads(res.read_text()):
        add("Tag Board y clasificación", "oracle", f"{r['kind']} › {r['step']}", r["status"], r["detail"], None, desc=r.get("expect", ""), expect=r.get("expect", ""), shots=[str(APP / "tagboard" / r["shot"])] if r.get("shot") else [])

def teardown():
    if (APP / "app.pid").exists():
        sh([str(V), "cleanup"], env=app_env())

def layer_evals():
    teardown()  # one llama.cpp at a time on the GPU
    t0 = time.time(); rc, out, err = sh('make cli-eval ARGS="--json"', timeout=3600)
    body = out[out.find("{"):] if "{" in out else ""
    (LAYERS / "evals.raw.json").write_text(body or out + err)
    try: d = json.loads(body)
    except Exception:
        add("Chat con el buzón", "eval", "eval de chat (CLI)", "fail", (out + err)[-3000:]); return
    if not d.get("ok"):
        e = d.get("error") or {}
        add("Chat con el buzón", "eval", "eval de chat (CLI)", "fail", f"{e.get('code')}: {e.get('message')}", trace=json.dumps(e, ensure_ascii=False)[:4000]); return
    for c in d["data"]["cases"]:
        failing = [ck for ck in c["checks"] if not ck["passed"]]
        detail = "; ".join(f"{ck['name']}: esperado {ck['expected']!r}, obtenido {ck['actual']!r}" for ck in failing)[:1500]
        model = (c.get("trace") or {}).get("model") or (meta.get("ai") or {}).get("model") or ""
        add(feature_for_eval(c["id"]), "eval", f"{c['id']} ({c['tier']})", "ok" if c["passed"] else "fail",
            detail or f"{c['checksPassed']}/{c['checksTotal']} checks", c.get("latencyMs"),
            desc=f"Pregunta: {c.get('question', '')} · Checks: {', '.join(ck['name'] for ck in c['checks'])}",
            question=c.get("question", ""), answer=c.get("answer", ""), ai_trace=c.get("trace"), checks=c["checks"], model=model, judge="ninguno: métricas heurísticas (anclas de texto, ruta, herramientas llamadas)")

def layer_perf():
    b = MANIFEST["budgets"]
    if "launch_s" in meta:
        add("Transversal", "perf", f"arranque de la instancia de verificación ≤ {b['launch_s']} s", "ok" if meta["launch_s"] <= b["launch_s"] else "fail", f"{meta['launch_s']} s", desc="Tiempo desde `verify.sh launch` hasta que el WebDriver embebido responde; presupuesto en features.json")
    for r in records:
        if r["type"] == "e2e" and "pregunta y respuesta" in r["name"]:
            m = re.search(r"(\d+) s;", r["detail"])
            if m: add("Chat con el buzón", "perf", f"respuesta del chat ≤ {b['chat_answer_s']} s", "ok" if int(m.group(1)) <= b["chat_answer_s"] else "fail", f"{m.group(1)} s")
    ev = [r for r in records if r["type"] == "eval" and r.get("duration_ms")]
    if ev:
        worst = max(ev, key=lambda r: r["duration_ms"])
        add("Chat con el buzón", "perf", f"caso de eval más lento ≤ {b['eval_case_s']} s", "ok" if worst["duration_ms"] / 1000 <= b["eval_case_s"] else "fail", f"{worst['name']}: {worst['duration_ms'] / 1000:.1f} s")

try:
    for name, fn in [("git", layer_git), ("static", layer_static), ("rust", layer_rust), ("vitest", layer_vitest), ("contract", layer_contract), ("e2e", layer_e2e), ("oracle", layer_oracle), ("evals", layer_evals), ("perf", layer_perf)]:
        layer_run(name, fn)
finally:
    teardown()
    sh(["git", "checkout", "--", "src-tauri/gen/schemas"])  # the webdriver dev build rewrites these
    meta["finished"] = datetime.datetime.now().isoformat(timespec="seconds")
    (RUN / "results.json").write_text(json.dumps({"meta": meta, "layers": layers, "records": records, "features": [f["name"] for f in FEATURES] + ["Transversal"], "types": MANIFEST["types"]}, ensure_ascii=False, indent=1))
    counts = {s: sum(r["status"] == s for r in records) for s in ("ok", "fail", "skip", "info")}
    log(f"done: {counts} → {RUN}/results.json")
