#!/usr/bin/env bash
#
# Guard the four-language rule for docs/site/.
#
# Published docs must exist in every site language with matching filenames,
# matching sidebar weights, and matching heading anchor ids — otherwise the
# site nav offers a page that 404s, sorts the sidebar differently per language,
# or breaks a cross-page link in one language only.
#
# Usage: scripts/check-docs-parity.sh   (exit 0 = parity holds)

set -euo pipefail

cd "$(dirname "$0")/.."
SITE_DIR="docs/site"
LANGS=(en es fr de)
REF_LANG="en"

python3 - "$SITE_DIR" "$REF_LANG" "${LANGS[@]}" <<'PY'
import pathlib, re, sys

site = pathlib.Path(sys.argv[1])
ref_lang = sys.argv[2]
langs = sys.argv[3:]
problems = []

def pages(lang):
    d = site / lang
    if not d.is_dir():
        problems.append(f"missing language directory: {d}")
        return {}
    return {p.name: p for p in d.glob("*.md") if p.name != "README.md"}

ref = pages(ref_lang)
if not ref:
    print(f"no reference pages in {site/ref_lang}", file=sys.stderr)
    sys.exit(1)

def front_matter(p):
    text = p.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        problems.append(f"{p}: no front matter")
        return {}
    fm = text.split("---", 2)[1]
    out = {}
    for line in fm.splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            out[k.strip()] = v.strip().strip("'\"")
    return out

def anchors(p):
    return set(re.findall(r"\{#([a-z0-9-]+)\}", p.read_text(encoding="utf-8")))

for lang in langs:
    if lang == ref_lang:
        continue
    got = pages(lang)
    for missing in sorted(set(ref) - set(got)):
        problems.append(f"{lang}: missing page {missing} (exists in {ref_lang})")
    for extra in sorted(set(got) - set(ref)):
        problems.append(f"{lang}: extra page {extra} (not in {ref_lang})")

    for name in sorted(set(ref) & set(got)):
        rfm, gfm = front_matter(ref[name]), front_matter(got[name])
        for key in ("title", "description"):
            if not gfm.get(key):
                problems.append(f"{lang}/{name}: front matter missing '{key}'")
        if rfm.get("weight") != gfm.get("weight"):
            problems.append(
                f"{lang}/{name}: weight {gfm.get('weight')!r} != {ref_lang} {rfm.get('weight')!r}"
            )
        if gfm.get("title") and gfm.get("title") == rfm.get("title") and name != "_index.md":
            # Not fatal — some titles are proper nouns — but usually means untranslated.
            print(f"note: {lang}/{name} title is identical to {ref_lang} ({gfm['title']!r})")

        ra, ga = anchors(ref[name]), anchors(got[name])
        for missing in sorted(ra - ga):
            problems.append(f"{lang}/{name}: missing anchor {{#{missing}}}")
        for extra in sorted(ga - ra):
            problems.append(f"{lang}/{name}: anchor {{#{extra}}} not present in {ref_lang}")

if problems:
    print("\ndocs parity FAILED:", file=sys.stderr)
    for p in problems:
        print(f"  - {p}", file=sys.stderr)
    sys.exit(1)

print(f"docs parity OK — {len(ref)} pages x {len(langs)} languages")
PY
