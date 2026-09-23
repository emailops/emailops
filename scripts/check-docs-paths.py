#!/usr/bin/env python3
"""Verify that repo file paths quoted in Markdown actually exist.

Docs point people (and agents) at files. When a file is renamed or moved and the
prose is not, the reference silently rots: `SECURITY.md` sent vulnerability
reporters to a sanitiser module that had been split in two, and three MODULE.md
files pointed at a `db/schema.rs` that refinery migrations replaced. Nothing
caught either, because prose is not compiled.

Only backticked spans that *look like a source file* are considered — they
contain a `/` and end in a known code/doc extension. Each candidate is resolved
both against the repo root and against the directory of the Markdown file
quoting it; existing at either is enough. Globs are satisfied by any match.

Usage: uv run --no-project scripts/check-docs-paths.py   (exit 0 = every path resolves)
"""

import fnmatch
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Paths that cannot resolve here and are still correct. Keyed by
# "<md path>:<quoted path>" so an exception never silences the same string in
# another file, with the reason recorded — an unexplained entry is how a check
# like this quietly stops meaning anything.
ALLOWED_UNRESOLVED = {
    "docs/site/README.md:scripts/sync-docs.sh": "lives in the getemailops.com repo",
    "homebrew/README.md:../homebrew-tap/Casks/emailops.rb": "lives in the emailops/homebrew-tap repo",
    # A worked example of adding a draft-review feature. The files are
    # deliberately fictional; the skill teaches the shape, not these paths.
    ".claude/skills/build-ai-feature/SKILL.md:src-tauri/src/commands/review.rs": "illustrative example",
    ".claude/skills/build-ai-feature/SKILL.md:src-tauri/src/evals/draft_review.rs": "illustrative example",
    ".claude/skills/build-ai-feature/SKILL.md:src-tauri/src/services/emails/review.rs": "illustrative example",
    ".claude/skills/build-ai-feature/SKILL.md:src-tauri/examples/draft_review_eval.rs": "illustrative example",
    ".claude/skills/build-ai-feature/SKILL.md:src/components/Settings/AiReviewSettings.tsx": "illustrative example",
}

# Source and config files only. Binary artefacts (png, icns, gguf) are
# generated into gitignored report dirs, so "does not exist" says nothing.
EXTENSIONS = "rs|ts|tsx|js|mjs|cjs|py|sh|sql|json|toml|md|yml|yaml|rb|html|css|lock|plist"
# A source-looking path: no whitespace, at least one directory separator, and a
# known extension.
CANDIDATE = re.compile(rf"^(?:[\w.@-]+/)+[\w.@*-]+\.(?:{EXTENSIONS})$")
# Placeholders (`docs/site/<lang>/x.md`, `${VAR}`) are templates, not paths.
PLACEHOLDER = re.compile(r"[<>{}$]")


def git_ls(*patterns: str) -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", *patterns],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [line for line in out.splitlines() if line]


def tracked_markdown() -> list[pathlib.Path]:
    return [ROOT / p for p in git_ls("*.md")]


def quoted_spans(text: str) -> list[str]:
    # Strip fenced code blocks: they hold commands and sample output, not
    # references the reader is meant to follow.
    without_fences = re.sub(r"```.*?```", "", text, flags=re.S)
    return re.findall(r"`([^`\n]+)`", without_fences)


def resolves(candidate: str, md_dir: pathlib.Path, tracked: list[str]) -> bool:
    """Accept a path that resolves any way a reader would plausibly read it.

    Three readings, cheapest first: anchored at the repo root; relative to the
    Markdown file's own directory (`../DECISIONS.md`, and a skill quoting its
    own `scripts/verify.sh`); or shorthand relative to some source root, which
    is satisfied by any tracked file ending in that suffix at a segment
    boundary (`services/emails/sync.rs` inside a MODULE.md).

    The suffix rule is deliberately generous. Resolving shorthand against a
    fixed list of base directories instead would mean growing that list on
    every miss until the check no longer says anything, and a path that exists
    under an unexpected prefix is a far smaller problem than one naming a file
    that is simply gone — which is what this catches and what it must keep
    catching without crying wolf.
    """
    if "*" in candidate:
        return any(
            fnmatch.fnmatch(p, candidate) or fnmatch.fnmatch(p, f"*/{candidate}")
            for p in tracked
        )
    for base in (ROOT, md_dir):
        path = base / candidate
        # A path escaping the repo only exists on machines with a sibling
        # checkout; CI never has one, so it must not count as resolving.
        if path.exists() and path.resolve().is_relative_to(ROOT.resolve()):
            return True
    return any(p.endswith(f"/{candidate}") for p in tracked)


