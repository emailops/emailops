"""Where each narration line starts, shared by the timing fitter and the mixer.

A segment flagged "vo_at_end" holds its line back so the voice lands just
before the shot ends: that is what makes a title card breathe before the
explanation begins.
"""
TAIL = 0.9    # silence after a line that sits at the end of its shot (matches GAP)


def segment_starts(segs):
    starts, t = [], 0.0
    for s in segs:
        starts.append(t)
        t += s["seconds"]
    return starts, t


def cue_starts(segs, durs):
    """Return (cue_start, segment_index) for every narrated segment."""
    starts, _ = segment_starts(segs)
    idx = [i for i, s in enumerate(segs) if s.get("vo")]
    assert len(idx) == len(durs), f"{len(idx)} cues vs {len(durs)} narration files"
    out = []
    for i, d in zip(idx, durs):
        begin = starts[i]
        if segs[i].get("vo_at_end"):
            begin = max(begin, starts[i] + segs[i]["seconds"] - d - TAIL)
        out.append((begin, i))
    return out
