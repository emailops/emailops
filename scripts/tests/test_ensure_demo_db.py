"""scripts/ensure_demo_db.sh: rebuild the demo DB only when its inputs changed.

A fresh worktree checks every file out with a new mtime. Deciding by mtime made
the generator always look newer than the demo DB, so a run from a new worktree
rebuilt (and could wipe) a demo DB shared with the main checkout.
"""

import os
import stat
import subprocess
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "ensure_demo_db.sh"

# Stands in for `make`: logs the target and, for the DB target, writes a demo DB
# at schema $SCHEMA_VERSION with one embedding so the embed step is skipped.
FAKE_MAKE = """#!/bin/sh
echo "$1" >> "$MAKE_LOG"
case "$1" in
  demo-db)
    rm -f "$DEMO_DB"
    sqlite3 "$DEMO_DB" "CREATE TABLE refinery_schema_history(version INTEGER);
      INSERT INTO refinery_schema_history VALUES ($SCHEMA_VERSION);
      CREATE TABLE embedding_chunks(id INTEGER); INSERT INTO embedding_chunks VALUES (1);"
    ;;
esac
"""


def _repo(tmp_path: Path) -> Path:
    repo = tmp_path / "repo"
    (repo / "scripts").mkdir(parents=True)
    (repo / "src-tauri" / "migrations").mkdir(parents=True)
    (repo / "scripts" / "generate_demo_db.py").write_text("# generator v1\n")
    (repo / "src-tauri" / "migrations" / "V001__init.sql").write_text("CREATE TABLE a(x);\n")
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    for name, body in {"make": FAKE_MAKE, "uv": "#!/bin/sh\nexit 0\n"}.items():
        path = bin_dir / name
        path.write_text(body)
        path.chmod(path.stat().st_mode | stat.S_IEXEC)
    return repo


def _run(tmp_path: Path, schema_version: int = 1) -> list[str]:
    """Run the script and return the make targets it invoked."""
    repo = tmp_path / "repo"
    demo_dir = tmp_path / "demo"
    demo_dir.mkdir(exist_ok=True)
    log = tmp_path / "make.log"
    log.write_text("")
    env = {
        **os.environ,
        "PATH": f"{tmp_path / 'bin'}:{os.environ['PATH']}",
        "MAKE_LOG": str(log),
        "DEMO_DB": str(demo_dir / "emailops.db"),
        "SCHEMA_VERSION": str(schema_version),
    }
    result = subprocess.run(
        ["bash", str(SCRIPT), str(demo_dir), "demo-db", "demo-embed"],
        cwd=repo,
        env=env,
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, result.stderr
    return log.read_text().split()


def _make_newer(path: Path) -> None:
    future = path.stat().st_mtime + 3600
    os.utime(path, (future, future))


def test_fresh_checkout_with_unchanged_inputs_keeps_db(tmp_path):
    repo = _repo(tmp_path)
    assert _run(tmp_path) == ["demo-db"]
    _make_newer(repo / "scripts" / "generate_demo_db.py")
    _make_newer(repo / "src-tauri" / "migrations" / "V001__init.sql")

    assert _run(tmp_path) == []


def test_changed_generator_rebuilds(tmp_path):
    repo = _repo(tmp_path)
    _run(tmp_path)
    (repo / "scripts" / "generate_demo_db.py").write_text("# generator v2\n")

    assert _run(tmp_path) == ["demo-db"]


def test_new_migration_rebuilds(tmp_path):
    repo = _repo(tmp_path)
    _run(tmp_path)
    (repo / "src-tauri" / "migrations" / "V002__more.sql").write_text("CREATE TABLE b(y);\n")

    assert _run(tmp_path, schema_version=2) == ["demo-db"]


def test_db_without_stamp_at_current_schema_is_kept(tmp_path):
    _repo(tmp_path)
    _run(tmp_path)
    (tmp_path / "demo" / "emailops.db.stamp").unlink()

    assert _run(tmp_path) == []
