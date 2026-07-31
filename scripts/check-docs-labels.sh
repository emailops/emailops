#!/usr/bin/env bash
#
# Verify that UI labels quoted in docs/site/<lang>/ match the app's own
# translations in src/locales/<lang>/.
#
# The docs tell people to click things ("Settings → AI Search"). If the app's
# German tab says „KI-Klassifikation" and the German docs say
# „KI-Klassifizierung", the reader hunts for a menu item that does not exist.
# This checks every `**A → B**` path in the docs against the real strings.
#
# Usage: scripts/check-docs-labels.sh   (exit 0 = every quoted label exists)

set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import json, pathlib, re, sys

LANGS = ["en", "es", "fr", "de"]

# Paths that belong to the operating system, not to EmailOps.
IGNORE = {
    # Windows' own Settings app, quoted in the uninstall instructions.
    "Configuración", "Apps", "Installed apps", "Applications",
    "Applications installées", "Aplicaciones", "Aplicaciones instaladas",
    "Installierte Apps", "EmailOps",
    # KeePassXC's settings, quoted in the Linux keyring section.
    "Secret Service Integration", "Intégration Secret Service",
    "Integración con Secret Service", "Secret-Service-Integration",
}

def locale_strings(lang):
    out = set()
    def walk(o):
        if isinstance(o, dict):
            for v in o.values():
                walk(v)
        elif isinstance(o, str):
            out.add(o.strip())
    for f in pathlib.Path("src/locales", lang).glob("*.json"):
        walk(json.load(open(f, encoding="utf-8")))
    return out

def matches(segment, strings):
    if segment in strings:
        return True
    # The UI often appends a unit: "Context window (tokens)".
    return any(s.startswith(segment + " (") for s in strings)

problems = 0
for lang in LANGS:
    strings = locale_strings(lang)
    docs = sorted(pathlib.Path("docs/site", lang).glob("*.md"))
    if not docs:
        print(f"{lang}: no docs", file=sys.stderr)
        problems += 1
        continue
    seen = set()
    for p in docs:
        text = p.read_text(encoding="utf-8")
        for bold in re.findall(r"\*\*([^*]+→[^*]+)\*\*", text):
            for seg in (s.strip() for s in bold.split("→")):
                if not seg or seg in IGNORE or seg in seen:
                    continue
                seen.add(seg)
                if not matches(seg, strings):
                    print(f"  {lang}: {seg!r} is not a string in src/locales/{lang}/"
                          f"  (in {p.name})")
                    problems += 1
    print(f"{lang}: checked {len(seen)} distinct UI labels")

if problems:
    print(f"\n{problems} label(s) do not exist in the app — fix the docs or the locale.",
          file=sys.stderr)
    sys.exit(1)
print("\nall quoted UI labels exist in the app's translations")
PY
