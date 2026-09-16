#!/usr/bin/env python3
"""Turn an authoring plan into a manifest build_short.py can read.

  make_manifest.py <plan.json> <manifest.json>

The plan keeps one caption per language in a single dict and carries the
narration text beside each shot, so a line and the picture it describes stay
together while editing. build_short.py wants the English caption on its own
with the other languages in caption_i18n, and knows nothing about narration.
"""
import json
import sys
from pathlib import Path

DROP = ("vo", "vo_at_end")


def convert(seg):
    out = {k: v for k, v in seg.items() if k not in DROP}
    caption = seg.get("caption")
    if isinstance(caption, dict):
        rest = {k: v for k, v in caption.items() if k != "en"}
        out["caption"] = caption["en"]
        if rest:
            out["caption_i18n"] = rest
    return out


def main(plan_path, manifest_path):
    plan = json.loads(Path(plan_path).read_text(encoding="utf-8"))
    manifest = {k: v for k, v in plan.items() if k != "segments"}
    manifest["segments"] = [convert(s) for s in plan["segments"]]
    Path(manifest_path).write_text(json.dumps(manifest, indent=1, ensure_ascii=False), encoding="utf-8")
    langs = sorted({l for s in manifest["segments"] for l in (s.get("caption_i18n") or {})})
    print(f"wrote {manifest_path}: {len(manifest['segments'])} segments, "
          f"subtitle tracks en + {', '.join(langs)}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
