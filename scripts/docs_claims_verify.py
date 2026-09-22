"""Evaluate the published docs, fragment by fragment.

A block (paragraph, item, table, code) splits into fragments — sentences,
table rows, code blocks — and every validation says which fragments it covers:
an app case by quoting the text it verifies (`covers`), a catalog check by the
text it quotes or by an explicit `covers`, anything else by the whole block.
A fragment's colour is then:

    red     a validation that covers it fails (or the agent contradicts it)
    green   a deterministic check of behaviour passes (or the agent supports it)
    yellow  everything else: only a label was seen, the expectation was typed
            into the check instead of read from the docs, the case does not
            say what it covers, manual, no claim, not run, or nothing covers it

Green is earned per sentence: a check that proves one sentence of a paragraph
leaves the others yellow, where the reader can see them.
"""

import fnmatch
import json
import pathlib
import re
import subprocess

from docs_claims_lib import ROOT, blocks, fragments, front_matter, load_catalog, locate, pages

KINDS = ("file", "tests", "app", "release", "generated", "judge", "manual", "none")
TAG_OF = {"file": "COD", "tests": "TST", "release": "REL", "generated": "GEN", "judge": "AGT",
          "manual": "MAN", "none": "MAN"}
GREEN = {"ok", "supported"}
RED = {"fail", "contradicted"}
DEFAULT_FIX = ("Corregir la frase en los 4 idiomas para que diga lo que hace la app, o actualizar "
               "docs/site/claims.toml si la app cambió a propósito.")


def kind(check):
    for k in KINDS:
        if k in check:
            return k
    raise ValueError(f"unknown check {check!r}")


def squash(s):
    return " ".join(s.split())


def files_for(pattern):
    if any(c in pattern for c in "*?["):
        tracked = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files"], capture_output=True, text=True, check=True
        ).stdout.splitlines()
        return [ROOT / p for p in tracked if fnmatch.fnmatch(p, pattern)]
    p = ROOT / pattern
    return [p] if p.exists() else []


def check_file(check, claim_text):
    """→ (passed, explanation)"""
    paths = files_for(check["file"])
    if not paths:
        return False, f"{check['file']} no existe"
    body = "\n".join(p.read_text(encoding="utf-8", errors="replace") for p in paths)
    where = check["file"]

    if "quoted" in check:
        q = check["quoted"]
        if squash(q) not in squash(claim_text):
            return False, f"la afirmación ya no cita «{q}» — actualizar el catálogo o la página"
        if q.lower() not in body.lower():
            return False, f"la doc cita «{q}» pero {where} no lo contiene"
        return True, f"«{q}» está en la doc y en {where}"
    if "has" in check:
        h = check["has"]
        if h not in body:
            return False, f"{where} ya no contiene «{h}»"
        return True, f"{where} contiene «{h[:60]}»"
    if "lacks" in check:
        h = check["lacks"]
        if h in body:
            return False, f"{where} contiene «{h}», lo que contradice la afirmación"
        return True, f"{where} no contiene «{h}»"
    if "regex" in check:
        if not re.search(check["regex"], body, re.M):
            return False, f"{where} no casa /{check['regex']}/"
        return True, f"{where} casa /{check['regex']}/"
    raise ValueError(f"file check without an assertion: {check!r}")


_RELEASE = None


def release_assets():
    """Asset names of the latest published release: the ground truth for
    "download X from the latest release". Fetched once per run."""
    global _RELEASE
    if _RELEASE is None:
        import urllib.request
        req = urllib.request.Request(
            "https://api.github.com/repos/emailops/emailops/releases/latest",
            headers={"Accept": "application/vnd.github+json", "User-Agent": "emailops-docs-check"},
        )
        try:
            with urllib.request.urlopen(req, timeout=20) as r:
                data = json.load(r)
            _RELEASE = (data.get("tag_name", "?"), {a["name"] for a in data.get("assets", [])})
        except Exception as e:  # offline: the check fails with the reason, never passes
            _RELEASE = (f"sin acceso a GitHub ({type(e).__name__})", None)
    return _RELEASE


