"""scripts/privacy_scan.py: the pre-commit block on mail stores and private strings."""

import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "privacy_scan.py"


def _git(repo: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True)


@pytest.fixture
def repo(tmp_path):
    root = tmp_path / "repo"
    root.mkdir()
    _git(root, "init", "-q")
    _git(root, "config", "user.email", "dev@example.com")
    _git(root, "config", "user.name", "Dev")
    (root / "README.md").write_text("old line\n")
    _git(root, "add", "README.md")
    _git(root, "commit", "-q", "-m", "init")
    return root


def _stage(repo: Path, path: str, content: str) -> None:
    target = repo / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content)
    _git(repo, "add", "-f", path)


def _private_strings(repo: Path, content: str) -> None:
    (repo / ".git" / "info").mkdir(exist_ok=True)
    (repo / ".git" / "info" / "private-strings").write_text(content)


def _scan(repo: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["python3", "-I", str(SCRIPT)], cwd=repo, capture_output=True, text=True, timeout=30
    )


@pytest.mark.parametrize(
    "path",
    [
        "fixtures/mail.db",
        "fixtures/mail.db-wal",
        "fixtures/mail.db-shm",
        "fixtures/store.sqlite",
        "fixtures/message.eml",
        "export/inbox.mbox",
        ".emailops-data/notes.txt",
        "sub/.emailops-demo-data/emailops.db.stamp",
    ],
)
def test_mail_store_or_app_data_file_blocks(repo, path):
    _stage(repo, path, "x\n")

    result = _scan(repo)

    assert result.returncode == 1
    assert path in result.stdout


def test_added_private_string_blocks_without_echoing_it(repo):
    _private_strings(repo, "Real.Person@private.example\n")
    _stage(repo, "notes.md", "intro\ncontact real.person@PRIVATE.example today\n")

    result = _scan(repo)

    assert result.returncode == 1
    assert "notes.md:2" in result.stdout
    assert "private.example" not in result.stdout.lower()


def test_removed_private_string_passes(repo):
    _private_strings(repo, "old line\n")
    _stage(repo, "README.md", "new line\n")

    assert _scan(repo).returncode == 0


def test_blank_lines_and_comments_in_private_strings_match_nothing(repo):
    _private_strings(repo, "# my addresses\n\n   \n")
    _stage(repo, "notes.md", "# heading\nplain text\n")

    assert _scan(repo).returncode == 0


def test_synthetic_change_passes(repo):
    _private_strings(repo, "real.person@private.example\n")
    _stage(repo, "evals/case.json", '{"from": "ulises@emailopslabs.dev"}\n')

    assert _scan(repo).returncode == 0


def test_without_private_strings_file_only_file_checks_run(repo):
    _stage(repo, "notes.md", "anything at all\n")

    assert _scan(repo).returncode == 0
