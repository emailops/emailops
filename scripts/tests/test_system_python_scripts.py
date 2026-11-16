"""Scripts the Makefile and scripts/*.sh run as a bare `python3`.

On macOS that is /usr/bin/python3, Python 3.9: a script there must not use
syntax newer than 3.9, or the target fails before it starts (report_all.py
did, at the end of every `make verify`).
"""

import re
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SYSTEM_PYTHON = Path("/usr/bin/python3")
# Shell variables these callers put in front of a script path.
PREFIXES = {
    "$skill/": ".claude/skills/verify-emailops/scripts/",
    "$REPO_ROOT/": "",
}
CALL = re.compile(r'(?<![\w-])python3 "?([^\s"]+\.py)')


def _scripts_run_by_bare_python3():
    callers = [REPO / "Makefile", *sorted((REPO / "scripts").glob("*.sh"))]
    found = set()
    for caller in callers:
        for raw in CALL.findall(caller.read_text()):
            path = raw
            for var, repl in PREFIXES.items():
                path = path.replace(var, repl)
            found.add(path)
    return sorted(found)


def test_the_callers_are_found():
    scripts = _scripts_run_by_bare_python3()
    assert "scripts/disk_clean.py" in scripts
    assert ".claude/skills/verify-emailops/scripts/report_all.py" in scripts


@pytest.mark.skipif(not SYSTEM_PYTHON.exists(), reason="no /usr/bin/python3 on this machine")
@pytest.mark.parametrize("script", _scripts_run_by_bare_python3())
def test_compiles_on_the_system_python(script):
    path = REPO / script
    assert path.exists(), f"{script} is called but does not exist"
    result = subprocess.run(
        [str(SYSTEM_PYTHON), "-c", "import ast, sys; ast.parse(open(sys.argv[1]).read())", str(path)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
