"""generate_cask.sh under the bash `make cask` runs: /bin/bash, 3.2 on macOS."""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parent.parent / "generate_cask.sh"
SHA = "ab" * 32

pytestmark = pytest.mark.skipif(sys.platform != "darwin", reason="the script is macOS-only")


def _fake_curl(bin_dir: Path, release: dict) -> None:
    """A curl on PATH that prints the release JSON the GitHub API would return."""
    payload = bin_dir / "release.json"
    payload.write_text(json.dumps(release))
    curl = bin_dir / "curl"
    curl.write_text(f'#!/bin/sh\ncat "{payload}"\n')
    curl.chmod(0o755)


def _run(tmp_path: Path, release: dict) -> tuple[subprocess.CompletedProcess, Path]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _fake_curl(bin_dir, release)
    out = tmp_path / "emailops.rb"
    env = {**os.environ, "PATH": f"{bin_dir}:/usr/bin:/bin", "CASK_OUT": str(out)}
    result = subprocess.run(["/bin/bash", str(SCRIPT), "v9.9.9"], capture_output=True, text=True, env=env)
    return result, out


def test_writes_the_version_and_dmg_digest_of_the_release(tmp_path):
    release = {
        "tag_name": "v9.9.9",
        "assets": [
            {"name": "EmailOps-macos.dmg", "digest": f"sha256:{SHA}"},
            {"name": "EmailOps-CLI-macos.dmg", "digest": "sha256:" + "cd" * 32},
        ],
    }
    result, out = _run(tmp_path, release)
    assert result.returncode == 0, result.stderr
    cask = out.read_text()
    assert 'version "9.9.9"' in cask
    assert f'sha256 "{SHA}"' in cask


def test_refuses_a_release_without_the_dmg_digest(tmp_path):
    result, out = _run(tmp_path, {"tag_name": "v9.9.9", "assets": [{"name": "EmailOps-macos.dmg"}]})
    assert result.returncode == 1
    assert "EmailOps-macos.dmg not found" in result.stderr
    assert not out.exists()
