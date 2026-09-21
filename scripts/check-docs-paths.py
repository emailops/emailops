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
    ".claude/skills/release/SKILL.md:scripts/sync-docs.sh": "lives in the getemailops.com repo",
    "homebrew/README.md:../homebrew-tap/Casks/emailops.rb": "lives in the emailops/homebrew-tap repo",
    "tools/kv_viz/README.md:src-tauri/reports/bench/kv_xconv_*.json": "generated at run time into gitignored reports/",
    # A worked example of adding a draft-review feature. The files are
    # deliberately fictional; the skill teaches the shape, not these paths.
    ".claude/skills/build-ai-feature/SKILL.md:src-tauri/src/commands/review.rs": "illustrative example",
    ".claude/skills/build-ai-feature/SKILL.md:src-tauri/src/evals/draft_review.rs": "illustrative example",
    ".claude/skills/build-ai-feature/SKILL.md:src-tauri/src/services/emails/review.rs": "illustrative example",
    ".claude/skills/build-ai-feature/SKILL.md:src-tauri/examples/draft_review_eval.rs": "illustrative example",
    ".claude/skills/build-ai-feature/SKILL.md:src/components/Settings/AiReviewSettings.tsx": "illustrative example",
    ".claude/skills/build-ai-feature/SKILL.md:private-evals/draft_review/cases.yaml": "illustrative example",
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
    if (ROOT / candidate).exists():
        return True
    if (md_dir / candidate).exists():
        return True
    return any(p.endswith(f"/{candidate}") for p in tracked)


def main() -> int:
    problems = []
    checked = 0
    tracked = git_ls()

    for md in tracked_markdown():
        rel_md = md.relative_to(ROOT).as_posix()
        for span in quoted_spans(md.read_text(encoding="utf-8", errors="replace")):
            candidate = span.strip()
            if PLACEHOLDER.search(candidate) or "://" in candidate:
                continue
            if not CANDIDATE.match(candidate):
                continue
            if f"{rel_md}:{candidate}" in ALLOWED_UNRESOLVED:
                continue
            checked += 1
            if not resolves(candidate, md.parent, tracked):
                problems.append(f"{rel_md}: `{candidate}` does not exist")

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
