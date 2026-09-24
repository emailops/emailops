"""Effects and renderer for EmailOps vertical shorts (1080x1920).

A short is a Python storyboard: a list of shots, each a duration and a
function that draws the frame at local time t. This module holds everything
those functions draw with:

- Panel     a screenshot seen through a camera view, placed in a rectangle
- cam/path  eased keyframes for views and pointer positions
- text_block kinetic text with **highlighted** words, optional backing box
- pill      a coloured tag (POV / ANTES / DESPUÉS)
- highlight a rounded outline that pops onto a region, with a label
- pointer   the mouse pointer and its click ripple
- typing    replace an input's text with its first n characters, with caret
- table_reveal / insert_row  make an extracted table fill in, or grow a row
- end_card  the closing card
- render / preview  encode the storyboard, or save single frames to check

Coordinates in storyboards are CSS pixels, as in rects.json; screenshots are
retina (2x). Run with Pillow, which is not a project dependency:

    uv run --no-project --with pillow python storyboard.py out.mp4
    uv run --no-project --with pillow python storyboard.py preview 3.0 12.5
"""
import math
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

W, H, FPS = 1080, 1920, 30
BG = (15, 23, 42)
YELLOW, WHITE, BLUE, GREY = (250, 204, 21), (255, 255, 255), (147, 197, 253), (148, 163, 184)
RED, GREEN = (239, 68, 68), (34, 197, 94)
# Helvetica Neue faces: 0 Regular, 1 Bold, 10 Medium. It has no arrow glyph:
# "→" renders as a box, so write "a", ":" or "," instead.
TTC = "/System/Library/Fonts/HelveticaNeue.ttc"
CSS = 2                       # screenshot px per CSS px
IW, IH = 1800 * CSS, 900 * CSS  # usable screenshot (the strip below is blank)
STAGE = (0, 330, 1080, 1260)  # where the app sits between the text zones
STAGE_ASPECT = STAGE[2] / STAGE[3]
FRAMES_DIR = Path("frames")
_fonts, _imgs = {}, {}


def font(size, face=1):
    if (size, face) not in _fonts:
        _fonts[(size, face)] = ImageFont.truetype(TTC, size, index=face)
    return _fonts[(size, face)]


def img(name):
    """A capture from FRAMES_DIR, cropped to the usable window."""
    if name not in _imgs:
        im = Image.open(FRAMES_DIR / f"{name}.png").convert("RGB")
        _imgs[name] = im.crop((0, 0, IW, IH))
    return _imgs[name]


def base():
    return Image.new("RGB", (W, H), BG)


# ── Timing ────────────────────────────────────────────────────────────────
def ease(t):
    t = min(1.0, max(0.0, t))
    return t * t * (3 - 2 * t)


def back(t):
    """Ease-out with a small overshoot, for pops. Never below 0: a radius
    computed from a float -2e-16 makes Pillow refuse to draw."""
    t = min(1.0, max(0.0, t))
    c = 1.7
    return max(0.0, 1 + (c + 1) * (t - 1) ** 3 + c * (t - 1) ** 2)


def V(cx, cy, w, h=None, aspect=STAGE_ASPECT):
    """A view in CSS px -> screenshot px. Without h, it fills a rectangle of
    `aspect`; with h it is letterboxed to its own shape (a wide table)."""
    h = h if h is not None else w / aspect
    return clamp((cx * CSS, cy * CSS, w * CSS, h * CSS))


def clamp(v):
    cx, cy, w, h = v
    w, h = min(w, IW), min(h, IH)
    return (min(max(cx, w / 2), IW - w / 2), min(max(cy, h / 2), IH - h / 2), w, h)


FULL = (IW / 2, IH / 2, IW, IH)


def cam(keys, t):
    """keys: [(t, view)]; zoom eases in log space so speed feels even."""
    def lerp(a, b, e):
        lg = lambda x, y: math.exp(math.log(x) + (math.log(y) - math.log(x)) * e)
        return (a[0] + (b[0] - a[0]) * e, a[1] + (b[1] - a[1]) * e, lg(a[2], b[2]), lg(a[3], b[3]))
    return _keyed(keys, t, lerp)


