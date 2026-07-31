#!/usr/bin/env bash
#
# Verify that UI labels quoted in docs/site/<lang>/ match the app's own
# translations in src/locales/<lang>/.
#
# The docs tell people to click things. If the app's German tab says
# „KI-Klassifikation" and the German docs say „KI-Klassifizierung", the reader
# hunts for a menu item that does not exist. Two checks:
#
#   1. Settings paths — every segment of a `**A → B**` path must be a real
#      string in that language's locale files.
#   2. Control labels — English is the oracle. A `**bold**` span whose text is
#      verbatim an English UI string is a control reference, so the span at the
#      same position in es/fr/de must likewise be a real string in that
#      language. Bold prose (emphasis, filenames, product names) is untouched.
#
# Check 2 relies on bold spans lining up positionally across translations,
# which is a consequence of translating page-for-page. If a translation adds or
# drops a bold span the script says so rather than guessing.
#
# Usage: scripts/check-docs-labels.sh   (exit 0 = every quoted label exists)

set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import json, pathlib, re, sys

LANGS = ["en", "es", "fr", "de"]
REF = "en"
DOCS = pathlib.Path("docs/site")

# Path segments that belong to another program, not to EmailOps.
IGNORE_SEGMENTS = {
    # Windows' own Settings app, quoted in the uninstall instructions.
    "Configuración", "Apps", "Installed apps", "Applications",
    "Applications installées", "Aplicaciones", "Aplicaciones instaladas",
    "Installierte Apps", "EmailOps",
    # KeePassXC's settings, quoted in the Linux keyring section.
    "Secret Service Integration", "Intégration Secret Service",
    "Integración con Secret Service", "Secret-Service-Integration",
}

# English bold spans that happen to collide with a UI string but are ordinary
# prose in context, so translations are not expected to quote a control.
# Keyed by "<file>:<english text>".
NOT_A_CONTROL = {
    "getting-started.md:Embeddings",   # "Embeddings are generated in the background"
}


def locale_strings(lang):
    out = set()
    def walk(o):
        if isinstance(o, dict):
            for v in o.values():
                walk(v)
        elif isinstance(o, str):
            out.add(" ".join(o.split()))
    for f in pathlib.Path("src/locales", lang).glob("*.json"):
        walk(json.load(open(f, encoding="utf-8")))
    return out


def is_ui(text, strings):
    if text in strings:
        return True
    # The UI often appends a unit: "Context window (tokens)".
    return any(s.startswith(text + " (") for s in strings)


def bold_spans(lang, name):
    text = pathlib.Path(DOCS, lang, name).read_text(encoding="utf-8")
    return [" ".join(b.split()) for b in re.findall(r"\*\*([^*]+)\*\*", text)]


STRINGS = {l: locale_strings(l) for l in LANGS}
problems = []

# --- 1. settings paths ------------------------------------------------------
for lang in LANGS:
    checked = 0
    for p in sorted(DOCS.joinpath(lang).glob("*.md")):
        for bold in re.findall(r"\*\*([^*]+→[^*]+)\*\*", p.read_text(encoding="utf-8")):
            for seg in (" ".join(s.split()) for s in bold.split("→")):
                if not seg or seg in IGNORE_SEGMENTS:
                    continue
                checked += 1
                if not is_ui(seg, STRINGS[lang]):
                    problems.append(f"{lang}/{p.name}: path segment {seg!r} is not in src/locales/{lang}/")
    print(f"{lang}: {checked} settings-path segments checked")

# --- 2. control labels, English as the oracle -------------------------------
names = sorted(p.name for p in DOCS.joinpath(REF).glob("*.md"))
controls = 0
for name in names:
    ref_spans = bold_spans(REF, name)
    for lang in LANGS:
        if lang == REF:
            continue
        spans = bold_spans(lang, name)
        if len(spans) != len(ref_spans):
            problems.append(
                f"{lang}/{name}: {len(spans)} bold spans but {REF} has {len(ref_spans)} — "
                f"cannot align control labels; keep the translation span-for-span"
            )
            continue
        for i, ref_text in enumerate(ref_spans):
            if f"{name}:{ref_text}" in NOT_A_CONTROL:
                continue
            if not is_ui(ref_text, STRINGS[REF]):
                continue
            if lang == LANGS[1]:
                controls += 1
            if not is_ui(spans[i], STRINGS[lang]):
                problems.append(
                    f"{lang}/{name}: {spans[i]!r} should be the app's label — "
                    f"{REF} quotes the control {ref_text!r} here"
                )
print(f"{REF}: {controls} control labels enforced across {len(LANGS) - 1} translations")

if problems:
    print("\nlabel check FAILED:", file=sys.stderr)
    for p in problems:
        print(f"  - {p}", file=sys.stderr)
    print("\nFix the docs to quote the app's string, or add a NOT_A_CONTROL entry "
          "if the English is prose.", file=sys.stderr)
    sys.exit(1)

print("\nall quoted UI labels exist in the app's translations")
PY
