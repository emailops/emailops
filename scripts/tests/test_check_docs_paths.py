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


def test_a_gitignored_dotfile_gets_the_same_verdict_whether_or_not_it_exists(tmp_path, monkeypatch):
    # `.claude/settings.local.json` is gitignored and per-checkout: present in
    # the developer's main checkout, absent in CI and in fresh worktrees. An
    # allowlist entry for it went "stale" wherever the file existed, so the
    # pre-commit hook failed in one checkout and passed in the next.
    import os
    import subprocess

    root = tmp_path / "repo"
    (root / "docs").mkdir(parents=True)
    (root / ".gitignore").write_text(".claude/settings.local.json\n")
    (root / "docs" / "guide.md").write_text("Allow the command in `.claude/settings.local.json`.\n")
    # A pre-commit hook exports GIT_INDEX_FILE / GIT_DIR for the real checkout;
    # inherited, `git add` here would overwrite that index with this fixture
    # and the checker would list the real repo's files under this root.
    for name in [k for k in os.environ if k.startswith("GIT_")]:
        monkeypatch.delenv(name)
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    subprocess.run(["git", "-C", str(root), "add", "."], check=True)
    monkeypatch.setattr(check_docs_paths, "ROOT", root)
    monkeypatch.setattr(check_docs_paths, "ALLOWED_UNRESOLVED", {})

    assert check_docs_paths.main() == 0
    (root / ".claude").mkdir()
    (root / ".claude" / "settings.local.json").write_text("{}")
    assert check_docs_paths.main() == 0