def path(keys, t):
    """keys: [(t, (x, y))] in CSS px, for the pointer."""
    return _keyed(keys, t, lambda a, b, e: (a[0] + (b[0] - a[0]) * e, a[1] + (b[1] - a[1]) * e))


def _keyed(keys, t, lerp):
    if t <= keys[0][0]:
        return keys[0][1]
    for (t0, v0), (t1, v1) in zip(keys, keys[1:]):
        if t <= t1:
            return lerp(v0, v1, ease((t - t0) / max(1e-6, t1 - t0)))
    return keys[-1][1]


# ── Drawing ───────────────────────────────────────────────────────────────
class Panel:
    """A screenshot seen through `view`, fitted into `rect` of the frame."""

    def __init__(self, frame, rect, image, view):
        x, y, w, h = rect
        cx, cy, vw, vh = clamp(view)
        self.k = min(w / vw, h / vh)
        ow, oh = round(vw * self.k), round(vh * self.k)
        self.x0, self.y0 = cx - vw / 2, cy - vh / 2
        self.ox, self.oy = x + (w - ow) // 2, y + (h - oh) // 2
        frame.paste(image.resize((ow, oh), Image.LANCZOS, box=(self.x0, self.y0, self.x0 + vw, self.y0 + vh)),
                    (self.ox, self.oy))

    def pt(self, css):
        return (self.ox + (css[0] * CSS - self.x0) * self.k, self.oy + (css[1] * CSS - self.y0) * self.k)

    def rect_of(self, x0, y0, x1, y1):
        a, b = self.pt((x0, y0)), self.pt((x1, y1))
        return (a[0], a[1], b[0], b[1])


def draw(frame):
    return ImageDraw.Draw(frame, "RGBA")


def highlight(frame, box, t, color=YELLOW, label=None, pad=8):
    """Rounded outline popping onto `box` (frame px) from t=0, with a tag."""
    if t < 0:
        return
    e = back(t / 0.3)
    x0, y0, x1, y1 = box
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    hw, hh = (x1 - x0) / 2 * e + pad, (y1 - y0) / 2 * e + pad
    d = draw(frame)
    d.rounded_rectangle((cx - hw, cy - hh, cx + hw, cy + hh), radius=10,
                        fill=color + (40,), outline=color + (255,), width=5)
    if label and t > 0.15:
        f = font(30, 1)
        tw = d.textlength(label, font=f)
        lx, ly = cx + hw + 14, cy - 22
        if lx + tw + 24 > W - 10:
            lx = cx - hw - tw - 38
        d.rounded_rectangle((lx, ly, lx + tw + 24, ly + 44), radius=22, fill=color + (255,))
        d.text((lx + 12, ly + 5), label, font=f, fill=BG)


def pointer(frame, p, click_u=None):
    """Pointer at frame point p; click_u in [0, 1) draws the ripple."""
    d = draw(frame)
    x, y = p
    if click_u is not None and 0 <= click_u < 1:
        r = 14 + 40 * click_u
        d.ellipse((x - r, y - r, x + r, y + r), fill=(255, 64, 48, int(180 * (1 - click_u))))
    shape = [(0, 0), (0, 34), (8, 26), (13, 38), (19, 35), (14, 23), (24, 23)]
    pts = [(x + a * 1.5, y + b * 1.5) for a, b in shape]
    d.polygon([(a + 2, b + 3) for a, b in pts], fill=(0, 0, 0, 100))
    d.polygon(pts, fill=(255, 255, 255, 255), outline=(20, 20, 20, 255))


def _rich_lines(text, size, face, max_w):
    # Keep punctuation inside the markup ("**solo.**"), or it lands after a space.
    words, hl = [], False
    for chunk in text.split("**"):
        words += [(w, hl) for w in chunk.split()]
        hl = not hl
    f = font(size, face)
    lines, cur = [], []
    for w in words:
        if cur and f.getlength(" ".join(x for x, _ in cur + [w])) > max_w:
            lines.append(cur)
            cur = [w]
        else:
            cur.append(w)
    return lines + ([cur] if cur else [])


