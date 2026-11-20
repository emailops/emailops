"""scripts/measure_model_memory.sh, run against a fake emailops-cli.

`make model-memory` exports every Makefile variable, including the eval
targets' `ACCOUNT ?= you@example.com`, so the script must not read `ACCOUNT`.
The CLI's `--json` envelope is pretty-printed, so the ok column has to parse it
rather than grep for a compact `"ok":true`.
"""

import os
import shutil
import stat
import subprocess
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "measure_model_memory.sh"

OK_ENVELOPE = """\
{
  "data": {
    "answer": "A synthetic answer."
  },
  "error": null,
  "ok": true
}
"""

ERROR_ENVELOPE = """\
{
  "data": null,
  "error": {
    "code": "not_found",
    "message": "Not found: account not found in DB"
  },
  "ok": false
}
"""


def _executable(path: Path, body: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


def _run(tmp_path: Path, envelope: str, env_overrides: dict) -> tuple[list, list]:
    """Run a copy of the script in a fake repo; return (cli argv, result row)."""
    repo = tmp_path / "repo"
    (repo / "scripts").mkdir(parents=True)
    shutil.copy(SCRIPT, repo / "scripts" / SCRIPT.name)
    (repo / ".emailops-demo-data").mkdir()
    (repo / ".emailops-demo-data" / "emailops.db").write_text("")
    (tmp_path / "models").mkdir()
    (tmp_path / "envelope.json").write_text(envelope)
    argv_file = tmp_path / "cli-argv.txt"
    _executable(
        repo / "src-tauri" / "target" / "debug" / "emailops-cli",
        f'#!/bin/sh\nprintf "%s\\n" "$@" > "{argv_file}"\ncat "{tmp_path}/envelope.json"\n',
    )
    _executable(tmp_path / "bin" / "cargo", "#!/bin/sh\nexit 0\n")

    env = {k: v for k, v in os.environ.items() if k not in ("ACCOUNT", "EMAILOPS_BENCH_ACCOUNT")}
    env["PATH"] = f"{tmp_path / 'bin'}{os.pathsep}{env['PATH']}"
    env["MODELS_DIR"] = str(tmp_path / "models")
    env.update(env_overrides)

    proc = subprocess.run(
        ["bash", str(repo / "scripts" / SCRIPT.name), "some-model"],
        capture_output=True,
        text=True,
        env=env,
        check=True,
    )
    row = proc.stdout.strip().splitlines()[-1].split("\t")
    return argv_file.read_text().splitlines(), row


def _account(argv: list) -> str:
    return argv[argv.index("--account") + 1]


def test_ignores_the_account_the_makefile_exports(tmp_path):
    argv, _ = _run(tmp_path, OK_ENVELOPE, {"ACCOUNT": "you@example.com"})

    assert _account(argv) == "ulises@emailopslabs.dev"


def test_bench_account_variable_overrides_the_demo_account(tmp_path):
    argv, _ = _run(tmp_path, OK_ENVELOPE, {"EMAILOPS_BENCH_ACCOUNT": "other@example.com"})

    assert _account(argv) == "other@example.com"


def test_pretty_printed_ok_envelope_reports_ok(tmp_path):
    _, row = _run(tmp_path, OK_ENVELOPE, {})

    assert row[0] == "some-model"
    assert row[2] == "yes"


def test_error_envelope_reports_not_ok(tmp_path):
    _, row = _run(tmp_path, ERROR_ENVELOPE, {})

    assert row[2] == "no"
