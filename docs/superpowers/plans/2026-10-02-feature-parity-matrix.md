# Feature Parity Matrix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a capability that is missing or unproven on one of its entry points a failing result in `make verify`, and give agents a reviewer that flags undeclared capabilities and entry points.

**Architecture:** Each feature file under the verify-emailops skill gains a `## Parity` table (capability × entry point, one test reference / `n/a` / `gap` per cell). A stdlib-only Python checker parses the tables, resolves each test reference statically against the source tree and emits one record per cell into the `static` layer of `verify_all.py`; `summary_md.py` adds a "Parity gaps" section. A read-only `completeness-reviewer` subagent reviews diffs against the tables; skills and the root `CLAUDE.md` wire both in.

**Tech Stack:** Python 3 stdlib (`re`, `json`, `pathlib`, `unittest`), Markdown, Claude Code subagent definitions.

**Spec:** `docs/superpowers/specs/2026-10-01-feature-parity-design.md`

## Global Constraints

- Work in the worktree `.claude/worktrees/feature-parity`, branch `chore/feature-parity-matrix`. Check `git rev-parse --show-toplevel` before every edit.
- Python stdlib only; no new dependencies. Scripts run with `python3`, tests with `python3 -m unittest` from the scripts directory (the style of `test_summary_md.py`).
- Code, comments, docstrings and commits in English. Record names and details that land in the verification report are Spanish, like the rest of `verify_all.py` (`paridad › …`, `celda vacía`).
- Every `gap` cell is a `fail`, always (developer decision, 2026-10-01). Never turn a `gap` into `n/a` to make a run green.
- No real personal data anywhere: test fixtures use synthetic names only (`compose.draft`, `Reply`, `example.com`).
- Commit messages: `type: description`, under 72 chars, no Claude co-author trailer (the developer's CLAUDE.md overrides the harness attribution). Run the `privacy-reviewer` subagent (scope `staged`) before each commit.
- Never `git add -A`; stage the files each task names.
- `$SCRATCH` below is the session scratchpad directory; gate and verify output goes there in full, and the report quotes the exit code plus the summary line.
- Starting state (2026-10-02, `origin/main`): `ReplyCompose` already autosaves and saves on leave (`dadb37a0`); `ComposeTabView` still has no save-on-close. The audit reads `main`, not the spec's earlier example table.
- Deviation from the spec, deliberate: `report.py` / `report_all.py` are not modified. Parity records are ordinary `static` records attributed to their feature, so the HTML report already lists them under each feature and in its failures; only the committed Markdown summary gets a dedicated section.

## Review Focus

1. A cell that references a sweep step generated in a loop (`step('Compose', \`borrador en ${entry}\`, …)`) must resolve — otherwise the loop shape the spec prescribes for multi-entry capabilities can never go green. Test in Task 1.
2. A table cell containing an escaped pipe (`\|`), surrounding backticks or extra spaces must parse into the same value as the clean form. Test in Task 1.
3. A feature whose `## Parity` heading exists but has no table, or a feature in `features.json` with no `doc` / a missing file, must produce a `fail` record, not a silent pass or a crash that takes down the rest of the `static` layer. Tests in Tasks 1 and 2.
4. A sub-feature listed under `## Sub-features` with no row in `## Parity` must fail — that is the mechanical form of `MISSING-ROW`. Test in Task 1.
5. A row with fewer cells than there are entry columns must treat the missing cells as empty (fail), not drop them. Test in Task 1.

---

## File map

| File | Responsibility |
|---|---|
| `.claude/skills/verify-emailops/scripts/check_parity.py` (create) | Parse `## Parity` tables, resolve references, judge cells, `check_all()` over `features.json`, CLI entry |
| `.claude/skills/verify-emailops/scripts/test_check_parity.py` (create) | Unit tests on synthetic tables and a synthetic repo tree |
| `.claude/skills/verify-emailops/features.json` (modify) | `doc` key per feature: the feature file name |
| `.claude/skills/verify-emailops/scripts/verify_all.py` (modify) | `layer_static` adds one record per parity cell |
| `.claude/skills/verify-emailops/scripts/summary_md.py` + its test (modify) | "Parity gaps" section |
| `.claude/skills/verify-emailops/features/README.md` (modify) | Contract: five H2s, cell grammar, audit rules |
| `.claude/skills/verify-emailops/features/*.md` (modify, 14 files) | The `## Parity` tables (the audit) |
| `.claude/agents/completeness-reviewer.md` (create) | Read-only reviewer subagent |
| `.claude/skills/maintain-verification/SKILL.md`, `build-ai-feature/SKILL.md`, `fix-ai-bug/SKILL.md` (modify) | Parity steps in the workflows |
| `CLAUDE.md`, `docs/DECISIONS.md` (modify) | Definition of done, reviewer in Git Conventions, decision entry |

---

### Task 1: `check_parity.py` — parse, resolve, judge

**Files:**
- Create: `.claude/skills/verify-emailops/scripts/check_parity.py`
- Test: `.claude/skills/verify-emailops/scripts/test_check_parity.py`

**Interfaces:**
- Produces:
  - `parse_parity(md: str) -> tuple[list[str], list[tuple[str, list[str]]]] | None` — `(entries, rows)`; `None` when there is no `## Parity` section; `([], [])` when the section has no table.
  - `subfeatures(md: str) -> list[str]` — ids from `## Sub-features` bullets (`` - `compose.draft` …``).
  - `sweep_steps(sweep_text: str) -> list[tuple[re.Pattern, re.Pattern]]` — one (feature, step) regex pair per `step(...)` call; `${…}` in a template literal matches any non-empty text.
  - `resolves(ref: str, repo: pathlib.Path, steps) -> bool`
  - `judge(cell: str, repo: pathlib.Path, steps) -> tuple[str, str]` — `(status, detail)`, status `ok` | `fail`.
  - `check_feature(md: str, repo: pathlib.Path, steps) -> list[dict]` — dicts `{capability, entry, status, detail}`.

- [ ] **Step 1: Write the failing tests**

```python
"""Tests for check_parity.py, the capability × entry point matrix check.

    cd .claude/skills/verify-emailops/scripts && python3 -m unittest test_check_parity
"""
import json, pathlib, tempfile, unittest

import check_parity as cp

SWEEP = """
await step('Compose', 'cerrar y borrador', 'x', async () => {});
for (const entry of ['Reply', 'Forward']) {
  await step('Compose', `borrador en ${entry}`, 'x', async () => {});
}
"""

VITEST = """
describe('ReplyCompose drafts', () => {
  it('saves what was typed', async () => {});
  test.each([1])('runs each', () => {});
});
"""

RUST = """
#[test]
fn draft_keeps_its_thread() {}
"""


def md(table, subs=("compose.draft",)):
    bullets = "\n".join(f"- `{s}` something." for s in subs)
    return f"# Compose\n\nText.\n\n## Sub-features\n\n{bullets}\n\n## Parity\n\n{table}\n\n## Gotchas\n\n- none\n"


class Repo:
    """A throwaway tree holding one vitest file, one Rust file and one integration test."""

    def __enter__(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = pathlib.Path(self.tmp.name)
        (root / "src/components").mkdir(parents=True)
        (root / "src/components/Reply.test.tsx").write_text(VITEST)
        (root / "src-tauri/src/db").mkdir(parents=True)
        (root / "src-tauri/src/db/drafts.rs").write_text(RUST)
        (root / "src-tauri/tests").mkdir(parents=True)
        (root / "src-tauri/tests/integration.rs").write_text("#[tokio::test]\nasync fn reply_draft_sends_in_thread() {}\n")
        return root

    def __exit__(self, *exc):
        self.tmp.cleanup()


class ParseTest(unittest.TestCase):
    def test_reads_entries_and_rows(self):
        table = "| Capability | Modal | Reply |\n|---|---|---|\n| compose.draft | e2e:Compose/cerrar y borrador | gap: missing — no autosave |"
        entries, rows = cp.parse_parity(md(table))
        self.assertEqual(entries, ["Modal", "Reply"])
        self.assertEqual(rows, [("compose.draft", ["e2e:Compose/cerrar y borrador", "gap: missing — no autosave"])])

    def test_strips_backticks_spaces_and_keeps_escaped_pipes(self):
        table = "| Capability | Modal |\n|---|---|\n|  `compose.draft`  |  `n/a: a \\| b`  |"
        _, rows = cp.parse_parity(md(table))
        self.assertEqual(rows, [("compose.draft", ["n/a: a | b"])])

    def test_short_row_pads_missing_cells_as_empty(self):
        table = "| Capability | Modal | Reply |\n|---|---|---|\n| compose.draft | e2e:Compose/cerrar y borrador |"
        _, rows = cp.parse_parity(md(table))
        self.assertEqual(rows[0][1], ["e2e:Compose/cerrar y borrador", ""])

    def test_no_section_is_none_and_heading_without_table_is_empty(self):
        self.assertIsNone(cp.parse_parity("# X\n\n## Sub-features\n\n- `a` b\n"))
        self.assertEqual(cp.parse_parity("# X\n\n## Parity\n\nnothing yet\n"), ([], []))

    def test_subfeature_ids(self):
        self.assertEqual(cp.subfeatures(md("", subs=("compose.open", "compose.draft"))), ["compose.open", "compose.draft"])


class ResolveTest(unittest.TestCase):
    def setUp(self):
        self.steps = cp.sweep_steps(SWEEP)

    def test_e2e_literal_and_template_steps(self):
        with Repo() as root:
            self.assertTrue(cp.resolves("e2e:Compose/cerrar y borrador", root, self.steps))
            self.assertTrue(cp.resolves("e2e:Compose/borrador en Reply", root, self.steps))
            self.assertFalse(cp.resolves("e2e:Compose/borrador en ", root, self.steps))
            self.assertFalse(cp.resolves("e2e:Compose/no such step", root, self.steps))

    def test_vitest_title_in_file(self):
        with Repo() as root:
            self.assertTrue(cp.resolves("vitest:src/components/Reply.test.tsx::saves what was typed", root, self.steps))
            self.assertTrue(cp.resolves("vitest:src/components/Reply.test.tsx::runs each", root, self.steps))
            self.assertFalse(cp.resolves("vitest:src/components/Reply.test.tsx::saves", root, self.steps))
            self.assertFalse(cp.resolves("vitest:src/components/Missing.test.tsx::saves what was typed", root, self.steps))

    def test_rust_fn_in_file_and_integration_fn_under_tests(self):
        with Repo() as root:
            self.assertTrue(cp.resolves("rust:src-tauri/src/db/drafts.rs::draft_keeps_its_thread", root, self.steps))
            self.assertFalse(cp.resolves("rust:src-tauri/src/db/drafts.rs::draft_keeps", root, self.steps))
            self.assertTrue(cp.resolves("integration:reply_draft_sends_in_thread", root, self.steps))
            self.assertFalse(cp.resolves("integration:nope", root, self.steps))


class JudgeTest(unittest.TestCase):
    def test_each_cell_kind(self):
        steps = cp.sweep_steps(SWEEP)
        with Repo() as root:
            cases = {
                "": ("fail", "celda vacía"),
                "n/a: inline composer, no modal to close": ("ok", "n/a: inline composer, no modal to close"),
                "n/a:": ("fail", "n/a sin motivo"),
                "n/a": ("fail", "n/a sin motivo"),
                "gap: missing — no autosave": ("fail", "gap: missing — no autosave"),
                "gap: untested — implemented, no test": ("fail", "gap: untested — implemented, no test"),
                "e2e:Compose/cerrar y borrador": ("ok", "e2e:Compose/cerrar y borrador"),
                "e2e:Compose/nope": ("fail", "referencia rota: e2e:Compose/nope"),
                "works fine": ("fail", "celda no reconocida: works fine"),
            }
            for cell, expected in cases.items():
                with self.subTest(cell=cell):
                    self.assertEqual(cp.judge(cell, root, steps), expected)


class CheckFeatureTest(unittest.TestCase):
    def test_one_result_per_cell(self):
        table = "| Capability | Modal | Reply |\n|---|---|---|\n| compose.draft | e2e:Compose/cerrar y borrador | gap: missing — x |"
        with Repo() as root:
            out = cp.check_feature(md(table), root, cp.sweep_steps(SWEEP))
        self.assertEqual([(r["capability"], r["entry"], r["status"]) for r in out],
                         [("compose.draft", "Modal", "ok"), ("compose.draft", "Reply", "fail")])

    def test_subfeature_without_row_fails(self):
        table = "| Capability | Modal |\n|---|---|\n| compose.draft | e2e:Compose/cerrar y borrador |"
        with Repo() as root:
            out = cp.check_feature(md(table, subs=("compose.draft", "compose.escape")), root, cp.sweep_steps(SWEEP))
        self.assertIn({"capability": "compose.escape", "entry": "-", "status": "fail", "detail": "sub-feature sin fila en ## Parity"}, out)

    def test_missing_section_and_empty_table_fail_once(self):
        with Repo() as root:
            steps = cp.sweep_steps(SWEEP)
            self.assertEqual([r["detail"] for r in cp.check_feature("# X\n", root, steps)], ["sin sección ## Parity"])
            self.assertEqual([r["detail"] for r in cp.check_feature("# X\n\n## Parity\n\ntbd\n", root, steps)], ["## Parity sin tabla"])


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd .claude/skills/verify-emailops/scripts && python3 -m unittest test_check_parity 2>&1 | tail -3`
Expected: `ModuleNotFoundError: No module named 'check_parity'` (an import error, not an assertion).

- [ ] **Step 3: Write the implementation**

```python
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

STEP_CALL = re.compile(r"""step\(\s*(['"`])(.*?)\1\s*,\s*(['"`])(.*?)\3""")
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
    return [(_template_regex(feature), _template_regex(step)) for _, feature, _, step in STEP_CALL.findall(sweep_text)]


def _has_fn(path, fn):
    return re.search(rf"\bfn {re.escape(fn)}\b", path.read_text(errors="replace")) is not None


def resolves(ref, repo, steps):
    kind, _, target = ref.partition(":")
    if kind == "e2e":
        feature, _, step = target.partition("/")
        return any(f.match(feature) and s.match(step) for f, s in steps)
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
    out = []
    for capability, cells in rows:
        for entry, cell in zip(entries, cells):
            out.append(_result(capability, entry, *judge(cell, repo, steps)))
    have = {capability for capability, _ in rows}
    out += [_result(sf, "-", "fail", "sub-feature sin fila en ## Parity") for sf in subfeatures(md) if sf not in have]
    return out


if __name__ == "__main__":
    print("check_all() lands in Task 2", file=sys.stderr)
    sys.exit(2)
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd .claude/skills/verify-emailops/scripts && python3 -m unittest -v test_check_parity`
Expected: `OK`, 12 tests.

- [ ] **Step 5: Commit**

```bash
git add .claude/skills/verify-emailops/scripts/check_parity.py .claude/skills/verify-emailops/scripts/test_check_parity.py
git commit -m "test: parse and judge feature parity matrices"
```

---

### Task 2: `check_all()` over `features.json` + CLI

**Files:**
- Modify: `.claude/skills/verify-emailops/scripts/check_parity.py` (replace the `__main__` stub)
- Modify: `.claude/skills/verify-emailops/features.json` (add `doc` to each feature)
- Test: `.claude/skills/verify-emailops/scripts/test_check_parity.py`

**Interfaces:**
- Consumes: `check_feature`, `sweep_steps` from Task 1.
- Produces: `check_all(repo: pathlib.Path = REPO, skill: pathlib.Path = SKILL) -> Iterator[tuple[str, dict]]` — `(feature name from features.json, result dict)`.

- [ ] **Step 1: Write the failing tests** (append to `test_check_parity.py`, before the `if __name__` block)

```python
class CheckAllTest(unittest.TestCase):
    def skill(self, root, features):
        skill = root / "skill"
        (skill / "features").mkdir(parents=True)
        (skill / "scripts").mkdir()
        (skill / "scripts/sweep.mjs").write_text(SWEEP)
        (skill / "features.json").write_text(json.dumps({"features": features}))
        return skill

    def test_results_carry_the_manifest_feature_name(self):
        with Repo() as root:
            skill = self.skill(root, [{"id": "compose", "name": "Redacción y borradores", "doc": "compose.md"}])
            table = "| Capability | Modal |\n|---|---|\n| compose.draft | e2e:Compose/cerrar y borrador |"
            (skill / "features/compose.md").write_text(md(table))
            self.assertEqual([(n, r["status"]) for n, r in cp.check_all(root, skill)], [("Redacción y borradores", "ok")])

    def test_feature_without_doc_or_file_fails_once(self):
        with Repo() as root:
            skill = self.skill(root, [{"id": "a", "name": "A"}, {"id": "b", "name": "B", "doc": "missing.md"}])
            out = list(cp.check_all(root, skill))
        self.assertEqual([(n, r["status"]) for n, r in out], [("A", "fail"), ("B", "fail")])
        self.assertIn("falta doc", out[0][1]["detail"])
        self.assertIn("missing.md", out[1][1]["detail"])
```

- [ ] **Step 2: Run to verify failure**

Run: `cd .claude/skills/verify-emailops/scripts && python3 -m unittest test_check_parity.CheckAllTest 2>&1 | tail -3`
Expected: `AttributeError: module 'check_parity' has no attribute 'check_all'`.

- [ ] **Step 3: Implement** — replace the `if __name__ == "__main__":` stub at the end of `check_parity.py` with:

```python
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
```

- [ ] **Step 4: Add `doc` to every feature in `features.json`**, right after each `"name"`. The mapping (id → file):

| id | doc |
|---|---|
| `cuentas-sync` | `accounts-sync.md` |
| `inbox` | `inbox-open-email.md` |
| `busqueda` | `search.md` |
| `compose` | `compose.md` |
| `skills` | `skills.md` |
| `chat` | `chat.md` |
| `ia-proveedores` | `ai-providers.md` |
| `tagboard` | `tag-board.md` |
| `junk` | `junk.md` |
| `calendario` | `calendar.md` |
| `lenses-tasks` | `lenses-tasks-attachments.md` |
| `contactos-dashboard` | `contacts-dashboard.md` |
| `ajustes` | `settings.md` |
| `cli` | `cli.md` |

Also extend the `_doc` string in `features.json` with: `` `doc` = the feature file under features/ whose `## Parity` table check_parity.py reads. ``

- [ ] **Step 5: Run tests and the CLI**

Run: `cd .claude/skills/verify-emailops/scripts && python3 -m unittest -v test_check_parity`
Expected: `OK`, 14 tests.

Run: `python3 .claude/skills/verify-emailops/scripts/check_parity.py; echo "exit=$?"`
Expected: 14 lines `FAIL <feature> :: - × - — sin sección ## Parity`, then `parity: 14 failing cells`, `exit=1` (no tables yet).

Run: `python3 -c "import json;m=json.load(open('.claude/skills/verify-emailops/features.json'));import pathlib;print([f['id'] for f in m['features'] if not (pathlib.Path('.claude/skills/verify-emailops/features')/f['doc']).is_file()])"`
Expected: `[]`.

- [ ] **Step 6: Commit**

```bash
git add .claude/skills/verify-emailops/scripts/check_parity.py .claude/skills/verify-emailops/scripts/test_check_parity.py .claude/skills/verify-emailops/features.json
git commit -m "feat: check parity matrices of every feature in the manifest"
```

---

### Task 3: wire into `verify_all.py` and `summary_md.py`

**Files:**
- Modify: `.claude/skills/verify-emailops/scripts/verify_all.py` (end of `layer_static`, ~line 189)
- Modify: `.claude/skills/verify-emailops/scripts/summary_md.py` (`summarize`, before `## Failing`)
- Test: `.claude/skills/verify-emailops/scripts/test_summary_md.py`

**Interfaces:**
- Consumes: `check_parity.check_all(REPO, SKILL)`.
- Produces: records `{"feature": <name>, "type": "static", "name": "paridad › <capability> × <entry>", "status": "ok"|"fail", …}`; `summary_md.PARITY_PREFIX = "paridad › "`.

- [ ] **Step 1: Write the failing summary tests** (add to `SummaryTest` in `test_summary_md.py`)

```python
    def test_parity_gaps_are_counted_per_feature(self):
        records = [rec("paridad › compose.draft × Reply", "fail", feature="Redacción y borradores", typ="static"),
                   rec("paridad › compose.escape × Reply", "fail", feature="Redacción y borradores", typ="static"),
                   rec("paridad › chat.ask × Panel", "ok", feature="Chat", typ="static"),
                   rec("tsc --noEmit", "fail", feature="Transversal", typ="static")]
        section = summary_md.summarize(run(records), None).split("## Parity gaps (2)")[1].split("## Failing")[0]
        self.assertIn("| Redacción y borradores | 2 |", section)
        self.assertNotIn("Chat", section)
        self.assertNotIn("tsc", section)

    def test_parity_gaps_section_says_none_when_all_cells_pass(self):
        md = summary_md.summarize(run([rec("paridad › chat.ask × Panel", "ok", feature="Chat", typ="static")]), None)
        self.assertIn("## Parity gaps (0)", md)
```

- [ ] **Step 2: Run to verify failure**

Run: `cd .claude/skills/verify-emailops/scripts && python3 -m unittest test_summary_md 2>&1 | tail -3`
Expected: `FAILED (failures=…)` / `IndexError` on the split — the section does not exist yet.

- [ ] **Step 3: Implement the summary section.** In `summary_md.py` add `PARITY_PREFIX = "paridad › "` under `STATUSES`, and replace the two `failing`/`out +=` lines at the end of `summarize` with:

```python
    failing = [r for r in records if r["status"] == "fail"]
    parity = collections.Counter(r["feature"] for r in failing if r["name"].startswith(PARITY_PREFIX))
    out += [f"## Parity gaps ({sum(parity.values())})", ""]
    out += (["| feature | failing cells |", "|---|---|"] + [f"| {f} | {n} |" for f, n in parity.items()]) if parity else ["- none"]
    out.append("")
    out += [f"## Failing ({len(failing)})", ""] + ([_line(r) for r in failing] or ["- none"]) + [""]
    return "\n".join(out)
```

- [ ] **Step 4: Run summary tests**

Run: `cd .claude/skills/verify-emailops/scripts && python3 -m unittest -v test_summary_md`
Expected: `OK`, 12 tests.

- [ ] **Step 5: Add the records in `verify_all.py`.** At the end of `layer_static` (after the `for name, cmd, sev in checks:` loop), add:

```python
    # Capability × entry point matrices (features/*.md, ## Parity): one record per cell.
    import check_parity
    for feature, r in check_parity.check_all(REPO, SKILL):
        add(feature, "static", f"paridad › {r['capability']} × {r['entry']}", r["status"],
            "" if r["status"] == "ok" else r["detail"],
            desc=f"Matriz ## Parity de la feature: {r['detail']}")
```

- [ ] **Step 6: Run the static layer end to end**

Run: `make verify ARGS="--only git,static" > "$SCRATCH/verify-static.log" 2>&1; echo "exit=$?"; tail -1 "$SCRATCH/verify-static.log"` (with `SCRATCH` = the session scratchpad).
Expected: the last line is `done: {...} → …/results.json`; then
`python3 -c "import json;d=json.load(open('src-tauri/reports/verify/current-full/results.json'));print(sum(r['name'].startswith('paridad') for r in d['records']))"` prints `14`.

- [ ] **Step 7: Commit**

```bash
git add .claude/skills/verify-emailops/scripts/verify_all.py .claude/skills/verify-emailops/scripts/summary_md.py .claude/skills/verify-emailops/scripts/test_summary_md.py
git commit -m "feat: report parity cells in the static verification layer"
```

---

### Task 4: feature-file contract

**Files:**
- Modify: `.claude/skills/verify-emailops/features/README.md` (section "Feature entry contract")

- [ ] **Step 1: Replace the "Feature entry contract" section** with:

```markdown
## Feature entry contract

Each file has an H1, one paragraph of user-visible behaviour, then exactly five H2s:
`Sub-features`, `How to get to it (user POV)`, `Parity`, `Driving it with verify.sh`,
`Gotchas`, and ends with a "test kind per case" table. A layer a feature does not have is
a row of that table that says `n/a` and why.

### `## Parity`

One table: a row per id in `Sub-features`, a column per entry point listed in "How to
get to it". A feature with one entry point still has the table, with one column — that
column is what makes the second entry point a visible decision. Each cell is exactly one of:

| Cell | Meaning |
|---|---|
| `e2e:<Feature>/<step>` | a `step(...)` in `scripts/sweep.mjs`; a template step (`` `borrador en ${entry}` ``) matches any value |
| `vitest:<file>::<test title>` | an `it()` / `test()` with that exact title in that file |
| `rust:<file>::<fn>` | a test fn in that Rust file |
| `integration:<fn>` | a test fn under `src-tauri/tests/` |
| `n/a: <reason>` | the capability does not apply to that entry point |
| `gap: missing — <what>` | not implemented there |
| `gap: untested — <what>` | implemented, but no test listed here proves it there |

`scripts/check_parity.py` (static layer of `make verify`) fails every `gap`, every empty
cell, every `n/a` without a reason, every reference that does not resolve, and every
sub-feature without a row. It proves a referenced test exists, not that it passes — the
other layers of the same run do that.

Rules when filling a cell:

- The test must go through **that** entry point: a component test of the entry's
  component, or an e2e step that opens it. A test of a shared helper (`src/lib/…`) does not
  prove an entry point calls it — that wiring is exactly what goes missing.
- A capability that lives entirely below the entry points (a backend service every entry
  calls the same way) puts its backend test in the first column and
  `n/a: backend, same path for every entry point` in the others.
- A `gap` is closed by a fix plus a test, or by a test alone. Never by rewriting it as `n/a`.
- When several columns need the same e2e proof, write one sweep loop over the entry points
  (the shape of the settings-tab loop) and reference its template step.
```

- [ ] **Step 2: Verify the docs-path guard accepts it**

Run: `uv run --no-project scripts/check-docs-paths.py; echo "exit=$?"`
Expected: `exit=0`.

- [ ] **Step 3: Commit**

```bash
git add .claude/skills/verify-emailops/features/README.md
git commit -m "docs: add the Parity section to the feature file contract"
```

---

### Task 5: Compose audit (`features/compose.md`)

**Files:**
- Modify: `.claude/skills/verify-emailops/features/compose.md`

Entry points on `main` (2026-10-02): **Compose modal** (`src/components/ComposeModal.tsx`), **Compose tab** (`src/components/EmailView/ComposeTabView.tsx`, also what Drafts → Continue editing opens), **Reply**, **Reply all**, **Forward** (all three `src/components/EmailView/ReplyCompose.tsx` with `mode`, mounted from `EmailView.tsx`).

- [ ] **Step 1: Extend `## Sub-features`** with the capabilities the composers offer that the list lacks. Add each as a bullet in the existing style:

```markdown
- `compose.flush-on-close` closing or leaving the composer within the autosave delay still saves the last edit.
- `compose.send-warnings` before sending, a warning appears for a promised but missing attachment and similar slips.
- `compose.ai` the composer offers AI drafting or rewriting.
- `compose.escape` Escape closes the composer.
- `compose.recipients` an address typed but not turned into a chip is still sent to.
- `compose.attachments` files can be attached.
- `compose.translate` the body can be translated before sending.
```

- [ ] **Step 2: Fill `## Parity`.** Insert the section between "How to get to it (user POV)" and "Driving it with verify.sh". Start from this table (references confirmed to exist on 2026-10-02) and resolve every `?` cell by reading the entry's component and its tests, applying the README rules from Task 4:

```markdown
## Parity

| Capability | Compose modal | Compose tab | Reply | Reply all | Forward |
|---|---|---|---|---|---|
| compose.open | e2e:Compose/abrir | ? | ? | ? | ? |
| compose.fields | e2e:Compose/rellenar | ? | ? | ? | ? |
| compose.draft | e2e:Compose/cerrar y borrador | e2e:Compose/borrador con tabla | vitest:src/components/EmailView/ReplyCompose.draft.test.tsx::saves what was typed when the user leaves before the autosave fires | ? | ? |
| compose.flush-on-close | ? | gap: missing — ComposeTabView has no createDebouncedDraftSaver flush on close | vitest:src/components/EmailView/ReplyCompose.draft.test.tsx::saves what was typed when the user leaves before the autosave fires | ? | ? |
| compose.discard | e2e:Compose/descartar borrador | ? | vitest:src/components/EmailView/ReplyCompose.draft.test.tsx::deletes the saved draft when the reply is cancelled | ? | ? |
| compose.tables | ? | e2e:Compose/borrador con tabla | ? | ? | ? |
| compose.conflict | ? | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| compose.send | e2e:Compose/enviar sin credenciales | ? | ? | ? | ? |
| compose.send-warnings | ? | ? | ? | ? | ? |
| compose.ai | vitest:src/components/ComposeModal.aiDraft.test.tsx::? | ? | ? | ? | ? |
| compose.escape | ? | ? | ? | ? | ? |
| compose.recipients | ? | vitest:src/components/EmailView/ComposeTabView.pendingRecipient.test.tsx::? | vitest:src/components/EmailView/ReplyCompose.pendingRecipient.test.tsx::enables Send and sends to the pending To and Cc addresses | ? | ? |
| compose.attachments | ? | ? | ? | ? | ? |
| compose.translate | ? | ? | ? | ? | ? |
```

How to resolve a `?`:
1. Open the entry's component and search for the capability (e.g. `findSendWarnings`, `onKeyDown`/`Escape`, `AiInstructionBar`, `handleDraftWithAI`). Not present → `gap: missing — <one line>`.
2. Present → search that component's tests (`<Component>*.test.tsx`) and `sweep.mjs` for one that exercises it **through that component**. Found → the reference; for vitest, copy the `it(...)` title exactly. Not found → `gap: untested — <one line>`.
3. Reply all and Forward share `ReplyCompose` with Reply: a test rendered with `mode="reply"` proves Reply only. Mark them `gap: untested — ReplyCompose tests render mode="reply" only` unless a test renders that mode.
4. `compose.conflict` first column: the Rust/integration test of the dirty-marker plan (`sync::draft_plan` / `db::drafts` per the "test kind per case" table); copy its exact path and fn.

- [ ] **Step 3: Update "How to get to it"** so the prose lists the same five entry points as the table columns.

- [ ] **Step 4: Run the check for this feature**

Run: `python3 .claude/skills/verify-emailops/scripts/check_parity.py | awk '/Redacción/'`
Expected: only `gap: …` lines for compose — no `celda vacía`, `referencia rota`, `celda no reconocida`, `n/a sin motivo` or `sub-feature sin fila`. Fix any such line before committing. Report the gap count: `python3 .claude/skills/verify-emailops/scripts/check_parity.py | awk '/Redacción/ && /gap:/' | wc -l`.

- [ ] **Step 5: The reply-draft `email_id` bug.** On the older branch, editing a chat-generated reply draft in the Compose tab nulled `drafts.email_id`. On `main`, `src/lib/composeDraft.ts` carries `emailId` and one upsert in `src-tauri/src/db/drafts.rs` uses `COALESCE(drafts.email_id, excluded.email_id)` while another still does `email_id = excluded.email_id`. Read both upserts and their callers: if the plain overwrite is reachable from a save that omits `emailId`, add one line to `NEXT-SESSION.md` (repo root, gitignored or untracked — do not commit it) describing it; otherwise record nothing.

- [ ] **Step 6: Commit**

```bash
git add .claude/skills/verify-emailops/features/compose.md
git commit -m "test: audit compose parity across its five entry points"
```

---

### Task 6: Audit inbox, search, accounts-sync, junk

**Files:**
- Modify: `.claude/skills/verify-emailops/features/{inbox-open-email,search,accounts-sync,junk}.md`

Same procedure as Task 5, per file:

- [ ] **Step 1:** List the entry points from "How to get to it" and from the code (each place in `src/components` that opens or triggers the feature — e.g. for inbox: a single account's list, **All accounts** (unified inbox), Tag Board row, search result row). Make the prose list and the table columns identical.
- [ ] **Step 2:** Add any capability the feature's components offer that `Sub-features` lacks (one bullet each, existing style).
- [ ] **Step 3:** Fill `## Parity` (placed after "How to get to it") using the README rules; every cell a reference, `n/a: <reason>` or `gap: missing|untested — <what>`. Copy test titles and step names exactly from the files (`sweep.mjs` step names: `awk '/step\(/' .claude/skills/verify-emailops/scripts/sweep.mjs`).
- [ ] **Step 4:** Run `python3 .claude/skills/verify-emailops/scripts/check_parity.py`; for these four features, only `gap:` lines may remain.
- [ ] **Step 5:** Commit: `git add` the four files; `git commit -m "test: audit parity for inbox, search, accounts and junk"`.

---

### Task 7: Audit chat, skills, ai-providers, cli

**Files:**
- Modify: `.claude/skills/verify-emailops/features/{chat,skills,ai-providers,cli}.md`

- [ ] **Step 1–4:** As Task 6, Steps 1–4. Entry-point hints: chat — docked panel, chat opened from an email/thread context, `/` skill suggestions, `emailops-cli chat`; ai-providers — Settings → AI tabs and the Logs status-bar model switcher; cli — each command is a capability, the single column is `emailops-cli --json` unless a capability is also reachable from the app.
- [ ] **Step 5:** Commit: `git commit -m "test: audit parity for chat, skills, AI providers and CLI"` with the four files added.

---

### Task 8: Audit tag-board, calendar, lenses-tasks-attachments, contacts-dashboard, settings

**Files:**
- Modify: `.claude/skills/verify-emailops/features/{tag-board,calendar,lenses-tasks-attachments,contacts-dashboard,settings}.md`

- [ ] **Step 1–4:** As Task 6, Steps 1–4. Entry-point hints: attachments — the attachment strip in a thread, the Attachments view, the chat sources list; settings — the dialog opened from the sidebar and from in-app deep links (e.g. the context-budget suggestion), plus Escape vs Close.
- [ ] **Step 5:** Run `python3 .claude/skills/verify-emailops/scripts/check_parity.py > "$SCRATCH/parity.txt"; echo "exit=$?"; tail -1 "$SCRATCH/parity.txt"`. Expected: `exit=1` and every `FAIL` line in the file contains `gap:` — check with `awk '/^FAIL/ && !/gap:/' "$SCRATCH/parity.txt"` printing nothing.
- [ ] **Step 6:** Commit: `git commit -m "test: audit parity for tag board, calendar, lenses, contacts, settings"` with the five files added.

---

### Task 9: `completeness-reviewer` subagent

**Files:**
- Create: `.claude/agents/completeness-reviewer.md`

- [ ] **Step 1: Write the agent**

```markdown
---
name: completeness-reviewer
description: Review a staged diff or a branch diff for features that land on one entry point and not its siblings, or that add a capability or entry point the feature's Parity matrix does not declare. Reads the `## Parity` tables in .claude/skills/verify-emailops/features/. Use before committing (scope staged) and before a push or PR (scope branch), alongside privacy-reviewer. Read-only; it reports, never edits.
tools: Bash, Read, Grep, Glob
model: sonnet
---

You check that an EmailOps change is complete across the places a user reaches the same
capability. A feature often has several entry points — the Compose modal, the Compose
tab, Reply, Reply all and Forward all write an email — and a change made in one component
silently skips its siblings. Each feature file under
`.claude/skills/verify-emailops/features/` has a `## Parity` table: a row per capability
(the ids in `## Sub-features`), a column per entry point, a test reference, `n/a: <reason>`
or `gap: …` per cell. You compare the diff with those tables. You never edit, stage,
commit or delete anything.

## What you are given

- **staged** — review `git diff --cached` in the given checkout.
- **branch** — review `git diff origin/main...HEAD` and `git log origin/main..HEAD --format=%B`.

If nothing is specified, review **staged**.

## Hard limits

- Bash is for `git diff`, `git log`, `git show`, `git status`, `git ls-files` and
  `python3 .claude/skills/maintain-verification/scripts/verify_changes.py <base>` only.
- Never open any database, `.emailops-*` directory, `private-evals/` or `src-tauri/reports/`.

## How to review

1. Map changed files to features: run `verify_changes.py` with the diff base
   (`origin/main` for branch, `HEAD` for staged) and read `features.json` `doc` keys.
2. For each touched feature, read its feature file: `Sub-features`, "How to get to it",
   `## Parity`.
3. For each user-visible change in the diff (a new handler, prop, button, keyboard
   shortcut, API call from a component, i18n string, sweep step), decide:
   - does it add a capability that has no row? → `MISSING-ROW`
   - does it add a way to reach a feature (a component that composes, a new button,
     route, view, CLI command, deep link) that has no column? → `MISSING-COLUMN`
   - does it change a capability in one entry point's component while a sibling column
     of the same row is neither changed in the diff nor `n/a`? Read the sibling
     component to confirm it lacks the same behaviour. → `SIBLING-SKIPPED`
   - does it close or open a gap without the `## Parity` cell changing in the same
     diff? → `MATRIX-STALE`
4. Internal refactors with no user-visible effect produce no finding.

## Report — exactly this shape

```
scope: <what was reviewed> · checkout: <path> · <n> files, <n> commits · features: <ids>
verdict: OK | FINDINGS
```

Then one line per finding:

```
<MISSING-ROW|MISSING-COLUMN|SIBLING-SKIPPED|MATRIX-STALE> · <feature id> · <capability or entry point> · <file:line in the diff> · <one sentence: what is missing where>
```

For `SIBLING-SKIPPED`, name the sibling component file that lacks the behaviour. Do not
propose code; the implementer decides between fixing the sibling and an `n/a` with a reason.
```

- [ ] **Step 2: Prove it catches a skipped sibling.** Create a throwaway branch and a synthetic change that adds Escape-to-close to `ReplyCompose` only:

```bash
git switch -c tmp/reviewer-probe
```

Edit `src/components/EmailView/ReplyCompose.tsx`: add to the outermost element `onKeyDown={(e) => { if (e.key === 'Escape') onCancel(); }}`. Stage it (`git add src/components/EmailView/ReplyCompose.tsx`), then dispatch the `completeness-reviewer` agent with scope `staged` in this worktree.
Expected: `verdict: FINDINGS` with `SIBLING-SKIPPED · compose · compose.escape …` naming `ComposeTabView.tsx`, and `MATRIX-STALE` (the Reply cell of `compose.escape` did not change).

- [ ] **Step 3: Prove it stays quiet on an internal refactor.** `git restore --staged --worktree src/components/EmailView/ReplyCompose.tsx`; rename a local variable inside one function of `src/lib/composeDraft.ts`, stage it, dispatch the reviewer with scope `staged`. Expected: `verdict: OK`.

- [ ] **Step 4: Clean up the probe** (ask the developer before deleting the branch, per the repo's destructive-action rule):

```bash
git restore --staged --worktree src/lib/composeDraft.ts
git switch chore/feature-parity-matrix
git branch -D tmp/reviewer-probe
```

- [ ] **Step 5: Commit**

```bash
git add .claude/agents/completeness-reviewer.md
git commit -m "feat: add completeness-reviewer subagent for parity drift"
```

---

### Task 10: Workflow integration and decision record

**Files:**
- Modify: `CLAUDE.md` (Git Conventions, ~line 161)
- Modify: `.claude/skills/maintain-verification/SKILL.md` (steps 1, 2, 4)
- Modify: `.claude/skills/build-ai-feature/SKILL.md` (Phase 1 bullets, ~line 113)
- Modify: `.claude/skills/fix-ai-bug/SKILL.md` (after the root-cause paragraph, ~line 142)
- Modify: `docs/DECISIONS.md` (append at the bottom)

- [ ] **Step 1: `CLAUDE.md`.** Replace the line `- Before committing, pushing or opening a PR, run the \`privacy-reviewer\` subagent (see Privacy First) alongside the gates.` with:

```markdown
- Before committing, pushing or opening a PR, run the `privacy-reviewer` subagent (see Privacy First) and the `completeness-reviewer` subagent (`.claude/agents/completeness-reviewer.md`, same scopes) alongside the gates. Resolve every `completeness-reviewer` finding: fix the sibling entry point, or update the feature's `## Parity` cell (`n/a: <reason>` only when the capability truly does not apply there).
- **Definition of done** for a feature or fix: the `commit` gates are green; every feature it touches has a `## Parity` table with no empty cell and no broken reference (`python3 .claude/skills/verify-emailops/scripts/check_parity.py`); `completeness-reviewer` reports `OK`.
```

- [ ] **Step 2: `maintain-verification/SKILL.md`.**
  - In the step-1 table, add the row: `| New entry point (component, button, route, CLI command) or new user-visible capability | \`## Parity\` in the feature file: a column or a row, every cell a test, \`n/a: <reason>\` or \`gap\` |`
  - In step 2, after "A feature touched in this change never keeps a missing layer silently.", add: `The same holds for \`## Parity\` cells: a \`gap\` is closed by a test (and a fix when it is \`missing\`), never by rewriting it as \`n/a\`.`
  - In step 4, add the bullet: `` - `## Parity`: one row per sub-feature, one column per entry point; `python3 .claude/skills/verify-emailops/scripts/check_parity.py` shows only the gaps you meant to leave. ``

- [ ] **Step 3: `build-ai-feature/SKILL.md` Phase 1.** Add after the `User-facing?` bullet:

```markdown
- **Parity** — which feature files' `## Parity` rows (capabilities) and columns (entry
  points) this touches, e.g. "`compose.ai` × Compose modal, Compose tab, Reply". A
  capability that will exist on some entry points and not others gets `n/a: <reason>`
  cells decided now, not discovered later.
```

- [ ] **Step 4: `fix-ai-bug/SKILL.md`.** After the paragraph that starts "When the symptom reproduces, walk the user through the root cause", add:

```markdown
Then open the feature's `## Parity` table (`.claude/skills/verify-emailops/features/`)
and check the sibling columns of the affected row: if the same defect exists on another
entry point, the fix covers it too, or the report says why not.
```

- [ ] **Step 5: `docs/DECISIONS.md`.** Append:

```markdown
## 2026-10-01 — Feature parity matrix gates verification

**Decision:** Every feature file in the verify-emailops skill carries a `## Parity` table
(capability × entry point; each cell a test reference, `n/a: <reason>` or `gap`), checked
by `check_parity.py` in the static layer of `make verify`. Every `gap`, empty cell or
broken reference is a failing result. A read-only `completeness-reviewer` subagent runs
with `privacy-reviewer` before commits and pushes to flag undeclared capabilities, entry
points and skipped siblings.
**Context:** Features landed on the component a change touched and not on its siblings:
draft autosave reached the Compose modal and tab but not Reply/Reply all/Forward, save-on-close
only the modal, pre-send warnings only Reply. Specs and the e2e sweep listed entry points
in prose and checked that the Reply buttons existed, never what they did.
**Rejected:**
- *LLM reviewer alone*: it can miss a sibling the same way the implementing agent did; a
  table a script checks cannot be forgotten.
- *Gaps blocking only at release*: the developer chose to have every gap fail every run.
- *Merging the three composers now*: removes the cause for compose only; deferred until
  the audit shows where shared components would remove the most gaps.
```

- [ ] **Step 6: Verify the docs guards**

Run: `uv run --no-project scripts/check-docs-paths.py; echo "exit=$?"`
Expected: `exit=0`.

- [ ] **Step 7: Commit**

```bash
git add CLAUDE.md .claude/skills/maintain-verification/SKILL.md .claude/skills/build-ai-feature/SKILL.md .claude/skills/fix-ai-bug/SKILL.md docs/DECISIONS.md
git commit -m "docs: make parity part of the definition of done"
```

---

### Task 11: Final verification

- [ ] **Step 1:** `cd .claude/skills/verify-emailops/scripts && python3 -m unittest -v test_check_parity test_summary_md` → `OK`.
- [ ] **Step 2:** `make verify ARGS="--tier quick" > "$SCRATCH/verify-quick.log" 2>&1; echo "exit=$?"` then `tail -1 "$SCRATCH/verify-quick.log"`. Generate the summary with `python3 .claude/skills/verify-emailops/scripts/summary_md.py src-tauri/reports/verify/current-full/results.json "$SCRATCH"` and read its `## Parity gaps (N)` table. Report N and the per-feature counts to the developer; the only new static failures must be `paridad ›` records whose detail starts with `gap:`.
- [ ] **Step 3:** Dispatch `gate-runner` with set `commit` in this worktree; act on its report.
- [ ] **Step 4:** Dispatch `privacy-reviewer` (scope `branch`) and `completeness-reviewer` (scope `branch`); resolve findings.
- [ ] **Step 5:** Do not push. Tell the developer the branch is ready, with the gap counts per feature from Step 2.
