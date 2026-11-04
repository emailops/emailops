#!/usr/bin/env python3
"""Block a commit that stages a mail store or a known private string.

Runs from the lefthook pre-commit hook. A deterministic floor under the
privacy-reviewer subagent, not a replacement for it:

- A staged mail store or export (`*.db`, `*.db-wal`, `*.db-shm`, `*.sqlite`,
  `*.eml`, `*.mbox`) or any file under an `.emailops-data*` /
  `.emailops-demo-data*` directory blocks the commit.
- An added line containing an entry of `<git common dir>/info/private-strings`
  blocks the commit. That file is local to this clone (git never versions
  `.git/info`) and shared by its worktrees; list one real address, name or
  other string per line, `#` for comments. The match is case-insensitive and
  the report names the file and line without echoing the string.

Usage: privacy_scan.py   (exit 0 clean, 1 blocked)
"""

import re
import subprocess
import sys
from pathlib import Path

MAIL_STORE_SUFFIXES = (".db", ".db-wal", ".db-shm", ".sqlite", ".sqlite3", ".eml", ".mbox")
APP_DATA_DIR = re.compile(r"(^|/)\.emailops-(demo-)?data[^/]*/")
HUNK = re.compile(r"^@@ -\S+ \+(\d+)(?:,\d+)? @@")


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def staged_paths() -> list[str]:
    out = git("diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z")
    return [p for p in out.split("\0") if p]


def added_lines():
    """Yield (path, line number, text) for every line the staged diff adds."""
    path, lineno = None, 0
    diff = git("diff", "--cached", "-U0", "--no-color", "--no-ext-diff", "--diff-filter=ACMR")
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif hunk := HUNK.match(line):
            lineno = int(hunk.group(1))
        elif line.startswith("+") and path is not None:
            yield path, lineno, line[1:]
            lineno += 1


def private_strings_file() -> Path:
    return Path(git("rev-parse", "--git-common-dir").strip()) / "info" / "private-strings"


def private_strings(path: Path) -> list[str]:
    if not path.is_file():
        return []
    entries = (line.strip().lower() for line in path.read_text().splitlines())
    return [e for e in entries if e and not e.startswith("#")]


def main() -> int:
    problems = []
    for path in staged_paths():
        if path.lower().endswith(MAIL_STORE_SUFFIXES) or APP_DATA_DIR.search(path):
            problems.append(f"{path}: mail store or app data file")

    denylist = private_strings_file()
    needles = private_strings(denylist)
    if needles:
        for path, lineno, text in added_lines():
            lowered = text.lower()
            if any(needle in lowered for needle in needles):
                problems.append(f"{path}:{lineno}: matches an entry of {denylist}")

    if not problems:
        return 0
    print("privacy-scan: commit blocked")
    for problem in problems:
        print(f"  {problem}")
    print("Unstage the file, or replace the private string with a synthetic equivalent.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
