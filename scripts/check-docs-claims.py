#!/usr/bin/env python3
"""Guarantee that every claim the published docs make is catalogued and checked.

Every paragraph, list item, table and code block in docs/site/en/*.md is a
claim: it carries a `<!-- claim:id -->` marker and has an entry in
docs/site/claims.toml saying how it is verified. This script is what makes
"the docs are fully verified" true rather than hoped for. It fails when:

- a block has no marker — new prose that nobody catalogued;
- es/fr/de do not carry the same markers on the same blocks — three quarters of
  readers would get a claim nothing checks (extends the four-language rule);
- a marker has no catalogue entry, or an entry no longer has a marker;
- an entry has no checks, or a check the runner does not understand;
- an `app` check has no claim('id', …) case in any app phase, or a case has no `app` entry;
- a check's `covers` quotes text its block no longer contains;
- the generated regions (<!-- generated:name -->) differ between languages.

Each end of every pairing rots on its own, and none of these show up as a
failure anywhere else, because both halves still parse.

Usage: uv run --no-project scripts/check-docs-claims.py   (exit 0 = complete)
"""

import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from docs_claims_lib import LANGS, ROOT, blocks, load_catalog, locate, pages  # noqa: E402

# Where app checks live: the WebDriver phases and the CLI phase. Each case is a
# literal claim('id', …) call — never an id from a loop variable, or this guard
# could not see it.
APP_SOURCES = [
    ROOT / ".claude/skills/verify-emailops/scripts/doc_claims.mjs",
    ROOT / ".claude/skills/verify-emailops/scripts/doc_claims_demo.mjs",
    ROOT / "scripts/docs_cli_claims.py",
]
CASE = re.compile(r"claim\(\s*['\"]([a-z0-9-]+)['\"]")
KINDS = {"file", "tests", "app", "release", "generated", "judge", "manual", "none"}
GENERATED = re.compile(r"^<!--\s*generated:([a-z0-9-]+)\s*-->$", re.M)
FILE_ASSERTIONS = {"quoted", "has", "lacks", "regex"}


def main() -> int:
    problems = []

    # 1. every English block is marked
    en = {}
    for p in pages("en"):
        en[p.name] = [b.claim for b in blocks(p)]
        for b in blocks(p):
            if not b.claim:
                first = " ".join(b.text.split())[:70]
                problems.append(f"en/{p.name}:{b.start}: unmarked {b.kind} — «{first}»")

    # 2. translations carry the same markers on the same blocks
    for lang in LANGS[1:]:
        for p in pages(lang):
            got = [b.claim for b in blocks(p)]
            want = en.get(p.name)
            if want is None:
                continue
            if got != want:
                missing = [c for c in want if c and c not in got]
                extra = [c for c in got if c and c not in want]
                detail = (
                    f"missing {', '.join(missing[:4])}" if missing
                    else f"extra {', '.join(extra[:4])}" if extra
                    else f"{len(got)} blocks vs {len(want)} in en, or in a different order"
                )
                problems.append(f"{lang}/{p.name}: markers differ from en ({detail})")

    # 2b. generated regions are the same in every language
    for p in pages("en"):
        want = GENERATED.findall(p.read_text(encoding="utf-8"))
        for lang in LANGS[1:]:
            other = ROOT / "docs/site" / lang / p.name
            if other.exists() and GENERATED.findall(other.read_text(encoding="utf-8")) != want:
                problems.append(f"{lang}/{p.name}: generated regions differ from en ({', '.join(want) or 'none'})")

    # 3. markers ↔ catalogue
    marked = {c for ids in en.values() for c in ids if c}
    catalog = load_catalog()
    for cid in sorted(marked - set(catalog)):
        problems.append(f"claim:{cid} is marked but has no entry in docs/site/claims.toml")
    for cid in sorted(set(catalog) - marked):
        problems.append(f"claims.toml entry {cid} has no marker in the docs")

    # 4. entries are well formed
    by_id = {b.claim: b for p in pages("en") for b in blocks(p) if b.claim}
    app_ids = set()
    for cid, entry in catalog.items():
        checks = entry.get("checks") or []
        if not checks:
            problems.append(f"claims.toml {cid}: no checks")
        for ch in checks:
            kinds = KINDS & set(ch)
            if len(kinds) != 1:
                problems.append(f"claims.toml {cid}: check {ch!r} must have exactly one of {sorted(KINDS)}")
                continue
            k = kinds.pop()
            if k == "file" and not (FILE_ASSERTIONS & set(ch)):
                problems.append(f"claims.toml {cid}: file check needs one of {sorted(FILE_ASSERTIONS)}")
            if k == "app":
                app_ids.add(cid)
            for c in ch.get("covers", []):
                if cid in by_id and not locate(by_id[cid], c):
                    problems.append(f"claims.toml {cid}: covers «{c}», which the block no longer says")

    # 5. app checks ↔ docClaim() cases
    cases = {c for src in APP_SOURCES for c in CASE.findall(src.read_text(encoding="utf-8"))}
    for cid in sorted(app_ids - cases):
        problems.append(f"claims.toml {cid} expects an app check but no phase has a claim('{cid}', …) case")
    for cid in sorted(cases - app_ids):
        problems.append(f"claim('{cid}', …) is checked in the app but claims.toml has no `app` check for it")

    if problems:
        print("\ndocs claim check FAILED:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        print(
            "\nMark new prose with <!-- claim:id --> (all four languages) and catalogue it "
            "in docs/site/claims.toml — a claim nobody checks is how the docs drift.",
            file=sys.stderr,
        )
        return 1

    print(f"docs claim check OK — {len(marked)} claims, every block marked in {len(LANGS)} languages, all catalogued")
    return 0


if __name__ == "__main__":
    sys.exit(main())
