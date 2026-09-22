"""Evaluate every claim in docs/site/claims.toml against the repo.

Produces one record per claim for check_docs_run.py. The file/tests checks run
here; `app` checks are satisfied from the sweep's results when the run drove
the app, and reported as skipped otherwise.

A claim's status is the conjunction of its checks:

    any check fails                  → fail
    otherwise, all checks are `none` → skip   (the block makes no claim)
    otherwise, any `manual` or an
      app check that did not run     → info   (shown as MANUAL)
    otherwise                        → ok
"""

import fnmatch
import json
import pathlib
import re
import subprocess

from docs_claims_lib import LANGS, ROOT, blocks, front_matter, load_catalog, pages

KINDS = ("file", "tests", "app", "release", "manual", "none")
# Method → report type. The type is how a claim was proven, so the report's
# per-page summary reads as "how much of this page does the code vouch for".
TYPE_OF = {"file": "source", "tests": "tests", "app": "doc", "release": "release", "manual": "manual", "none": "manual"}


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


def evaluate(app_results):
    """app_results: {claim id: sweep record} when the app was driven, else None."""
    catalog = load_catalog()
    idx = claim_index()

    tests_wanted = {t for c in catalog.values() for ch in c.get("checks", []) if "tests" in ch for t in ch["tests"]}
    test_status, test_log = run_rust_tests(tests_wanted) if tests_wanted else ({}, "")

    records = []
    for cid, (page, block) in idx.items():
        entry = catalog.get(cid)
        section = block.section if block.section == block.heading or not block.heading else f"{block.section} › {block.heading}"
        base = {
            "feature": page_title(page),
            "name": f"{section or 'Introducción'} · {cid}",
            "claim": squash(block.text) if block.kind != "code" else block.text,
            "page": page.name,
            "line": block.start,
        }
        if entry is None:
            records.append({**base, "type": "manual", "status": "fail",
                            "detail": "sin entrada en claims.toml", "results": []})
            continue

        results = []
        for ch in entry.get("checks", []):
            k = kind(ch)
            if k == "file":
                ok, why = check_file(ch, block.text)
                results.append(("file", "ok" if ok else "fail", why))
            elif k == "tests":
                for t in ch["tests"]:
                    st = test_status.get(t, "missing")
                    why = {"ok": f"test {t} pasa", "fail": f"test {t} FALLA",
                           "missing": f"test {t} no existe (¿renombrado?)"}[st]
                    results.append(("tests", "ok" if st == "ok" else "fail", why))
            elif k == "app":
                if app_results is None:
                    results.append(("app", "pending", "comprobación en la app no ejecutada (--with-app)"))
                else:
                    r = app_results.get(cid)
                    if r is None:
                        results.append(("app", "fail", f"ninguna fase de la app comprobó claim:{cid}"))
                    elif r["status"] == "skip":
                        # Not observable here (e.g. greying out on a machine where every
                        # model fits): not proven, so it must not read as OK.
                        results.append(("app", "manual", r.get("detail", "")))
                    else:
                        results.append(("app", "ok" if r["status"] == "ok" else "fail", r.get("detail", "")))
            elif k == "release":
                ok, why = check_release(ch, block.text)
                results.append(("release", "ok" if ok else "fail", why))
            elif k == "manual":
                results.append(("manual", "manual", ch["manual"]))
            elif k == "none":
                results.append(("none", "none", ch["none"]))

        states = [s for _, s, _ in results]
        if "fail" in states:
            status = "fail"
        elif states and all(s == "none" for s in states):
            status = "skip"
        elif "manual" in states or "pending" in states:
            status = "info"
        else:
            status = "ok"

        # The report type is the strongest method that ran for this claim.
        methods = [m for m, s, _ in results if s not in ("none",)]
        order = ["app", "release", "tests", "file", "manual", "none"]
        best = next((m for m in order if m in methods), "none")
        detail = "; ".join(why for _, s, why in results if s == "fail") or "; ".join(
            why for _, _, why in results)
        app = (app_results or {}).get(cid) or {}
        records.append({**base, "type": TYPE_OF[best], "status": status, "detail": detail,
                        "results": results, "fix": app.get("fix") or entry.get("fix", ""),
                        "shots": app.get("shots", [])})
    return records, test_log
