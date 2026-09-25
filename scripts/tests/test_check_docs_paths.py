"""The docs path check: which quoted paths count as resolving."""

import importlib.util
import pathlib

SCRIPT = pathlib.Path(__file__).resolve().parent.parent / "check-docs-paths.py"
spec = importlib.util.spec_from_file_location("check_docs_paths", SCRIPT)
check_docs_paths = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_docs_paths)


def test_a_path_inside_the_repo_resolves(tmp_path, monkeypatch):
    root = tmp_path / "repo"
    (root / "docs").mkdir(parents=True)
    (root / "docs" / "guide.md").write_text("x")
    monkeypatch.setattr(check_docs_paths, "ROOT", root)
    assert check_docs_paths.resolves("docs/guide.md", root / "docs", [])


def test_a_path_escaping_the_repo_does_not_resolve_even_if_a_sibling_checkout_has_it(tmp_path, monkeypatch):
    # A developer with the tap repo cloned next to this one must get the same
    # verdict as CI, where the sibling does not exist.
    root = tmp_path / "repo"
    (root / "homebrew").mkdir(parents=True)
    sibling = tmp_path / "homebrew-tap" / "Casks"
    sibling.mkdir(parents=True)
    (sibling / "emailops.rb").write_text("x")
    monkeypatch.setattr(check_docs_paths, "ROOT", root)
    candidate = "../homebrew-tap/Casks/emailops.rb"
    assert not check_docs_paths.resolves(candidate, root / "homebrew", [])
    assert not check_docs_paths.resolves(candidate, root, [])
