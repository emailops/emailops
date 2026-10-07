"""PHASE guards in scripts/build_platform.sh.

The compile/bundle split exists only for the signed Windows release build; a
wrong combination must stop before any download or compile starts.
"""

import os
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "build_platform.sh"


def _run(platform: str, **env: str) -> subprocess.CompletedProcess:
    full_env = {k: v for k, v in os.environ.items() if k not in {"PHASE", "SIGNED_BIN_DIR", "DYNAMIC_BACKENDS"}}
    full_env.update(env)
    return subprocess.run(
        ["bash", str(SCRIPT), platform],
        env=full_env,
        capture_output=True,
        text=True,
        timeout=30,
    )


@pytest.mark.parametrize("phase", ["compile", "bundle"])
def test_split_phases_are_windows_only(phase):
    result = _run("linux", PHASE=phase, DYNAMIC_BACKENDS="1")
    assert result.returncode == 2
    assert "only used for the signed Windows build" in result.stderr


@pytest.mark.parametrize("phase", ["compile", "bundle"])
def test_split_phases_need_dynamic_backends(phase):
    result = _run("windows", PHASE=phase)
    assert result.returncode == 2
    assert "needs DYNAMIC_BACKENDS=1" in result.stderr


def test_bundle_needs_the_signed_exe(tmp_path):
    result = _run("windows", PHASE="bundle", DYNAMIC_BACKENDS="1", SIGNED_BIN_DIR=str(tmp_path))
    assert result.returncode == 2
    assert "SIGNED_BIN_DIR" in result.stderr


def test_unknown_phase_is_rejected():
    result = _run("windows", PHASE="sign")
    assert result.returncode == 2
    assert "unknown PHASE" in result.stderr
