"""scripts/update_security_audit_skill.sh: refresh the vendored security-audit skill.

The skill is vendored, not installed, so every upstream change arrives as a
reviewable diff. The script copies skills/security-audit plus LICENSE at a
chosen commit, records that commit in SOURCE, lists the upstream commits since
the previous pin, and refuses a tree that carries a symlink.
"""

import os
import subprocess
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "update_security_audit_skill.sh"
DEST = Path(".claude/skills/security-audit")


def _git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@example.invalid", *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def _upstream(tmp_path: Path) -> Path:
    repo = tmp_path / "upstream"
    (repo / "skills" / "security-audit").mkdir(parents=True)
    subprocess.run(["git", "init", "-q", "-b", "main", str(repo)], check=True)
    (repo / "LICENSE").write_text("MIT License\n")
    (repo / "README.md").write_text("not vendored\n")
    return repo


def _commit(repo: Path, files: dict[str, str], message: str) -> str:
    for name, text in files.items():
        path = repo / "skills" / "security-audit" / name
        path.write_text(text)
    _git(repo, "add", "-A")
    _git(repo, "commit", "-q", "-m", message)
    return _git(repo, "rev-parse", "HEAD")


def _run(target: Path, upstream: Path, *ref: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(SCRIPT), *ref],
        cwd=target,
        env={**os.environ, "SECURITY_AUDIT_SKILL_REPO": str(upstream)},
        capture_output=True,
        text=True,
        timeout=60,
    )


def _target(tmp_path: Path) -> Path:
    root = tmp_path / "app"
    root.mkdir()
    return root


def test_vendors_skill_and_license_and_records_the_commit(tmp_path):
    upstream = _upstream(tmp_path)
    sha = _commit(upstream, {"SKILL.md": "v1\n", "HUNTING.md": "hunt\n"}, "first")
    root = _target(tmp_path)

    result = _run(root, upstream)

    assert result.returncode == 0, result.stderr
    dest = root / DEST
    assert (dest / "SKILL.md").read_text() == "v1\n"
    assert (dest / "LICENSE").read_text() == "MIT License\n"
    assert not (dest / "README.md").exists()
    assert f"commit: {sha}" in (dest / "SOURCE").read_text()


def test_pins_to_the_requested_ref_and_drops_files_removed_upstream(tmp_path):
    upstream = _upstream(tmp_path)
    first = _commit(upstream, {"SKILL.md": "v1\n", "OLD.md": "old\n"}, "first")
    (upstream / "skills" / "security-audit" / "OLD.md").unlink()
    second = _commit(upstream, {"SKILL.md": "v2\n"}, "second")
    root = _target(tmp_path)

    assert _run(root, upstream, first).returncode == 0
    assert (root / DEST / "OLD.md").exists()
    result = _run(root, upstream, second)

    assert result.returncode == 0, result.stderr
    assert (root / DEST / "SKILL.md").read_text() == "v2\n"
    assert not (root / DEST / "OLD.md").exists()
    assert f"commit: {second}" in (root / DEST / "SOURCE").read_text()


def test_lists_upstream_commits_since_the_previous_pin(tmp_path):
    upstream = _upstream(tmp_path)
    first = _commit(upstream, {"SKILL.md": "v1\n"}, "first")
    _commit(upstream, {"SKILL.md": "v2\n"}, "tighten the candidate gate")
    root = _target(tmp_path)
    assert _run(root, upstream, first).returncode == 0

    result = _run(root, upstream)

    assert result.returncode == 0, result.stderr
    assert "tighten the candidate gate" in result.stdout


def test_refuses_a_symlink_and_keeps_the_current_copy(tmp_path):
    upstream = _upstream(tmp_path)
    first = _commit(upstream, {"SKILL.md": "v1\n"}, "first")
    root = _target(tmp_path)
    assert _run(root, upstream, first).returncode == 0
    (upstream / "skills" / "security-audit" / "link.md").symlink_to("../../README.md")
    _commit(upstream, {"SKILL.md": "v2\n"}, "adds a symlink")

    result = _run(root, upstream)

    assert result.returncode != 0
    assert "symlink" in result.stderr
    assert (root / DEST / "SKILL.md").read_text() == "v1\n"
    assert f"commit: {first}" in (root / DEST / "SOURCE").read_text()
