#!/usr/bin/env python3
"""What changed since the last verified commit, mapped to features and layers.

    verify_changes.py [<since-commit>]

Reads .claude/skills/verify-emailops/verified.json (last verified commit) unless a
commit is given, lists the touched files per feature through features.json, and,
when a *-full run exists, which test layers that feature has today.
"""
import json, pathlib, subprocess, sys, collections

REPO = pathlib.Path(__file__).resolve().parents[4]
SKILL = REPO / ".claude/skills/verify-emailops"
manifest = json.loads((SKILL / "features.json").read_text())
verified = json.loads((SKILL / "verified.json").read_text()) if (SKILL / "verified.json").exists() else {}
since = sys.argv[1] if len(sys.argv) > 1 else verified.get("commit")
if not since:
    print("no verified.json and no commit given"); sys.exit(0)
files = subprocess.run(["git", "diff", "--name-only", f"{since}..HEAD"], cwd=REPO, capture_output=True, text=True).stdout.split()
staged = subprocess.run(["git", "status", "--short"], cwd=REPO, capture_output=True, text=True).stdout.splitlines()
files += [l.split()[-1] for l in staged if l.strip()]

def rust_module(path):
    return path.replace("src-tauri/src/", "").replace(".rs", "").replace("/mod", "").replace("/", "::")
def feature_of(path):
    if path.startswith("src-tauri/src/"):
        mod = rust_module(path)
        for f in manifest["features"]:
            if any(mod == pref or mod.startswith(pref + "::") or pref.startswith(mod + "::") for pref in f.get("rust", [])): return f["name"]
    if path.startswith("src/"):
        for f in manifest["features"]:
            if any(path.startswith(pref) for pref in f.get("vitest", [])): return f["name"]
        if path.startswith("src/locales/"): return "Ajustes, idioma y actualizaciones"
    if path.startswith("src-tauri/migrations/"): return "Transversal (migración: revisar paridad de esquema y oráculos SQL)"
    if path.startswith("src-tauri/evals/"): return "Chat con el buzón (evals)"
    return "Transversal / sin mapear"

touched = collections.defaultdict(list)
for fpath in sorted(set(files)): touched[feature_of(fpath)].append(fpath)

runs = sorted(p for p in (REPO / "src-tauri/reports/verify").glob("*-full") if (p / "results.json").exists())
layers_by_feature = {}
if runs:
    for r in json.loads((runs[-1] / "results.json").read_text())["records"]:
        layers_by_feature.setdefault(r["feature"], set()).add(r["type"])

print(f"since {since}: {len(set(files))} files touched\n")
for feat, fl in touched.items():
    have = sorted(layers_by_feature.get(feat, []))
    missing = [t for t in ("unit", "integration", "contract", "e2e", "eval") if t not in have]
    print(f"== {feat}")
    for fpath in fl: print(f"   {fpath}")
    if runs: print(f"   layers today: {', '.join(have) or 'none'} | missing: {', '.join(missing) or 'none'}")
    print()
