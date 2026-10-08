"""Write one .srt per language from a cue list.

    uv run --no-project python make_srt.py cues.json <out_dir> <name>

cues.json: [{"start": 0.1, "end": 2.0, "text": {"en": "...", "es": "...", "fr": "...", "de": "..."}}, ...]
Times are seconds on the teaser timeline. A text-only teaser's subtitles are the
on-screen words, translated: put each beat's headline + subline in one cue that
spans the beat (start ~0.1 s after the scene starts, end when it ends).
"""
import json
import sys
from pathlib import Path


def ts(x):
    ms = int(round(x * 1000))
    return f"{ms // 3600000:02}:{ms // 60000 % 60:02}:{ms // 1000 % 60:02},{ms % 1000:03}"


def main():
    cues = json.loads(Path(sys.argv[1]).read_text())
    out, name = Path(sys.argv[2]), sys.argv[3]
    langs = sorted({lang for c in cues for lang in c["text"]})
    for lang in langs:
        rows = [c for c in sorted(cues, key=lambda c: c["start"]) if lang in c["text"]]
        body = "".join(f"{i}\n{ts(c['start'])} --> {ts(c['end'])}\n{c['text'][lang]}\n\n" for i, c in enumerate(rows, 1))
        (out / f"{name}-{lang}.srt").write_text(body)
        print(out / f"{name}-{lang}.srt", len(rows), "cues")


if __name__ == "__main__":
    main()
