#!/usr/bin/env python3
"""Keep doc claims and the tests that prove them tied together.

A claim is a promise the published docs make about the UI — "the toolbar
carries a Hide junk messages switch". The paragraph making it carries a
`<!-- claim:id -->` marker; a `docClaim('id', …)` case in sweep.mjs drives the
real app and checks it.

That pairing is only worth anything while both ends exist, and each end rots in
its own way: a paragraph gets rewritten and the marker is dropped, leaving a
test guarding prose nobody publishes any more; or a case is deleted and the
marker stays, promising a check that no longer runs. Neither shows up as a
failure anywhere else, because both halves still parse.

So: every marker needs a case, every case needs a marker, and — extending the
four-language rule — a marker present in one language must be present in all,
or three quarters of readers get a claim nothing verifies.

Usage: uv run --no-project scripts/check-docs-claims.py   (exit 0 = every claim is paired)
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SITE = ROOT / "docs/site"
LANGS = ("en", "es", "fr", "de")
SWEEP = ROOT / ".claude/skills/verify-emailops/scripts/sweep.mjs"

MARKER = re.compile(r"<!--\s*claim:([a-z0-9-]+)\s*-->")
CASE = re.compile(r"docClaim\(\s*['\"]([a-z0-9-]+)['\"]")


def markers_by_lang() -> dict[str, dict[str, str]]:
    """{lang: {claim id: page}} for every marker in the published docs."""
    found: dict[str, dict[str, str]] = {}
    for lang in LANGS:
        found[lang] = {}
        for page in sorted((SITE / lang).glob("*.md")):
            if page.name == "README.md":
                continue
            for claim in MARKER.findall(page.read_text(encoding="utf-8")):
                found[lang][claim] = page.name
    return found


def main() -> int:
    problems = []
    found = markers_by_lang()
    cases = set(CASE.findall(SWEEP.read_text(encoding="utf-8")))

    reference = found["en"]
    for lang in LANGS[1:]:
        for claim in sorted(set(reference) - set(found[lang])):
            problems.append(f"{lang}: no <!-- claim:{claim} --> (en marks it in {reference[claim]})")
        for claim in sorted(set(found[lang]) - set(reference)):
            problems.append(f"{lang}: marks claim:{claim}, which en does not")
        for claim in sorted(set(reference) & set(found[lang])):
            if found[lang][claim] != reference[claim]:
                problems.append(
                    f"{lang}: claim:{claim} sits in {found[lang][claim]}, "
                    f"en has it in {reference[claim]}"
                )

    for claim in sorted(set(reference) - cases):
        problems.append(f"claim:{claim} is marked in {reference[claim]} but no docClaim() proves it")
    for claim in sorted(cases - set(reference)):
        problems.append(f"docClaim('{claim}') has no <!-- claim:{claim} --> marker in the docs")

    if problems:
        print("\ndocs claim check FAILED:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        print(
            "\nAdd the missing marker or case, or drop both — a claim guarded "
            "at only one end proves nothing.",
            file=sys.stderr,
        )
        return 1

    print(f"docs claim check OK — {len(reference)} claims paired across {len(LANGS)} languages")
    return 0


if __name__ == "__main__":
    sys.exit(main())
