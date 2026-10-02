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


if __name__ == "__main__":
    unittest.main()
