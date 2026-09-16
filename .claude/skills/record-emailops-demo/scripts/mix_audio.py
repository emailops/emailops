"""Mix narration over the music bed for the promo.

  mix_audio.py <plan.json> <music.wav> <vo_dir> <out.wav>

Each plan segment with a "vo" line gets its narration placed at the second the
segment starts. The music is ducked under the narration with a sidechain
compressor, so the voice stays readable without riding the gain by hand.
"""
import json, subprocess, sys
from pathlib import Path


def duration(path):
    out = subprocess.check_output(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1", str(path)],
        text=True)
    return float(out.strip())


def main(plan_path, music_path, vo_dir, out_path):
    plan = json.loads(Path(plan_path).read_text(encoding="utf-8"))
    vo_dir = Path(vo_dir)
    sys.path.insert(0, str(Path(__file__).parent))
    from vo_timing import cue_starts, segment_starts

    segs = plan["segments"]
    files = sorted(vo_dir.glob("line_*.wav"))
    _, total = segment_starts(segs)
    lines = [start for start, _ in cue_starts(segs, [duration(f) for f in files])]

    inputs = ["-i", str(music_path)]
    for f in files:
        inputs += ["-i", str(f)]

    # Delay each line to its cue, pad them all to the full length, sum them.
    parts, labels = [], []
    for i, (f, start) in enumerate(zip(files, lines), start=1):
        end = start + duration(f)
        if end > total + 0.05:
            print(f"warning: line {i} runs {end - total:.2f}s past the end")
        parts.append(f"[{i}:a]adelay={int(start * 1000)}|{int(start * 1000)},apad[v{i}]")
        labels.append(f"[v{i}]")
    chain = ";".join(parts)
    n = len(labels)
    chain += f";{''.join(labels)}amix=inputs={n}:duration=longest[vosum]"
    chain += f";[vosum]volume={n}[vo]"
    chain += ";[vo]volume=1.6,atrim=0:%.3f,asplit=2[duck][voice]" % total
    # Duck the bed: the voice drives the compressor, the music is compressed.
    chain += ";[0:a][duck]sidechaincompress=threshold=0.05:ratio=8:attack=20:release=400[bed]"
    # The final amix halves both sides equally, so the balance survives and
    # loudnorm sets the absolute level.
    chain += ";[bed][voice]amix=inputs=2:duration=first,loudnorm=I=-15:TP=-1.5:LRA=11[out]"

    cmd = ["ffmpeg", "-v", "error", "-y", *inputs, "-filter_complex", chain,
           "-map", "[out]", "-ar", "48000", "-ac", "2", str(out_path)]
    subprocess.run(cmd, check=True)
    print(f"wrote {out_path} ({total:.1f}s, {len(files)} narration lines)")


if __name__ == "__main__":
    main(*sys.argv[1:5])