def check_release(check, claim_text):
    name = check["release"]
    if squash(name) not in squash(claim_text):
        return False, f"la afirmación ya no cita «{name}» — actualizar el catálogo o la página"
    tag, assets = release_assets()
    if assets is None:
        return False, f"no se pudo consultar la release: {tag}"
    if name not in assets:
        return False, f"la release {tag} no publica «{name}» (publica: {', '.join(sorted(assets))})"
    return True, f"la release {tag} publica «{name}»"


def run_rust_tests(names):
    """Run the named tests once, exactly. → {name: 'ok' | 'fail' | 'missing'}"""
    if not names:
        return {}, ""
    out = subprocess.run(
        ["cargo", "test", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features",
         "--lib", "--", "--exact", *sorted(names)],
        cwd=ROOT, capture_output=True, text=True,
    )
    text = out.stdout + out.stderr
    got = {}
    for m in re.finditer(r"^test (\S+) \.\.\. (ok|FAILED)", text, re.M):
        got[m.group(1)] = "ok" if m.group(2) == "ok" else "fail"
    return {n: got.get(n, "missing") for n in names}, text


def claim_index():
    """{claim id: (page, block)} for the English pages, the verification oracle."""
    idx = {}
    for p in pages("en"):
        for b in blocks(p):
            if b.claim:
                idx[b.claim] = (p, b)
    return idx


def page_title(page):
    """The Spanish page title, so the report reads in the reader's language."""
    es = ROOT / "docs/site/es" / page.name
    return front_matter(es).get("title") or page.name


# ── validations → fragments ─────────────────────────────────────────────────

def color(validations):
    states = {v["state"] for v in validations}
    if states & RED:
        return "red"
    if states & GREEN:
        return "green"
    return "yellow"


def validation(tag, state, how, detail="", where="", fix="", **evidence):
    return {"tag": tag, "state": state, "how": how, "detail": detail, "where": where,
            "fix": fix if state not in GREEN else "", "evidence": {k: v for k, v in evidence.items() if v}}


def spread(block, covers):
    """→ (fragment indices, phrases not found). No covers = the whole block."""
    everything = [n for n, f in enumerate(fragments(block)) if f.kind != "header"]
    if not covers:
        return everything, []
    idx, missing = set(), []
    for c in covers:
        hit = locate(block, c)
        if hit:
            idx.update(hit)
        else:
            missing.append(c)
    return sorted(idx), missing


def app_validations(block, parts, ran):
    """The app cases for one block → [(indices, validation)]."""
    everything, _ = spread(block, None)
    if not ran:
        return [(everything, validation(
            "APP", "pending", "Se comprueba manejando la app; esta ejecución no la lanzó.",
            fix="Ejecutar make docs-check ARGS=--with-app."))]
    if not parts:
        return [(everything, validation(
            "APP", "fail", "Debería comprobarse en la app, pero ninguna fase lo hizo.",
            detail=f"ninguna fase de la app comprobó claim:{block.claim}",
            fix=f"Añadir el caso claim('{block.claim}', …) a doc_claims.mjs o docs_cli_claims.py."))]
    out = []
    for p in parts:
        tag, where = p.get("tag", "APP"), p.get("where", "")
        how = p.get("how") or f"Caso «{p.get('name', '')}» (sin descripción de cómo valida)."
        ev = {"observed": p.get("detail", ""), "screen": p.get("screen", ""), "shots": p.get("shots", [])}
        covers = p.get("covers")
        idx, missing = spread(block, covers)
        if missing:
            out.append((everything, validation(
                tag, "fail", how, where=where,
                detail="caso desactualizado: la doc ya no dice " + ", ".join(f"«{m}»" for m in missing),
                fix=f"Actualizar la comprobación y su `covers` en {where} a lo que dice ahora la doc.", **ev)))
            continue
        status = p.get("status", "fail")
        if status == "fail":
            state, fix = "fail", p.get("fix") or DEFAULT_FIX
        elif status == "skip":
            state, fix = "skip", "No se puede observar en esta máquina; revisarlo a mano o en otra máquina."
        elif not p.get("read_doc", True):
            state, fix = "fixed", f"Leer el valor esperado del texto de la doc en {where}, no fijarlo en el caso."
        elif p.get("proof") == "label":
            state, fix = "label", (f"El caso solo ve que el texto aparece en la app; probar el comportamiento "
                                   f"que describe la frase ({where}).")
        elif not covers:
            state, fix = "undeclared", f"Declarar en {where} qué frases cubre el caso (`covers`)."
        else:
            state, fix = "ok", ""
        out.append((idx, validation(tag, state, how, detail=p.get("detail", ""), where=where, fix=fix, **ev)))
    return out


