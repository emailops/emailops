"""Compose a vertical short or a 16:9 promo from UI screenshots.

manifest.json:
  {"title": "..." | null, "font": "Helvetica Neue",
   "size": [W, H], "stage": [x, y, w, h], "caption_size": 56, "title_size": 64,
   "cover": true,   # first segment is a cover card: the title starts after it
   "burn_captions": false,  # write the .srt tracks but keep the picture clean
   "segments[].caption_i18n": {"de": "...", "fr": "..."},  # extra subtitle tracks
   "segments": [{"image": "a.png", "seconds": 2.5, "caption": "...",
                 "crop": [x, y, w, h] | null, "click": [x, y] | null,
                 "cut": true}]}   # cut: joins the previous segment without a fade
Each segment: crop of the screenshot scaled to fit the stage (1080x1340 at
y=330), dark background, title band on top, caption band below, optional
click marker. Segments dip in and out of the background colour (the local ffmpeg has no
xfade) and are concatenated; captions and title
are burned in through an ASS file (libass).
"""
import json, subprocess, sys
from pathlib import Path

FADE = 0.25
DISSOLVE = 0.45     # cross-fade length between shots of the same screen
BG = "0x0f172a"


def ts(t):
    h, rem = divmod(t, 3600); m, s = divmod(rem, 60)
    return f"{int(h)}:{int(m):02d}:{s:05.2f}"


def fit(cw, ch, stage_w, stage_h):
    k = min(stage_w / cw, stage_h / ch)
    return k, round(cw * k) // 2 * 2, round(ch * k) // 2 * 2


# Mouse pointer drawn as an ASS shape, tip at the anchor point.
POINTER = "m 0 0 l 0 34 l 8 26 l 13 38 l 19 35 l 14 23 l 24 23"


def circle(cx, cy, r):
    k = 0.5523 * r
    return (f"m {cx} {cy - r} b {cx + k} {cy - r} {cx + r} {cy - k} {cx + r} {cy} "
            f"b {cx + r} {cy + k} {cx + k} {cy + r} {cx} {cy + r} "
            f"b {cx - k} {cy + r} {cx - r} {cy + k} {cx - r} {cy} "
            f"b {cx - r} {cy - k} {cx - k} {cy - r} {cx} {cy - r}")


