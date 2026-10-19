"""The `outdated` gate in scripts/gates.sh.

Outdated root dependencies block the push. A crates.io index the scratch
resolver cannot satisfy (every compatible release of a transitive dependency
yanked) is not something this repo can fix, so it warns instead of blocking.
"""

import os
import stat
import subprocess
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "gates.sh"

OUTDATED_TABLE = """\
Name      Project  Compat  Latest  Kind    Platform
----      -------  ------  ------  ----    --------
calamine  0.35.0   ---     0.36.1  Normal  ---
"""

RESOLVE_ERROR = """\
error: failed to select a version for the requirement `zip = "^8.6"`
  version 8.6.0 is yanked
location searched: crates.io index
required by package `calamine v0.36.0`
"""


def _run(tmp_path: Path, output: str, code: int) -> subprocess.CompletedProcess:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (tmp_path / "cargo-output.txt").write_text(output)
    fakes = {
        "cargo": f'#!/bin/sh\ncat "{tmp_path}/cargo-output.txt"\nexit {code}\n',
        "cargo-outdated": "#!/bin/sh\nexit 0\n",
    }
    for name, body in fakes.items():
        path = bin_dir / name
        path.write_text(body)
        path.chmod(path.stat().st_mode | stat.S_IEXEC)
    env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}", "CARGO_TARGET_DIR": str(tmp_path)}
    return subprocess.run(
        ["bash", str(SCRIPT), "outdated", str(tmp_path / "out")],
        env=env,
        capture_output=True,
        text=True,
        timeout=30,
    )


def test_outdated_dependencies_block(tmp_path):
    result = _run(tmp_path, OUTDATED_TABLE, 1)
    assert result.returncode == 1
    assert "FAIL" in result.stdout
    assert "1 outdated root dependencies" in result.stdout


def test_unresolvable_index_warns_without_blocking(tmp_path):
    result = _run(tmp_path, RESOLVE_ERROR, 1)
    assert result.returncode == 0
    assert "warn" in result.stdout
    assert "could not resolve" in result.stdout
    assert "outdated root dependencies" not in result.stdout


def test_up_to_date_passes(tmp_path):
    result = _run(tmp_path, "All dependencies are up to date, yay!\n", 0)
    assert result.returncode == 0
    assert "ok" in result.stdout