def text_block(frame, text, y, t, size=74, face=1, color=WHITE, hl=YELLOW, max_w=960, box=None):
    """Kinetic text popping in line by line from t=0; returns the y below it."""
    if t < 0:
        return y
    f = font(size, face)
    lh = round(size * 1.18)
    lines = _rich_lines(text, size, face, max_w)
    layer = Image.new("RGBA", (W, lh * len(lines) + 40), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    for i, line in enumerate(lines):
        u = (t - i * 0.08) / 0.28
        if u <= 0:
            continue
        a = int(255 * min(1, u))
        width = f.getlength(" ".join(w for w, _ in line))
        lx, ly = (W - width) / 2, 10 + i * lh + (1 - back(u)) * 24
        if box:
            d.rounded_rectangle((lx - 18, ly - 6, lx + width + 18, ly + lh - 2), radius=14, fill=box + (int(0.85 * a),))
        for w, is_hl in line:
            d.text((lx, ly), w, font=f, fill=(hl if is_hl else color) + (a,))
            lx += f.getlength(w + " ")
    frame.paste(layer, (0, int(y) - 10), layer)
    return y + lh * len(lines)


def pill(frame, text, x, y, t, color, size=34):
    if t < 0:
        return
    e = back(t / 0.3)
    f = font(size, 1)
    d = draw(frame)
    tw = d.textlength(text, font=f)
    w, h = (tw + 40) * e, (size + 22) * e
    d.rounded_rectangle((x, y, x + w, y + h), radius=h / 2, fill=color + (255,))
    if e > 0.8:
        d.text((x + 20, y + 10), text, font=f, fill=WHITE)


def typing(frame, panel, box, origin, lines, n, font_px=27, line_h=20, color=(31, 41, 55), bg=(255, 255, 255)):
    """Show the first n characters of `lines` in an input, with a caret.

    box: the input's inner area (CSS) to blank; origin: CSS top-left of the
    first line; lines: the text split exactly where the app wraps it (copy it
    from the screenshot, so the last frame matches the real one); font_px:
    glyph size in screenshot px; line_h: CSS px between lines.
    """
    d = draw(frame)
    d.rectangle(panel.rect_of(*box), fill=bg + (255,))
    f = font(max(8, round(font_px * panel.k)), 0)
    left = n
    for i, line in enumerate(lines):
        if left <= 0:
            break
        d.text(panel.pt((origin[0], origin[1] + line_h * i)), line[:left], font=f, fill=color)
        left -= len(line) + 1
    if 0 < n < total_chars(lines):
        acc, li = n, 0
        while li < len(lines) - 1 and acc > len(lines[li]):
            acc -= len(lines[li]) + 1
            li += 1
        px, py = panel.pt((origin[0], origin[1] + line_h * li))
        cx = px + f.getlength(lines[li][:max(0, acc)])
        d.rectangle((cx + 2, py + 2, cx + 5, py + font_px * 1.25 * panel.k), fill=(37, 99, 235, 255))


def total_chars(lines):
    return sum(len(l) + 1 for l in lines) - 1


def table_reveal(frame, panel, t, start, rows_top, row_h, x0, x1, bg=(30, 30, 30), step=0.28):
    """Hide extracted cells (CSS x0..x1) and wipe them in row by row from
    `start`, so the table visibly fills."""
    d = draw(frame)
    for i, top in enumerate(rows_top):
        u = (t - start - i * step) / 0.22
        if u >= 1:
            continue
        a0, b0, a1, b1 = panel.rect_of(x0, top + 4, x1, top + row_h - 4)
        if u > 0:
            a0 += (a1 - a0) * ease(u)
        d.rectangle((a0, b0, a1, b1), fill=bg + (255,))


def insert_row(image, p, rows_box, row_h, cells, sep=(30, 41, 57), bg=(30, 30, 30),
               color=(229, 231, 235), font_px=27, baseline=21):
    """A copy of `image` where a new row slides in on top of the rows.

    rows_box: CSS (x0, top, x1, n_rows) of the existing rows; cells: [(css_x,
    text)] for the new row; p: 0 -> 1 progress. Newest-first tables grow at
    the top, which is where a new email lands.
    """
    x0, top, x1, n = rows_box
    im = image.copy()
    X0, X1, T, RH = x0 * CSS, x1 * CSS, top * CSS, row_h * CSS
    rows = im.crop((X0, T, X1, T + RH * n + 4))
    d = ImageDraw.Draw(im, "RGBA")
    d.rectangle((X0, T, X1, T + RH * (n + 1) + 4), fill=bg + (255,))
    im.paste(rows, (X0, T + round(RH * ease(p))))
    a = int(255 * ease((p - 0.4) / 0.6))
    if a > 0:
        f = font(font_px, 0)
        for cx, text in cells:
            d.text((cx * CSS, T + baseline * CSS), text, font=f, fill=color + (a,), anchor="ls")
        d.rectangle((X0, T + RH - 2, X1, T + RH), fill=sep + (a,))
    return im


def end_card(t, tagline="Cliente de correo libre con IA local",
             footnote="Datos de demostración sintéticos · Música de Kevin MacLeod (incompetech.com) · CC BY 4.0"):
    fr = base()
    text_block(fr, "EmailOps", 560, t, size=130)
    text_block(fr, tagline, 740, t - 0.25, size=52, face=0, color=BLUE)
    text_block(fr, "getemailops.com", 900, t - 0.5, size=70, color=YELLOW)
    text_block(fr, "macOS · Windows · Linux", 1010, t - 0.7, size=42, face=0, color=GREY)
    text_block(fr, footnote, 1760, t - 0.8, size=24, face=0, color=(100, 116, 139), max_w=1000)
    return fr


# ── Output ────────────────────────────────────────────────────────────────
def _starts(shots):
    starts, t = [], 0.0
    for dur, _, _ in shots:
        starts.append(t)
        t += dur
    return starts, t


def render(shots, out, xfade=0.18, fade_out=0.5):
    """shots: [(dur, fn(t) -> frame, srt_text | None)]. Writes out and .srt."""
    starts, total = _starts(shots)
    enc = subprocess.Popen(["ffmpeg", "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
                            "-r", str(FPS), "-i", "-", "-c:v", "libx264", "-preset", "slow", "-crf", "16",
                            "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(out)], stdin=subprocess.PIPE)
    prev_last = None
    for n in range(round(total * FPS)):
        T = n / FPS
        i = max(j for j, s in enumerate(starts) if s <= T + 1e-9)
        dur, fn, _ = shots[i]
        lt = T - starts[i]
        fr = fn(lt)
        if i > 0 and lt < xfade and prev_last is not None:
            fr = Image.blend(prev_last, fr, ease(lt / xfade))
        if dur - lt <= 1 / FPS + 1e-9:
            prev_last = fr
        if i == len(shots) - 1 and dur - lt < fade_out:
            fr = Image.blend(base(), fr, max(0, (dur - lt) / fade_out))
        enc.stdin.write(fr.tobytes())
    enc.stdin.close()
    if enc.wait() != 0:
        raise SystemExit("ffmpeg failed")

    def ts(x):
        ms = round(x * 1000)
        h, ms = divmod(ms, 3_600_000)
        m, ms = divmod(ms, 60_000)
        s, ms = divmod(ms, 1000)
        return f"{h:02d}:{m:02d}:{s:02d},{ms:03d}"
    cues = [(st, st + d, srt) for (d, _, srt), st in zip(shots, starts) if srt]
    Path(out).with_suffix(".srt").write_text(
        "".join(f"{k}\n{ts(a)} --> {ts(b)}\n{c}\n\n" for k, (a, b, c) in enumerate(cues, 1)), encoding="utf-8")
    print(f"wrote {out} ({total:.1f}s) and its .srt")
    print("shot starts:", ", ".join(f"{s:.1f}" for s in starts))


def preview(shots, times, prefix="preview-"):
    """Save the frames at the given times, to check framing before rendering."""
    starts, _ = _starts(shots)
    for T in times:
        i = max(j for j, s in enumerate(starts) if s <= T + 1e-9)
        shots[i][1](T - starts[i]).save(f"{prefix}{T}.png")
        print(f"{prefix}{T}.png")


def main(shots, argv):
    """`storyboard.py out.mp4` renders; `storyboard.py preview 3 7.5` checks frames."""
    if argv and argv[0] == "preview":
        preview(shots, [float(x) for x in argv[1:]])
    elif argv:
        render(shots, argv[0])
    else:
        raise SystemExit("usage: storyboard.py out.mp4 | preview <t> [<t> ...]")