def main(manifest_path, out_path):
    base = Path(manifest_path).parent
    m = json.loads(Path(manifest_path).read_text(encoding="utf-8"))
    font = m.get("font", "Helvetica Neue")
    W, H = m.get("size", [1080, 1920])
    STAGE_X, STAGE_Y, STAGE_W, STAGE_H = m.get("stage", [0, 330, W, 1340])
    caption_size = m.get("caption_size", 56)
    title_size = m.get("title_size", 64)
    caption_margin = m.get("caption_margin", 95)
    burn_captions = m.get("burn_captions", True)
    segs = m["segments"]
    work = base / "build"; work.mkdir(exist_ok=True)

    parts, events, srt, t = [], [], [], 0.0
    srt_i18n = {}   # extra subtitle tracks, keyed by language code
    for i, s in enumerate(segs):
        img = base / s["image"]
        iw, ih = map(int, subprocess.check_output(
            ["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height",
             "-of", "csv=p=0", str(img)], text=True).strip().split(","))
        x, y, cw, ch = s.get("crop") or [0, 0, iw, ih]
        k, sw, sh = fit(cw, ch, STAGE_W, STAGE_H)
        ox, oy = STAGE_X + (STAGE_W - sw) // 2, STAGE_Y + (STAGE_H - sh) // 2
        part = work / f"seg{i:02d}.mp4"
        d = s["seconds"]
        canvas = work / f"canvas{i:02d}.png"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(img), "-vf",
                        f"crop={cw}:{ch}:{x}:{y},scale={sw}:{sh}:flags=lanczos,"
                        f"pad={W}:{H}:{ox}:{oy}:color={BG},setsar=1",
                        "-frames:v", "1", str(canvas)], check=True)
        fades = ""
        if not s.get("cut") and not s.get("dissolve"):
            fades += f"fade=t=in:st=0:d={FADE}:color={BG},"
        if i == len(segs) - 1 or not (segs[i + 1].get("cut") or segs[i + 1].get("dissolve")):
            fades += f"fade=t=out:st={d - FADE:.3f}:d={FADE}:color={BG},"
        dis = s.get("dissolve") and i > 0
        if dis:
            # No xfade in this ffmpeg build: overlay the new canvas on the old
            # one and ramp its alpha, which reads as a dissolve.
            prev_canvas = work / f"canvas{i-1:02d}.png"
            chain = (f"[1:v]format=rgba,fade=t=in:st=0:d={DISSOLVE}:alpha=1[top];"
                     f"[0:v][top]overlay=format=auto,fps=30,{fades}format=yuv420p")
            subprocess.run(["ffmpeg", "-v", "error", "-y",
                            "-loop", "1", "-t", f"{d}", "-i", str(prev_canvas),
                            "-loop", "1", "-t", f"{d}", "-i", str(canvas),
                            "-filter_complex", chain, "-c:v", "libx264", "-preset", "veryfast",
                            "-crf", "10", "-tune", "stillimage", str(part)], check=True)
        else:
            vf = f"fps=30,{fades}format=yuv420p"
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-loop", "1", "-t", f"{d}",
                            "-i", str(canvas), "-vf", vf, "-c:v", "libx264", "-preset", "veryfast",
                            "-crf", "10", "-tune", "stillimage", str(part)], check=True)
        parts.append(part)
        start, end = t, t + d
        if s.get("caption"):
            if burn_captions:
                events.append(f"Dialogue: 1,{ts(start)},{ts(end)},Caption,,0,0,0,,{s['caption']}")
            if srt and srt[-1][2] == s["caption"] and abs(srt[-1][1] - start) < 1e-6:
                srt[-1][1] = end  # same caption continues across a cut
            else:
                srt.append([start, end, s["caption"]])
            for lang, text in (s.get("caption_i18n") or {}).items():
                bucket = srt_i18n.setdefault(lang, [])
                if bucket and bucket[-1][2] == text and abs(bucket[-1][1] - start) < 1e-6:
                    bucket[-1][1] = end
                else:
                    bucket.append([start, end, text])
        if s.get("cursor") and not s.get("click"):
            # Cursor with no click: it rests on the screen, or drifts to a
            # second point, so the viewer sees who is driving.
            to_x, to_y = s["cursor"]
            fx, fy = s.get("cursor_from") or [to_x, to_y]
            cx, cy = ox + round((to_x - x) * k), oy + round((to_y - y) * k)
            px, py = ox + round((fx - x) * k), oy + round((fy - y) * k)
            events.append(
                f"Dialogue: 3,{ts(start)},{ts(end)},Pointer,,0,0,0,,"
                f"{{\\an7\\p1\\move({px},{py},{cx},{cy},0,{int(max(0.1, d - 0.2) * 1000)})\\fad(120,150)}}"
                f"{POINTER}{{\\p0}}")
        if s.get("click"):
            cx = ox + round((s["click"][0] - x) * k); cy = oy + round((s["click"][1] - y) * k)
            fx, fy = s.get("cursor_from") or [s["click"][0] - 260, s["click"][1] + 220]
            px = ox + round((fx - x) * k); py = oy + round((fy - y) * k)
            travel, hit = 0.55, 0.62
            # Pointer glides in, then stays put for the rest of the shot.
            events.append(
                f"Dialogue: 3,{ts(start)},{ts(end)},Pointer,,0,0,0,,"
                f"{{\\an7\\p1\\move({px},{py},{cx},{cy},0,{int(travel*1000)})\\fad(120,150)}}"
                f"{POINTER}{{\\p0}}")
            # Red dot that expands and fades where the click lands.
            events.append(
                f"Dialogue: 2,{ts(start + hit)},{ts(min(end, start + hit + 0.75))},Click,,0,0,0,,"
                f"{{\\an7\\pos(0,0)\\p1\\fad(0,450)\\t(0,700,\\alpha&HFF&)}}"
                f"{circle(cx, cy, 20)}{{\\p0}}")
        t += s["seconds"]
    total = t

    title_from = segs[0]["seconds"] if m.get("cover") else 0.0
    ass = work / "overlay.ass"
    ass.write_text("\n".join([
        "[Script Info]", "ScriptType: v4.00+", f"PlayResX: {W}", f"PlayResY: {H}", "WrapStyle: 0", "",
        "[V4+ Styles]",
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, "
        "Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, "
        "MarginR, MarginV, Encoding",
        f"Style: Title,{font},{title_size},&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,1,0,0,0,100,100,0,0,1,0,0,8,60,60,110,1",
        f"Style: Caption,{font},{caption_size},&H00FFFFFF,&H00FFFFFF,&H00000000,&HB0000000,1,0,0,0,100,100,0,0,3,18,0,2,70,70,{caption_margin},1",
        f"Style: Click,{font},20,&H003040FF,&H003040FF,&H002020FF,&H00000000,0,0,0,0,100,100,0,0,1,0,0,5,0,0,0,1",
        f"Style: Pointer,{font},20,&H00FFFFFF,&H00FFFFFF,&H00202020,&H80000000,0,0,0,0,100,100,0,0,1,3,2,7,0,0,0,1",
        "", "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
        # The standing title starts after the cover segment so the two do not
        # compete for the same frame.
        *([f"Dialogue: 0,{ts(title_from)},{ts(total)},Title,,0,0,0,,{m['title']}"] if m.get("title") else []),
        *events, ""]), encoding="utf-8")

    inputs = []
    for p in parts:
        inputs += ["-i", str(p)]
    chain = "".join(f"[{i}:v]" for i in range(len(parts)))
    chain += f"concat=n={len(parts)}:v=1:a=0[c];[c]subtitles='{ass}'[v]"
    subprocess.run(["ffmpeg", "-v", "error", "-y", *inputs, "-filter_complex", chain, "-map", "[v]",
                    "-c:v", "libx264", "-preset", "slow", "-crf", "15", "-tune", "stillimage",
                    "-pix_fmt", "yuv420p",
                    "-movflags", "+faststart", str(out_path)], check=True)
    def srt_ts(t):
        ms = round(t * 1000); h, ms = divmod(ms, 3_600_000); m, ms = divmod(ms, 60_000); sec, ms = divmod(ms, 1000)
        return f"{h:02d}:{m:02d}:{sec:02d},{ms:03d}"
    def write_srt(path, cues):
        path.write_text("".join(f"{n}\n{srt_ts(a)} --> {srt_ts(b)}\n{c}\n\n" for n, (a, b, c) in enumerate(cues, 1)),
                        encoding="utf-8")
        return path.name

    names = [write_srt(Path(out_path).with_suffix(".srt"), srt)]
    for lang, cues in sorted(srt_i18n.items()):
        names.append(write_srt(Path(out_path).with_suffix(f".{lang}.srt"), cues))
    print(f"wrote {out_path} ({total:.1f}s, {len(parts)} segments) and {', '.join(names)}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