def _check_ignore(paths: list[str]) -> list[str]:
    """`git check-ignore` over a batch. Git aborts the whole batch (exit 128)
    on one bad pathspec — a path through a symlink — so a failed batch is
    retried one path at a time rather than silently answering "none ignored"."""
    def ask(batch):
        return subprocess.run(
            ["git", "-C", str(ROOT), "check-ignore", "--no-index", "--stdin"],
            input="\n".join(batch), capture_output=True, text=True,
        )
    r = ask(paths)
    if r.returncode in (0, 1):
        return r.stdout.splitlines()
    return [p for p in paths if ask([p]).returncode == 0]


def ignored_by_git(candidates: set[str]) -> set[str]:
    """Paths under gitignored locations are generated artefacts (reports,
    build output). Whether one exists depends on what last ran on this
    machine, so checking them would make this guard pass on one checkout and
    fail on the next. They are skipped, not resolved.

    Asked level by level (`a/`, then `a/b/`, …) in one batch per depth, and a
    path drops out as soon as a prefix is ignored. That matters twice: git
    aborts a whole batch on a pathspec "beyond a symbolic link" (reports/verify
    has a `current-full` symlink), and `--no-index` answers for directories a
    fresh clone does not have yet.
    """
    pending = set(candidates)
    ignored: set[str] = set()
    depth = 1
    while pending:
        prefixes = {}
        for c in pending:
            parts = c.split("/")
            if depth < len(parts):
                prefixes.setdefault("/".join(parts[:depth]) + "/", set()).add(c)
            elif depth == len(parts):
                prefixes.setdefault(c, set()).add(c)
        if not prefixes:
            break
        for hit in _check_ignore(sorted(prefixes)):
            ignored |= prefixes.get(hit, set())
        pending -= ignored
        pending = {c for c in pending if len(c.split("/")) > depth}
        depth += 1
    return ignored


def main() -> int:
    problems = []
    checked = 0
    used: set[str] = set()
    tracked = git_ls()

    # First pass: collect candidates, so gitignored ones can be asked about at once.
    found = []
    for md in tracked_markdown():
        rel_md = md.relative_to(ROOT).as_posix()
        for span in quoted_spans(md.read_text(encoding="utf-8", errors="replace")):
            candidate = span.strip()
            if PLACEHOLDER.search(candidate) or "://" in candidate:
                continue
            if CANDIDATE.match(candidate):
                found.append((md, rel_md, candidate))
    # `../x.md` is relative to the quoting file and outside git's pathspec
    # rules; it is never a generated artefact, so it is not asked about.
    generated = ignored_by_git({c for _, _, c in found if not c.startswith('.')})

    for md, rel_md, candidate in found:
        if candidate in generated:
            continue
        checked += 1
        if resolves(candidate, md.parent, tracked):
            continue
        key = f"{rel_md}:{candidate}"
        if key in ALLOWED_UNRESOLVED:
            used.add(key)
            continue
        problems.append(f"{rel_md}: `{candidate}` does not exist")

    # An exception that no longer applies is the same rot one level up: it
    # sits there implying a path is checked-and-excused when the reference has
    # simply gone, and the next real one gets added next to it unquestioned.
    for key in sorted(set(ALLOWED_UNRESOLVED) - used):
        problems.append(
            f"stale ALLOWED_UNRESOLVED entry {key!r} ({ALLOWED_UNRESOLVED[key]}) — "
            "that path no longer needs excusing; remove it"
        )

    if problems:
        print("\ndocs path check FAILED:", file=sys.stderr)
        for p in sorted(set(problems)):
            print(f"  - {p}", file=sys.stderr)
        print(
            "\nFix the reference, or add an ALLOWED_UNRESOLVED entry (with a "
            "reason) if the file genuinely lives outside this repo.",
            file=sys.stderr,
        )
        return 1

    print(f"docs path check OK — {checked} quoted paths resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