def catalog_validations(block, entry, file_check, test_status, release_check=None):
    """The non-app checks of one catalog entry → [(indices, validation)]."""
    release_check = release_check or check_release
    everything, _ = spread(block, None)
    out = []
    for ch in entry.get("checks", []):
        k = kind(ch)
        if k in ("app", "judge"):
            continue  # from the app phases / the agent's judgments
        covers = ch.get("covers")
        if covers is None and k == "file" and "quoted" in ch:
            covers = [ch["quoted"]]
        if covers is None and k == "release":
            covers = [ch["release"]]
        idx, missing = spread(block, covers)
        if k == "generated" and covers is None:
            # What is generated is the table, not the sentence introducing it.
            idx = [n for n, f in enumerate(fragments(block)) if f.kind == "row"]
        tag = TAG_OF[k]
        if missing:
            out.append((everything, validation(
                tag, "fail", "Comprobación del catálogo que cita un texto que la doc ya no tiene.",
                detail="la doc ya no dice " + ", ".join(f"«{m}»" for m in missing),
                where=f"docs/site/claims.toml [{block.claim}]",
                fix="Actualizar la entrada del catálogo a lo que dice ahora la doc.")))
            continue
        where = f"docs/site/claims.toml [{block.claim}]"
        # A passing check that does not say which sentences it proves cannot
        # vouch for all of them (a generated table is the exception: the whole
        # block is the generated output).
        declared = covers is not None or k == "generated"

        def settle(state, fix):
            if state == "ok" and not declared:
                return "undeclared", f"Declarar en {where} qué frases prueba la comprobación (`covers`)."
            return state, fix

        if k == "file":
            ok, why = file_check(ch, block.text)
            f = ch["file"]
            how = ch.get("how") or (
                f"Se comprueba que la doc cita «{ch['quoted']}» y que {f} también lo contiene." if "quoted" in ch
                else f"Se comprueba que {f} contiene «{ch['has']}»." if "has" in ch
                else f"Se comprueba que {f} no contiene «{ch['lacks']}»." if "lacks" in ch
                else f"Se comprueba que {f} casa con /{ch.get('regex')}/.")
            out.append((idx, validation(tag, *_swap(settle("ok" if ok else "fail", DEFAULT_FIX), how, why, where))))
        elif k == "tests":
            for t in ch["tests"]:
                st = test_status.get(t, "missing")
                why = {"ok": f"el test {t} pasa", "fail": f"el test {t} FALLA",
                       "missing": f"el test {t} no existe (¿renombrado?)"}[st]
                how = ch.get("how") or f"Se ejecuta el test `{t}` del código, que prueba lo que dice la frase."
                fix = DEFAULT_FIX if st == "fail" else "Actualizar el nombre del test en claims.toml."
                out.append((idx, validation(tag, *_swap(settle("ok" if st == "ok" else "fail", fix), how, why, where))))
        elif k == "release":
            ok, why = release_check(ch, block.text)
            how = ch.get("how") or f"Se consulta la última release publicada en GitHub y se busca «{ch['release']}» entre sus ficheros."
            out.append((idx, validation(tag, *_swap(settle("ok" if ok else "fail", DEFAULT_FIX), how, why, where))))
        elif k == "generated":
            st = test_status.get(ch["generated"], "missing")
            how = ch.get("how") or (f"La tabla se genera desde el código; el test `{ch['generated']}` falla si "
                                    f"el markdown publicado no coincide con lo generado.")
            out.append((idx, validation(tag, "ok" if st == "ok" else "fail", how,
                                        "coincide con lo generado" if st == "ok" else "no coincide con lo generado",
                                        where, "Ejecutar make docs-gen y revisar el diff en los 4 idiomas.")))
        elif k == "manual":
            out.append((idx, validation(tag, "manual", f"Revisión manual: {ch['manual']}", where=where)))
        elif k == "none":
            out.append((idx, validation(tag, "none", f"No afirma nada verificable: {ch['none']}", where=where)))
    return out


