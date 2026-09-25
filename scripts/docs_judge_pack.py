#!/usr/bin/env python3
"""Pack the evidence for the agent to judge what no deterministic check proves.

Reads a docs-check run (results.json) and writes <run>/judge/packets.json: one
packet per block, listing the block's yellow sentences to judge and, once, the
evidence to judge them against — the screens the app showed while the block's
cases ran, or the files a `judge` entry in docs/site/claims.toml names. Blocks
with no evidence at all are left out: a judgment without evidence is a guess.

The agent running the maintain-docs skill reads the packets and writes
<run>/judge/judgments.json:

    [{"claim": "...", "sentence": "<the packet's sentence, verbatim>",
      "verdict": "supported" | "contradicted" | "insufficient",
      "reason": "one or two sentences, citing the evidence",
      "evidence": "the quote from the evidence the verdict rests on",
      "fix": "for contradicted: the edit the docs need",
      "model": "<the judging model>"}]

then `scripts/check_docs.sh --render <run>` folds them into the report.

Usage: docs_judge_pack.py <run dir>
"""

import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from docs_claims_lib import ROOT, load_catalog, plain  # noqa: E402

MAX_FILE = 6000
MAX_SCREEN = 4000


def packets(results, app_parts, catalog):
    out = []
    for page in results["pages"]:
        for it in page["items"]:
            cid = it.get("claim")
            if not cid:
                continue
            judge = [ch for ch in catalog.get(cid, {}).get("checks", []) if "judge" in ch]
            screens = [{"case": p.get("name", ""), "phase": p.get("phase", ""), "observed": p.get("detail", ""),
                        "screen": p.get("screen", "")[:MAX_SCREEN],
                        "shots": p.get("shots", []) if any(ch.get("visual") for ch in judge) else []}
                       for p in app_parts.get(cid, []) if p.get("screen") or p.get("detail")]
            files = []
            for ch in judge:
                for ref in ch.get("evidence", []):
                    if ref.startswith("file:"):
                        path = ROOT / ref[5:]
                        text = path.read_text(encoding="utf-8", errors="replace") if path.exists() else "(missing)"
                        files.append({"path": ref[5:], "text": text[:MAX_FILE]})
            if not screens and not files:
                continue
            block = " ".join(plain(f["text"]) for f in it["fragments"] if f["kind"] != "header")
            sentences = [plain(f["text"]) for f in it["fragments"] if f["color"] == "yellow" and (
                judge or not any(v["state"] in ("manual", "none") for v in f["validations"]))]
            if sentences:  # manual/none ones are accepted as they are unless the catalog asks
                out.append({"claim": cid, "page": page["page"], "sentences": sentences, "block": block,
                            "question": " ".join(ch["judge"] for ch in judge),
                            "app_evidence": screens, "files": files})
    return out


def main():
    run = pathlib.Path(sys.argv[1])
    results = json.loads((run / "results.json").read_text())
    app_parts = {}
    for f in sorted((run / "app").glob("*.json")):
        if f.name == "claims.json":
            continue
        for r in json.loads(f.read_text()):
            r["phase"] = f.stem
            app_parts.setdefault(r.get("claim"), []).append(r)
    got = packets(results, app_parts, load_catalog())
    (run / "judge").mkdir(exist_ok=True)
    (run / "judge" / "packets.json").write_text(json.dumps(got, ensure_ascii=False, indent=1))
    n = sum(len(p["sentences"]) for p in got)
    print(f"{n} frases para juzgar en {len(got)} bloques → {run}/judge/packets.json")


if __name__ == "__main__":
    main()
