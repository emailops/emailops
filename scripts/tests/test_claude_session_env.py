"""scripts/claude_session_env.sh: the SessionStart hook that puts the .nvmrc Node on PATH.

Claude Code sources $CLAUDE_ENV_FILE before every Bash command of the session, so
one export line there replaces the nvm path agents used to prepend by hand.
"""

import os
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "claude_session_env.sh"
NODE_MAJOR = (REPO / ".nvmrc").read_text().strip().lstrip("v").split(".")[0]


def _fake_node(home: Path, version: str) -> Path:
    bin_dir = home / ".nvm" / "versions" / "node" / f"v{version}" / "bin"
    bin_dir.mkdir(parents=True)
    node = bin_dir / "node"
    node.write_text(f"#!/bin/sh\necho v{version}\n")
    node.chmod(0o755)
    return bin_dir


def _env(home: Path, env_file: Path | None) -> dict:
    env = {**os.environ, "HOME": str(home), "PATH": "/usr/bin:/bin"}
    env.pop("CLAUDE_ENV_FILE", None)
    if env_file is not None:
        env["CLAUDE_ENV_FILE"] = str(env_file)
    return env


def _run(home: Path, env_file: Path | None) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(SCRIPT)], env=_env(home, env_file), capture_output=True, text=True, timeout=30
    )


def test_appends_newest_matching_nvm_node_to_env_file(tmp_path):
    home = tmp_path / "home"
    _fake_node(home, f"{NODE_MAJOR}.1.0")
    newest = _fake_node(home, f"{NODE_MAJOR}.10.0")
    env_file = tmp_path / "env.sh"
    env_file.write_text("export FROM_ANOTHER_HOOK=1\n")

    result = _run(home, env_file)

    assert result.returncode == 0, result.stderr
    assert env_file.read_text() == f'export FROM_ANOTHER_HOOK=1\nexport PATH="{newest}:$PATH"\n'


def test_bash_commands_then_find_that_node_first(tmp_path):
    home = tmp_path / "home"
    bin_dir = _fake_node(home, f"{NODE_MAJOR}.1.0")
    env_file = tmp_path / "env.sh"
    _run(home, env_file)

    found = subprocess.run(
        ["bash", "-c", 'source "$CLAUDE_ENV_FILE"; command -v node'],
        env=_env(home, env_file),
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert found.stdout.strip() == str(bin_dir / "node")


def test_no_matching_install_leaves_env_file_alone(tmp_path):
    home = tmp_path / "home"
    _fake_node(home, f"{int(NODE_MAJOR) - 2}.0.0")
    env_file = tmp_path / "env.sh"

    result = _run(home, env_file)

    assert result.returncode == 0
    assert not env_file.exists()


def test_without_env_file_does_nothing(tmp_path):
    home = tmp_path / "home"
    _fake_node(home, f"{NODE_MAJOR}.1.0")

    result = _run(home, None)

    assert result.returncode == 0
    assert result.stdout == ""
