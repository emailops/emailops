#!/usr/bin/env python3
"""Stretch plan segments until every narration line fits with room to breathe.

Run:  python3 fit_timings.py <plan.json> <vo_dir> [--write]

A segment marked "vo_at_end" holds its line until just before the shot ends, so
a feature card stays silent for a beat before the explanation starts. Segments
only ever grow: the pictures wait for the voice, never the other way round.
"""
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from vo_timing import TAIL, cue_starts, segment_starts

GAP = 0.9  # silence between one line ending and the next beginning


def duration(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)],
        capture_output=True, text=True, check=True).stdout.strip()
    return float(out)


def fit(segs, durs):
    """Grow segments until no line starts before the previous one has finished.

    Which segment to stretch depends on where the late line sits. A line held to
    the end of its own shot moves later when that shot grows; a line that starts
    with its shot only moves if something *before* it grows.
    """
    for _ in range(400):
        cues = cue_starts(segs, durs)
        prev_end, prev_seg, grew = -GAP, None, False
        for (start, seg_i), d in zip(cues, durs):
            need = prev_end + GAP - start
            if need > 0.01:
                if segs[seg_i].get("vo_at_end"):
                    target = seg_i
                elif prev_seg is not None and prev_seg < seg_i - 1:
                    target = seg_i - 1
                elif prev_seg is not None and not segs[prev_seg].get("vo_at_end"):
                    target = prev_seg
                else:
                    raise SystemExit(f"no room to delay the line in segment {seg_i}")
                segs[target]["seconds"] = round(segs[target]["seconds"] + need, 2)
                grew = True
                break
            prev_end, prev_seg = start + d, seg_i
        if not grew:
            return cue_starts(segs, durs)
    raise SystemExit("timings did not converge")


def main():
    plan_path, vo_dir = Path(sys.argv[1]), Path(sys.argv[2])
    plan = json.loads(plan_path.read_text())
    segs = plan["segments"]
    durs = [duration(f) for f in sorted(vo_dir.glob("line_*.wav"))]
    cues = fit(segs, durs)
    starts, total = segment_starts(segs)

    prev_end = 0.0
    for n, ((start, seg_i), d) in enumerate(zip(cues, durs), 1):
        seg = segs[seg_i]
        held = "end" if seg.get("vo_at_end") else "   "
        print(f"line {n:2d}  seg {seg_i:2d} {held}  starts {start:6.2f}  ends {start + d:6.2f}"
              f"  gap {start - prev_end:5.2f}  shot {starts[seg_i]:6.2f}+{seg['seconds']:5.2f}")
        prev_end = start + d
    print(f"\ntotal {total:.2f}s, last line ends {prev_end:.2f}s, tail {total - prev_end:.2f}s")

    if "--write" in sys.argv:
        plan_path.write_text(json.dumps(plan, indent=1, ensure_ascii=False))
        print(f"written {plan_path}")


if __name__ == "__main__":
    main()
