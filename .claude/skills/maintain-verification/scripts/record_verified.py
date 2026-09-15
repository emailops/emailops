#!/usr/bin/env python3
"""Record the last full run as the verified baseline (commit, date, counts)."""
import json, subprocess, datetime, pathlib
REPO = pathlib.Path(__file__).resolve().parents[4]
run = REPO / "src-tauri/reports/verify/current-full"
r = json.loads((run / "results.json").read_text())
c = {s: sum(x["status"] == s for x in r["records"]) for s in ("ok", "fail", "skip", "info")}
out = REPO / ".claude/skills/verify-emailops/verified.json"
out.write_text(json.dumps({"commit": r["meta"].get("commit", "").split()[0], "date": datetime.date.today().isoformat(), "counts": c, "tests": len(r["records"]), "run": run.resolve().name}, indent=1) + "\n")
print(out, c)
