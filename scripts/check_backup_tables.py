#!/usr/bin/env python3
"""Check that every table the migrations leave behind is classified for backup.

The local data backup (`src-tauri/src/services/backup/`) copies the tables in
`BACKED_UP` and leaves out the ones in `NOT_BACKED_UP`, each with a reason. A
migration that creates a table nobody classified would leave it out of the
backup without anyone deciding so; one that drops or renames a listed table
would make the backup fail. This replays the migrations in order (CREATE TABLE,
CREATE VIRTUAL TABLE, DROP TABLE, ALTER TABLE … RENAME TO) and compares the
tables that remain with both lists.

It reads SQL text only, so it runs in the pre-commit hook without a build. The
Rust tests in `services/backup/tables.rs` and `local.rs` check the same lists
against the migrated schema and make a real copy.

On failure, run the `backup-tables` agent (`.claude/agents/backup-tables.md`):
it classifies each table and updates the lists.

Usage: uv run --no-project scripts/check_backup_tables.py   (exit 0 = all classified)
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
MIGRATIONS = ROOT / "src-tauri" / "migrations"
TABLES_RS = ROOT / "src-tauri" / "src" / "services" / "backup" / "tables.rs"

# Tables that exist without a migration creating them.
OUTSIDE_MIGRATIONS = {
    # refinery creates it to record which migrations ran.
    "refinery_schema_history",
}

_NAME = r'["`\[]?(\w+)["`\]]?'
_STATEMENT = re.compile(
    r"\bCREATE\s+(?:VIRTUAL\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?" + _NAME
    + r"|\bDROP\s+TABLE\s+(?:IF\s+EXISTS\s+)?" + _NAME
    + r"|\bALTER\s+TABLE\s+" + _NAME + r"\s+RENAME\s+TO\s+" + _NAME,
    re.IGNORECASE,
)
_COMMENTS = re.compile(r"--[^\n]*|/\*.*?\*/", re.DOTALL)
_VERSION = re.compile(r"^V(\d+)__.*\.sql$")


def migration_files(directory):
    """The migration files in the order refinery applies them."""
    numbered = []
    for path in directory.iterdir():
        match = _VERSION.match(path.name)
        if match:
            numbered.append((int(match.group(1)), path))
    return [path for _, path in sorted(numbered)]


def tables_after(sql_texts):
    """Replay the table statements of `sql_texts`, in order, into a set of names."""
    tables = set()
    for text in sql_texts:
        for match in _STATEMENT.finditer(_COMMENTS.sub("", text)):
            created, dropped, renamed_from, renamed_to = match.groups()
            if created:
                tables.add(created)
            elif dropped:
                tables.discard(dropped)
            else:
                tables.discard(renamed_from)
                tables.add(renamed_to)
    return tables


def _block(source, const_name):
    start = source.index(f"pub const {const_name}")
    return source[start : source.index("\n];", start)]


def listed_tables(source):
    """The table names in `BACKED_UP` and in `NOT_BACKED_UP` of tables.rs."""
    backed_up = re.findall(r'\ball\("(\w+)"\)|\btable:\s*"(\w+)"', _block(source, "BACKED_UP"))
    backed_up = [a or b for a, b in backed_up]
    not_backed_up = re.findall(r'\(\s*"(\w+)"\s*,\s*Reason::', _block(source, "NOT_BACKED_UP"))
    return backed_up, not_backed_up


def problems(schema, backed_up, not_backed_up):
    """Human-readable problems with the two lists, empty when they are right."""
    listed = backed_up + not_backed_up
    found = []
    unlisted = sorted(schema - set(listed))
    if unlisted:
        found.append("not classified: " + ", ".join(unlisted))
    gone = sorted(set(listed) - schema)
    if gone:
        found.append("listed but no migration leaves them: " + ", ".join(gone))
    twice = sorted({t for t in listed if listed.count(t) > 1})
    if twice:
        found.append("listed twice: " + ", ".join(twice))
    return found


def main():
    texts = [path.read_text(encoding="utf-8") for path in migration_files(MIGRATIONS)]
    schema = tables_after(texts) | OUTSIDE_MIGRATIONS
    backed_up, not_backed_up = listed_tables(TABLES_RS.read_text(encoding="utf-8"))
    found = problems(schema, backed_up, not_backed_up)
    if not found:
        print(f"backup-tables: {len(backed_up)} backed up, {len(not_backed_up)} left out, all classified")
        return 0
    print(f"backup-tables: {TABLES_RS.relative_to(ROOT)} is out of date with the migrations")
    for problem in found:
        print(f"  - {problem}")
    print("Run the backup-tables agent (.claude/agents/backup-tables.md) to classify them.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