def _swap(settled, how, detail, where):
    """(state, fix) + how/detail/where → validation()'s positional order."""
    state, fix = settled
    return state, how, detail, where, fix


def judge_validations(block, judgments):
    """The agent's verdicts on this block's sentences → [(indices, validation)]."""
    out = []
    for j in judgments:
        idx, missing = spread(block, [j["sentence"]])
        if missing:
            continue  # judged an older text; the next judging pass redoes it
        out.append((idx, validation(
            "AGT", j["verdict"], f"Juicio del agente ({j.get('model', '?')}) sobre la evidencia recogida en la app.",
            detail=j.get("reason", ""), where="judge/judgments.json",
            fix=j.get("fix") or ("Revisar la frase: el agente la contradice." if j["verdict"] == "contradicted"
                                 else "La evidencia no basta; añadir un caso determinista o marcarla como manual."),
            observed=j.get("evidence", ""))))
    return out


def fragment_model(block, validations):
    frs = fragments(block)
    per = [[] for _ in frs]
    for idx, v in validations:
        for n in idx:
            per[n].append(v)
    out = []
    for f, vs in zip(frs, per):
        if f.kind == "header":
            out.append({"kind": f.kind, "text": f.text, "prefix": f.prefix, "validations": [], "color": "none", "tags": []})
            continue
        tags = list(dict.fromkeys(v["tag"] for v in vs)) or ["SIN"]
        out.append({"kind": f.kind, "text": f.text, "prefix": f.prefix, "validations": vs,
                    "color": color(vs), "tags": tags})
    return out


def evaluate(app_parts, ran, judgments=()):
    """app_parts: {claim id: [part, …]} from the app phases; ran: whether the
    app was driven at all. → (pages, test log)."""
    catalog = load_catalog()
    tests_wanted = {t for c in catalog.values() for ch in c.get("checks", []) if "tests" in ch for t in ch["tests"]}
    tests_wanted |= {ch["generated"] for c in catalog.values() for ch in c.get("checks", []) if "generated" in ch}
    test_status, test_log = run_rust_tests(tests_wanted) if tests_wanted else ({}, "")
    by_claim = {}
    for j in judgments:
        by_claim.setdefault(j["claim"], []).append(j)

    out = []
    for page in pages("en"):
        items = []
        for b in blocks(page, with_headings=True):
            if b.kind == "heading":
                items.append({"kind": "heading", "level": b.level, "text": b.text, "line": b.start})
                continue
            entry = catalog.get(b.claim) if b.claim else None
            if entry is None:
                v = [(spread(b, None)[0], validation("MAN", "fail", "Bloque sin catalogar.",
                                                     detail=f"sin entrada en claims.toml para {b.claim}",
                                                     fix="Marcar el bloque y catalogarlo en docs/site/claims.toml."))]
            else:
                v = catalog_validations(b, entry, check_file, test_status)
                if any("app" in ch for ch in entry.get("checks", [])) or app_parts.get(b.claim):
                    v += app_validations(b, app_parts.get(b.claim, []), ran)
                v += judge_validations(b, by_claim.get(b.claim, []))
            items.append({"kind": b.kind, "claim": b.claim, "line": b.start, "fragments": fragment_model(b, v)})
        out.append({"page": page.name, "title": page_title(page), "items": items})
    return out, test_log
