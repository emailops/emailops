#!/usr/bin/env python3
"""Compose a short with an animated camera that moves to where the action is.

    uv run --no-project --with pillow python build_camera.py camera.json out.mp4

Each shot is a still screenshot seen through a camera that eases between
views: an establishing view of the whole window, then a smooth zoom onto the
control about to be clicked, the text being typed or the result that
appeared. The pointer glides between targets and a red ripple marks each
click. Cards (title, section, closing) are shown as they are. The title and
captions are burned in through an ASS file, and an .srt with the captions and
every card's `srt` text is written beside the video.

camera.json (coordinates are CSS pixels, as in rects.json; `css_scale`
converts them to screenshot pixels — 2 on a retina Mac):

  {"size": [1080, 1920], "stage": [0, 300, 1080, 1300], "css_scale": 2,
   "image_size": [1800, 900],            # CSS size of the usable screenshot
   "title": "…", "title_from": 4.0,      # standing title band (optional)
   "shots": [
     {"card": "cards/card-hero.png", "dur": 4.0, "srt": "Card text for the .srt"},
     {"img": "frames/f03.png", "dur": 3.0, "slow": 1.35,
      "cam": [[0, "full"], [0.7, "full"], [1.9, [350, 517, 700]]],
      "ptr": [[0.5, [750, 350]], [2.3, [117, 517]]], "click": 2.55,
      "cap": [[0, 3.0, "Abre Lentes"]]},
     {"img": "frames/f04.png", "dur": 3.0, "trans": "dissolve", …}]}

A view is "full" (the whole window, letterboxed), [cx, cy, w] (fills the
stage, height from the stage's aspect) or [cx, cy, w, h] (letterboxed to its
own aspect — a wide table). Views are clamped to the picture. `trans` is
"fade" (through the background, the default — use it between sections) or
"dissolve" (same screen, next moment). `slow` stretches a shot and every time
inside it.
"""
import json
import math
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

FPS = 30
FADE, DISSOLVE = 0.3, 0.35
POINTER = [(0, 0), (0, 34), (8, 26), (13, 38), (19, 35), (14, 23), (24, 23)]


def ease(t):
    t = min(1.0, max(0.0, t))
    return t * t * (3 - 2 * t)


def at_keys(keys, t, lerp):
    """keys: [(t, value)], held before the first and after the last."""
    if t <= keys[0][0]:
        return keys[0][1]
    for (t0, v0), (t1, v1) in zip(keys, keys[1:]):
        if t <= t1:
            return lerp(v0, v1, ease((t - t0) / max(1e-6, t1 - t0)))
    return keys[-1][1]


def lerp_view(a, b, e):
    # Zoom in log space so the speed feels even at every scale.
    def log_lerp(x, y):
        return math.exp(math.log(x) + (math.log(y) - math.log(x)) * e)
    return (a[0] + (b[0] - a[0]) * e, a[1] + (b[1] - a[1]) * e, log_lerp(a[2], b[2]), log_lerp(a[3], b[3]))


def lerp_pt(a, b, e):
    return (a[0] + (b[0] - a[0]) * e, a[1] + (b[1] - a[1]) * e)


def ts_ass(x):
    h, rem = divmod(x, 3600)
    m, sec = divmod(rem, 60)
    return f"{int(h)}:{int(m):02d}:{sec:05.2f}"


def ts_srt(x):
    ms = round(x * 1000)
    h, ms = divmod(ms, 3_600_000)
    m, ms = divmod(ms, 60_000)
    s, ms = divmod(ms, 1000)
    return f"{h:02d}:{m:02d}:{s:02d},{ms:03d}"


