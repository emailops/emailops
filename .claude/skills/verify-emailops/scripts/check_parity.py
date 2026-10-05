#!/usr/bin/env python3
"""Parity matrix check: every capability × entry point of a feature is proven or explained.

    check_parity.py        # prints each failing cell, exit 1 if any

Each feature file named by `features.json` (`doc` key) carries a `## Parity` table: rows
are the ids from `## Sub-features`, columns are the user's entry points, and each cell is
exactly one of

    e2e:<Feature>/<step>          a step(...) in sweep.mjs (`${…}` in a template step matches any text)
    vitest:<file>::<test title>   an it()/test() with that exact title in the file
    rust:<file>::<fn>             a fn with that name in the file
    integration:<fn>              a fn with that name under src-tauri/tests/
    n/a: <reason>                 the capability does not apply to that entry point
    gap: missing — … | gap: untested — …   a known hole; always a failure

References are resolved statically: this proves the test exists, not that it passes —
the rust, vitest and e2e layers of the same run do that. verify_all.py adds one record
per cell to the static layer.
"""
import json, pathlib, re, sys

HERE = pathlib.Path(__file__).resolve().parent
SKILL = HERE.parent
REPO = SKILL.parents[2]
REF_KINDS = ("e2e", "vitest", "rust", "integration")

STEP_CALL = re.compile(r"""step\(\s*(['"`])(.*?)\1\s*,\s*(?:(['"`])(.*?)\3|([A-Za-z_$][\w$]*))\s*,""")
SUBFEATURE = re.compile(r"^- `([\w.-]+)`", re.M)


def _section(md, title):
    m = re.search(rf"^## {re.escape(title)}[ \t]*$(.*?)(?=^## |\Z)", md, re.M | re.S)
    return m.group(1) if m else None


def _cells(line):
    parts = re.split(r"(?<!\\)\|", line.strip().strip("|"))
    return [p.strip().strip("`").strip().replace("\\|", "|") for p in parts]


def parse_parity(md):
    body = _section(md, "Parity")
    if body is None:
        return None
    lines = [l for l in body.splitlines() if l.strip().startswith("|")]
    if len(lines) < 2:
        return [], []
    entries = _cells(lines[0])[1:]
    rows = []
    for line in lines[2:]:  # lines[1] is the |---| separator
        cells = _cells(line)
        rows.append((cells[0], (cells[1:] + [""] * len(entries))[:len(entries)]))
    return entries, rows


def subfeatures(md):
    return SUBFEATURE.findall(_section(md, "Sub-features") or "")


def _template_regex(text):
    parts = re.split(r"\$\{[^}]*\}", text)
    return re.compile("^" + ".+".join(re.escape(p) for p in parts) + "$")


def sweep_steps(sweep_text):
    # A step named by a bare loop variable (`step('Vistas', view, …)`) matches any name.
    return [(_template_regex(feature), _template_regex(step) if not variable else re.compile(r"^.+$"))
            for _, feature, _, step, variable in STEP_CALL.findall(sweep_text)]


def _has_fn(path, fn):
    # Only a test fn counts: `#[test]` / `#[tokio::test]`, other attributes allowed in between.
    test_fn = rf"#\[(?:tokio::)?test[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub )?(?:async )?fn {re.escape(fn)}\b"
    return bool(fn) and re.search(test_fn, path.read_text(errors="replace")) is not None


def resolves(ref, repo, steps):
    kind, _, target = ref.partition(":")
    if kind == "e2e":
        # Sweep feature names may hold a slash (`Chat/Formularios`): try every split point.
        splits = [(target[:i], target[i + 1:]) for i, c in enumerate(target) if c == "/"]
        return any(f.match(feature) and s.match(step) for feature, step in splits for f, s in steps)
    if kind == "vitest":
        path, _, title = target.partition("::")
        f = repo / path
        call = rf"""\b(?:it|test)(?:\.\w+(?:\([^)]*\))?)?\(\s*(['"`]){re.escape(title)}\1"""
        return f.is_file() and re.search(call, f.read_text(errors="replace")) is not None
    if kind == "rust":
        path, _, fn = target.partition("::")
        f = repo / path
        return f.is_file() and _has_fn(f, fn)
    if kind == "integration":
        return any(_has_fn(f, target) for f in (repo / "src-tauri/tests").rglob("*.rs"))
    return False


def judge(cell, repo, steps):
    if not cell:
        return "fail", "celda vacía"
    if cell.startswith("n/a"):
        reason = cell[3:].lstrip(":").strip()
        return ("ok", f"n/a: {reason}") if reason else ("fail", "n/a sin motivo")
    if cell.startswith("gap:"):
        return "fail", cell
    if cell.split(":", 1)[0] in REF_KINDS:
        return ("ok", cell) if resolves(cell, repo, steps) else ("fail", f"referencia rota: {cell}")
    return "fail", f"celda no reconocida: {cell}"


def _result(capability, entry, status, detail):
    return {"capability": capability, "entry": entry, "status": status, "detail": detail}


def check_feature(md, repo, steps):
    parsed = parse_parity(md)
    if parsed is None:
        return [_result("-", "-", "fail", "sin sección ## Parity")]
    entries, rows = parsed
    if not entries:
        return [_result("-", "-", "fail", "## Parity sin tabla")]
    if not rows:
        return [_result("-", "-", "fail", "## Parity sin filas")]
    out = []
    for capability, cells in rows:
        for entry, cell in zip(entries, cells):
            out.append(_result(capability, entry, *judge(cell, repo, steps)))
    have = {capability for capability, _ in rows}
    out += [_result(sf, "-", "fail", "sub-feature sin fila en ## Parity") for sf in subfeatures(md) if sf not in have]
    return out


def check_all(repo=REPO, skill=SKILL):
    manifest = json.loads((skill / "features.json").read_text())
    steps = sweep_steps((skill / "scripts/sweep.mjs").read_text())
    for feature in manifest["features"]:
        doc = feature.get("doc")
        path = skill / "features" / doc if doc else None
        if path is None or not path.is_file():
            yield feature["name"], _result("-", "-", "fail", f"sin fichero de feature ({doc or 'falta doc en features.json'})")
            continue
        for result in check_feature(path.read_text(), repo, steps):
            yield feature["name"], result


def main():
    failing = 0
    for name, r in check_all():
        if r["status"] == "fail":
            failing += 1
            print(f"FAIL {name} :: {r['capability']} × {r['entry']} — {r['detail']}")
    print(f"parity: {failing} failing cells")
    return 1 if failing else 0


if __name__ == "__main__":
    sys.exit(main())
