"""scripts/bench_preflight.sh: refuse a speed benchmark on a noisy machine.

`pmset` and `pgrep` are faked on PATH, so the tests read the same output the
script parses on a real Mac without depending on this machine's state.
"""

import os
import stat
import subprocess
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "bench_preflight.sh"

ON_AC = "Now drawing from 'AC Power'"
ON_BATTERY = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t80%; discharging"


def _executable(path: Path, body: str) -> None:
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


def _run(tmp_path: Path, batt: str = ON_AC, low_power: str = "0", emailops_pids: str = ""):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (tmp_path / "batt.txt").write_text(batt + "\n")
    (tmp_path / "settings.txt").write_text(f" sleep                1\n lowpowermode         {low_power}\n")
    _executable(
        bin_dir / "pmset",
        f'#!/bin/sh\n[ "$2" = batt ] && cat "{tmp_path}/batt.txt" || cat "{tmp_path}/settings.txt"\n',
    )
    pgrep_body = f'#!/bin/sh\n[ "$1 $2" = "-x emailops" ] && printf "{emailops_pids}" && exit 0\nexit 1\n'
    _executable(bin_dir / "pgrep", pgrep_body if emailops_pids else "#!/bin/sh\nexit 1\n")
    env = dict(os.environ, PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}")
    return subprocess.run(["bash", str(SCRIPT)], capture_output=True, text=True, env=env)


def test_passes_on_ac_power_with_no_app_running(tmp_path):
    proc = _run(tmp_path)

    assert proc.returncode == 0, proc.stderr


def test_refuses_on_battery(tmp_path):
    proc = _run(tmp_path, batt=ON_BATTERY)

    assert proc.returncode == 1
    assert "battery" in proc.stderr


def test_refuses_in_low_power_mode(tmp_path):
    proc = _run(tmp_path, low_power="1")

    assert proc.returncode == 1
    assert "Low Power Mode" in proc.stderr


def test_refuses_while_an_emailops_app_runs(tmp_path):
    proc = _run(tmp_path, emailops_pids="4242\n")

    assert proc.returncode == 1
    assert "4242" in proc.stderr


def _run_bench_on_battery(tmp_path: Path, bench: str, args: list) -> tuple:
    """Run a copy of a bench script on battery; return (process, whether cargo ran)."""
    scripts = tmp_path / "repo" / "scripts"
    scripts.mkdir(parents=True)
    for name in (bench, SCRIPT.name):
        (scripts / name).write_text((SCRIPT.parent / name).read_text())
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _executable(bin_dir / "pmset", f'#!/bin/sh\n[ "$2" = batt ] && echo "{ON_BATTERY}" || true\n')
    _executable(bin_dir / "pgrep", "#!/bin/sh\nexit 1\n")
    cargo_ran = tmp_path / "cargo-ran"
    for tool in ("cargo", "make"):
        _executable(bin_dir / tool, f'#!/bin/sh\ntouch "{cargo_ran}"\n')
    env = dict(os.environ, PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}")
    proc = subprocess.run(["bash", str(scripts / bench), *args], capture_output=True, text=True, env=env)
    return proc, cargo_ran.exists()


def test_cli_bench_stops_before_building_on_battery(tmp_path):
    proc, built = _run_bench_on_battery(tmp_path, "cli_bench.sh", [])

    assert proc.returncode != 0
    assert not built


def test_model_bench_stops_before_building_on_battery(tmp_path):
    proc, built = _run_bench_on_battery(tmp_path, "model_bench.sh", ["--models", "some-model"])

    assert proc.returncode != 0
    assert not built