class Composer:
    def __init__(self, spec, base):
        self.base = base
        self.W, self.H = spec.get("size", [1080, 1920])
        self.SX, self.SY, self.SW, self.SH = spec.get("stage", [0, 300, 1080, 1300])
        self.k_css = spec.get("css_scale", 2)
        iw, ih = spec.get("image_size", [1800, 900])
        self.IW, self.IH = iw * self.k_css, ih * self.k_css
        self.bg = tuple(spec.get("bg", [15, 23, 42]))
        self.cache = {}

    def view(self, v):
        """A spec view in CSS px -> (cx, cy, w, h) in screenshot px, clamped."""
        k = self.k_css
        if v == "full":
            return (self.IW / 2, self.IH / 2, self.IW, self.IH)
        if len(v) == 3:
            cx, cy, w = (c * k for c in v)
            h = w * self.SH / self.SW
        else:
            cx, cy, w, h = (c * k for c in v)
        return self.clamp((cx, cy, w, h))

    def clamp(self, v):
        cx, cy, w, h = v
        w, h = min(w, self.IW), min(h, self.IH)
        cx = min(max(cx, w / 2), self.IW - w / 2)
        cy = min(max(cy, h / 2), self.IH - h / 2)
        return (cx, cy, w, h)

    def load(self, rel, crop):
        if rel not in self.cache:
            im = Image.open(self.base / rel).convert("RGB")
            if crop:  # drop the webview's blank strip below the window
                im = im.crop((0, 0, self.IW, self.IH))
            self.cache[rel] = im
        return self.cache[rel]

    def stage(self, shot, t):
        out = Image.new("RGB", (self.SW, self.SH), self.bg)
        if "card" in shot:
            card = self.load(shot["card"], crop=False)
            out.paste(card, ((self.SW - card.width) // 2, (self.SH - card.height) // 2))
            return out
        im = self.load(shot["img"], crop=True)
        cx, cy, w, h = self.clamp(at_keys(shot["cam"], t, lerp_view))
        k = min(self.SW / w, self.SH / h)
        ow, oh = round(w * k), round(h * k)
        box = (cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2)
        out.paste(im.resize((ow, oh), Image.LANCZOS, box=box), ((self.SW - ow) // 2, (self.SH - oh) // 2))
        ox, oy = (self.SW - ow) // 2, (self.SH - oh) // 2

        def to_stage(p):
            return (ox + (p[0] - box[0]) * k, oy + (p[1] - box[1]) * k)

        draw = ImageDraw.Draw(out, "RGBA")
        click = shot.get("click")
        if click is not None and click <= t < click + 0.7:
            u = (t - click) / 0.7
            x, y = to_stage(shot["ptr"][-1][1])
            r = 14 + 34 * u
            draw.ellipse((x - r, y - r, x + r, y + r), fill=(255, 64, 48, int(170 * (1 - u))))
        if shot.get("ptr") and t >= shot["ptr"][0][0] - 0.15:
            x, y = to_stage(at_keys(shot["ptr"], t, lerp_pt))
            pts = [(x + px * 1.35, y + py * 1.35) for px, py in POINTER]
            draw.polygon([(a + 2, b + 2) for a, b in pts], fill=(0, 0, 0, 90))
            draw.polygon(pts, fill=(255, 255, 255, 255), outline=(20, 20, 20, 255))
        return out


def normalise(shot, comp):
    """Convert a spec shot to screenshot px and apply its `slow` factor."""
    k = shot.get("slow", 1.0)
    out = dict(shot, dur=shot["dur"] * k)
    if "cam" in shot:
        out["cam"] = [(t * k, comp.view(v)) for t, v in shot["cam"]]
    if "ptr" in shot:
        out["ptr"] = [(t * k, (p[0] * comp.k_css, p[1] * comp.k_css)) for t, p in shot["ptr"]]
    if "click" in shot:
        out["click"] = shot["click"] * k
    if "cap" in shot:
        out["cap"] = [(a * k, b * k, c) for a, b, c in shot["cap"]]
    return out


def main(spec_path, out_path):
    spec_path = Path(spec_path)
    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    comp = Composer(spec, spec_path.parent)
    shots = [normalise(s, comp) for s in spec["shots"]]
    for s in shots:
        if "img" in s and "cam" not in s:
            raise SystemExit(f"{s['img']}: a screenshot shot needs a `cam`")
        if s.get("click") is not None and not s.get("ptr"):
            raise SystemExit(f"{s['img']}: a click needs `ptr` keys ending on the target")

    starts, t = [], 0.0
    for s in shots:
        starts.append(t)
        t += s["dur"]
    total = t

    events, cues = [], []
    if spec.get("title"):
        title_to = starts[-1] if "card" in shots[-1] else total
        events.append(f"Dialogue: 0,{ts_ass(spec.get('title_from', 0))},{ts_ass(title_to)},Title,,0,0,0,,{spec['title']}")
    for s, st in zip(shots, starts):
        if s.get("srt"):
            cues.append((st, st + s["dur"], s["srt"]))
        for a, b, text in s.get("cap", []):
            events.append(f"Dialogue: 1,{ts_ass(st + a)},{ts_ass(st + b)},Caption,,0,0,0,,{text}")
            cues.append((st + a, st + b, text))
    font = spec.get("font", "Helvetica Neue")
    work = spec_path.parent / "build"
    work.mkdir(exist_ok=True)
    ass = work / "camera.ass"
    ass.write_text("\n".join([
        "[Script Info]", "ScriptType: v4.00+", f"PlayResX: {comp.W}", f"PlayResY: {comp.H}", "WrapStyle: 0", "",
        "[V4+ Styles]",
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, "
        "Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, "
        "MarginR, MarginV, Encoding",
        f"Style: Title,{font},{spec.get('title_size', 52)},&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,1,0,0,0,100,100,0,0,1,0,0,8,60,60,110,1",
        f"Style: Caption,{font},{spec.get('caption_size', 50)},&H00FFFFFF,&H00FFFFFF,&H00000000,&HB0000000,1,0,0,0,100,100,0,0,3,18,0,2,70,70,{spec.get('caption_margin', 120)},1",
        "", "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
        *events, ""]), encoding="utf-8")

    enc = subprocess.Popen(
        ["ffmpeg", "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{comp.W}x{comp.H}",
         "-r", str(FPS), "-i", "-", "-vf", f"subtitles='{ass}'", "-c:v", "libx264", "-preset", "slow",
         "-crf", "16", "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(out_path)],
        stdin=subprocess.PIPE)
    blank = Image.new("RGB", (comp.SW, comp.SH), comp.bg)
    canvas = Image.new("RGB", (comp.W, comp.H), comp.bg)
    last = {}
    frames = round(total * FPS)
    for n in range(frames):
        T = n / FPS
        i = max(j for j, st in enumerate(starts) if st <= T + 1e-9)
        shot, lt = shots[i], T - starts[i]
        frame = comp.stage(shot, lt)
        trans = shot.get("trans", "fade")
        if trans == "dissolve" and i > 0 and lt < DISSOLVE:
            prev = last.get(i - 1) or comp.stage(shots[i - 1], shots[i - 1]["dur"])
            frame = Image.blend(prev, frame, ease(lt / DISSOLVE))
        elif trans == "fade" and lt < FADE:
            frame = Image.blend(blank, frame, lt / FADE)
        nxt = shots[i + 1] if i + 1 < len(shots) else None
        if (nxt is None or nxt.get("trans", "fade") == "fade") and shot["dur"] - lt < FADE:
            frame = Image.blend(blank, frame, max(0.0, (shot["dur"] - lt) / FADE))
        if shot["dur"] - lt <= 1 / FPS + 1e-9:
            last[i] = frame
        canvas.paste(frame, (comp.SX, comp.SY))
        enc.stdin.write(canvas.tobytes())
    enc.stdin.close()
    if enc.wait() != 0:
        raise SystemExit("ffmpeg failed")

    Path(out_path).with_suffix(".srt").write_text(
        "".join(f"{k}\n{ts_srt(a)} --> {ts_srt(b)}\n{c}\n\n" for k, (a, b, c) in enumerate(sorted(cues), 1)),
        encoding="utf-8")
    print(f"wrote {out_path} ({total:.1f}s, {frames} frames) and its .srt")
    print("shot starts:", ", ".join(f"{s:.1f}" for s in starts))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
