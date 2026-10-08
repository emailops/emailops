"""Primitives for 16:9 EmailOps launch teasers in the "One" style.

Cinematic colour fields, a grid-paper canvas with blue dots, kinetic
word-by-word titles, real UI panels cropped from 2x screenshots with an
animated camera (place / cam_place / focus_state), macOS keycaps, a pointer
with click ripples, and a raw-frame renderer that pipes RGB to ffmpeg.

A storyboard does from teaser_fx import *, defines scene functions
scene(t) -> RGBA Image and a TIMELINE = [(start, dur, scene), ...],
then calls run(TIMELINE). See examples/launch_teaser.py.

Coordinates: screen space is 1920x1080. Panel boxes are CSS pixels of the
1800-wide app window; screenshots are 2x (retina), so panel crops at 2x.

Config (env):
  TEASER_FRAMES  directory the panel names are relative to (default: cwd)
  TEASER_FONTS   directory holding the Inter TTFs (default: ~/.cache/emailops-teaser/fonts,
                 filled by scripts/fetch_fonts.sh)
"""
import math
import os
import subprocess
import sys
from functools import lru_cache
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H, FPS = 1920, 1080, 60
FRAMES = Path(os.environ.get("TEASER_FRAMES", "."))
FONTS = Path(os.environ.get("TEASER_FONTS", Path.home() / ".cache/emailops-teaser/fonts"))
FONT_DISPLAY = str(FONTS / "InterDisplay-SemiBold.ttf")
FONT_MED = str(FONTS / "Inter-Medium.ttf")
FONT_REG = str(FONTS / "Inter-Regular.ttf")
REPO = Path(__file__).resolve().parents[4]
ICON = REPO / "src-tauri/icons/icon.png"
SYMBOLS = "/System/Library/Fonts/Apple Symbols.ttf"  # ⌘ and ↵ glyphs for keycaps

INK = (17, 17, 19)


GREY = (118, 118, 124)


PAPER = (246, 246, 244)


BLUE = (30, 123, 255)


def clamp(x, a=0.0, b=1.0):
    return max(a, min(b, x))


def ramp(t, t0, d):
    return clamp((t - t0) / d) if d > 0 else float(t >= t0)


def ease_out(x):
    return 1 - (1 - x) ** 3


def ease_io(x):
    return 4 * x ** 3 if x < 0.5 else 1 - (-2 * x + 2) ** 3 / 2


def ease_back(x, s=1.4):
    x -= 1
    return 1 + (s + 1) * x ** 3 + s * x ** 2


def lerp(a, b, x):
    return a + (b - a) * x


def loglerp(a, b, x):
    return math.exp(lerp(math.log(a), math.log(b), x))


@lru_cache(None)
def font(path, size):
    return ImageFont.truetype(path, size)


@lru_cache(None)
def text_img(s, path, size, color, tracking=0):
    f = font(path, size)
    l, t, r, b = f.getbbox(s)
    asc, desc = f.getmetrics()
    w = r + max(0, tracking * len(s)) + 4
    im = Image.new("RGBA", (int(w), asc + desc + 4), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    if tracking:
        x = 0
        for ch in s:
            d.text((x, 0), ch, font=f, fill=color)
            x += f.getlength(ch) + tracking
    else:
        d.text((0, 0), s, font=f, fill=color)
    return im


def paste_alpha(canvas, im, x, y, alpha=1.0):
    if alpha <= 0.003:
        return
    if alpha < 0.999:
        im = im.copy()
        a = im.getchannel("A").point(lambda v: int(v * alpha))
        im.putalpha(a)
    canvas.alpha_composite(im, (int(round(x)), int(round(y))))


def words_in(canvas, line, x, y, t, t0, path, size, color, stagger=0.07, rise=26, dur=0.5, align="left",
             out_t=None, out_d=0.25):
    """Word-by-word rise + fade, like the reference's kinetic titles."""
    words = line.split(" ")
    space = font(path, size).getlength(" ")
    imgs = [text_img(w, path, size, color) for w in words]
    total = sum(font(path, size).getlength(w) for w in words) + space * (len(words) - 1)
    cx = x - total / 2 if align == "center" else x
    for i, (w, im) in enumerate(zip(words, imgs)):
        k = ease_out(ramp(t, t0 + i * stagger, dur))
        a = k
        if out_t is not None:
            a *= 1 - ramp(t, out_t, out_d)
        paste_alpha(canvas, im, cx, y + (1 - k) * rise, a)
        cx += font(path, size).getlength(w) + space
    return total


# film grain tiles, vignette and the low-res sampling grid for field()
_grain = [np.clip(np.random.default_rng(i).normal(0, 7, (H, W, 1)), -20, 20).astype(np.int16) for i in range(6)]
_yy, _xx = np.mgrid[0:H, 0:W].astype(np.float32)
_vig = (1 - 0.55 * (((_xx - W / 2) / (W / 2)) ** 2 + ((_yy - H / 2) / (H / 2)) ** 2) ** 1.2 * 0.5)[..., None]
_ly, _lx = np.mgrid[0:270, 0:480].astype(np.float32)


def _make_noise(seed):
    """Fractal value noise, 0..1, on a 960x540 tile we slide a 480x270 window over."""
    rng = np.random.default_rng(seed)
    acc = np.zeros((540, 960), np.float32)
    amp, tot = 1.0, 0.0
    for cells in (6, 12, 24, 48):
        g = rng.random((cells * 9 // 16 + 2, cells + 2)).astype(np.float32)
        im = Image.fromarray((g * 255).astype(np.uint8)).resize((960, 540), Image.BICUBIC)
        acc += np.asarray(im, np.float32) / 255 * amp
        tot += amp
        amp *= 0.5
    acc /= tot
    return (acc - acc.min()) / (acc.max() - acc.min())


_NOISE = {s: _make_noise(s) for s in (1, 2, 3)}


def _noise_at(t, speed, seed):
    n = _NOISE[seed]
    ox = int(240 + 60 * math.sin(t * speed * 0.7 + seed)) % 480
    oy = int(135 + 40 * math.cos(t * speed * 0.5 + seed)) % 270
    return n[oy:oy + 270, ox:ox + 480]


def field(t, palette, blobs, base_top, base_bot, push=0.06, bands=None, clouds=(), horizon=None):
    """A soft cinematic colour field: drifting gaussian blobs over a vertical
    gradient, rendered small and upscaled, then grain + vignette."""
    g = np.linspace(0, 1, 270, dtype=np.float32)[:, None, None]
    img = (np.array(base_top, np.float32) * (1 - g) + np.array(base_bot, np.float32) * g) * np.ones((270, 480, 3), np.float32)
    for (cx, cy, r, ci, vx, vy, strength) in blobs:
        px, py = cx + vx * t, cy + vy * t
        m = np.exp(-(((_lx - px) / r) ** 2 + ((_ly - py) / (r * 0.8)) ** 2))[..., None] * strength
        img = img * (1 - m) + np.array(palette[ci], np.float32) * m
    if bands:
        # agate: rings around a centre, warped by noise, mapped through the palette
        bx, by, freq = bands
        n = _noise_at(t, 0.6, 1)
        dist = np.sqrt((_lx - bx) ** 2 + ((_ly - by) * 1.2) ** 2)
        v = 0.5 + 0.5 * np.sin(dist * freq + n * 13 + t * 0.5)
        v2 = 0.5 + 0.5 * np.sin(dist * freq * 2.7 + n * 5)
        ring = np.array(palette[1], np.float32) * (1 - v[..., None]) + np.array(palette[0], np.float32) * v[..., None]
        ring = ring * (0.85 + 0.15 * v2[..., None])
        mask = np.clip((dist - 40) / 60, 0, 1)[..., None] * np.clip(1 - (dist - 420) / 90, 0, 1)[..., None]
        img = img * (1 - mask) + ring * mask
    for (col, thr, strength, sp, seed) in clouds:
        n = _noise_at(t, sp, seed)
        m = np.clip((n - thr) / 0.35, 0, 1)[..., None] * strength
        img = img * (1 - m) + np.array(col, np.float32) * m
    if horizon:
        hy, hc, hs = horizon
        m = np.exp(-((_ly - hy) / 26) ** 2)[..., None] * hs
        img = img * (1 - m) + np.array(hc, np.float32) * m
    small = Image.fromarray(np.clip(img, 0, 255).astype(np.uint8))
    z = 1 + push * t
    cw, ch = 480 / z, 270 / z
    small = small.resize((W, H), Image.BICUBIC, box=((480 - cw) / 2, (270 - ch) / 2, (480 + cw) / 2, (270 + ch) / 2))
    a = np.asarray(small).astype(np.float32) * _vig
    a = a.astype(np.int16) + _grain[int(t * 24) % 6]
    return Image.fromarray(np.clip(a, 0, 255).astype(np.uint8)).convert("RGBA")


FIELDS = {
    # agate slice: cream / amber / rust rings
    "agate": dict(palette=[(252, 240, 222), (214, 120, 46), (150, 60, 24), (255, 250, 240)],
                  blobs=[(380, 60, 160, 3, -3, 4, .6)], base_top=(200, 196, 192), base_bot=(170, 160, 150),
                  bands=(70, 330, 0.12), push=0.08),
    # the planet from orbit: navy, cyan, swirling cloud
    "ocean": dict(palette=[(8, 30, 70), (30, 100, 170), (235, 242, 250), (255, 214, 160)],
                  blobs=[(240, 330, 330, 1, 0, -4, .95), (60, 20, 70, 3, 2, 1, .5)],
                  base_top=(2, 4, 14), base_bot=(10, 30, 70),
                  clouds=[((240, 245, 252), 0.52, 0.9, 0.9, 2)], horizon=(118, (120, 180, 255), 0.35)),
    # launch morning: blue sky, warm horizon, cloud bank
    "sky": dict(palette=[(110, 160, 215), (255, 200, 150), (255, 240, 220), (70, 110, 170)],
                blobs=[(240, 300, 260, 1, 0, -3, .7)],
                base_top=(60, 110, 180), base_bot=(170, 195, 220),
                clouds=[((250, 246, 240), 0.58, 0.85, 0.7, 3)], horizon=(205, (255, 214, 170), 0.6)),
    # dusk: violet, rose, ember
    "dusk": dict(palette=[(60, 40, 110), (230, 110, 120), (255, 180, 120), (20, 16, 40)],
                 blobs=[(260, 250, 220, 2, -3, -3, .8), (180, 190, 150, 1, 4, -2, .7), (40, 260, 120, 3, 2, 0, .6)],
                 base_top=(30, 24, 70), base_bot=(120, 60, 100),
                 clouds=[((90, 50, 110), 0.55, 0.6, 0.6, 1)], horizon=(200, (255, 170, 110), 0.55)),
    # meadow at golden hour (closing)
    "meadow": dict(palette=[(70, 90, 50), (210, 170, 70), (150, 190, 230), (40, 52, 30), (250, 225, 160)],
                   blobs=[(240, 40, 300, 2, 0, -1, .8), (100, 230, 160, 0, 2, 0, .8), (380, 240, 150, 3, -2, 0, .8),
                          (140, 120, 80, 4, 2, -1, .45)],
                   base_top=(130, 170, 210), base_bot=(50, 60, 34),
                   clouds=[((246, 242, 232), 0.6, 0.7, 0.25, 3), ((200, 160, 60), 0.62, 0.55, 0.3, 2)],
                   horizon=(150, (250, 220, 160), 0.45)),
}


def make_grid():
    im = Image.new("RGBA", (W, H), PAPER + (255,))
    d = ImageDraw.Draw(im)
    step = 48
    for x in range(0, W, step):
        d.line([(x, 0), (x, H)], fill=(234, 234, 230, 255))
    for y in range(0, H, step):
        d.line([(0, y), (W, y)], fill=(234, 234, 230, 255))
    return im


GRID = make_grid()


def rounded_mask(size, r):
    m = Image.new("L", size, 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, size[0] - 1, size[1] - 1], r, fill=255)
    return m


@lru_cache(None)
def panel(name, box, scale, radius=18, shadow=36, border=True):
    """Crop a CSS-pixel box out of a 2x screenshot, scale it, round it and give
    it a soft drop shadow. Returns (image, pad)."""
    src = Image.open(FRAMES / f"{name}.png").convert("RGBA")
    x0, y0, x1, y1 = [v * 2 for v in box]
    im = src.crop((x0, y0, x1, y1))
    w, h = int((x1 - x0) / 2 * scale), int((y1 - y0) / 2 * scale)
    im = im.resize((w, h), Image.LANCZOS)
    im.putalpha(rounded_mask((w, h), radius))
    pad = shadow * 2
    out = Image.new("RGBA", (w + pad * 2, h + pad * 2), (0, 0, 0, 0))
    sh = Image.new("L", out.size, 0)
    ImageDraw.Draw(sh).rounded_rectangle([pad, pad + 18, pad + w, pad + h + 18], radius, fill=70)
    sh = sh.filter(ImageFilter.GaussianBlur(shadow))
    out.putalpha(sh)
    out = Image.composite(out, Image.new("RGBA", out.size, (0, 0, 0, 0)), sh)
    black = Image.new("RGBA", out.size, (10, 12, 20, 0))
    black.putalpha(sh)
    out = black
    out.alpha_composite(im, (pad, pad))
    if border:
        ImageDraw.Draw(out).rounded_rectangle([pad, pad, pad + w - 1, pad + h - 1], radius, outline=(0, 0, 0, 28), width=1)
    return out, pad


def put_panel(canvas, pimg, pad, x, y, s=1.0, alpha=1.0):
    """Place panel so its content's top-left is at (x, y), scaled by s around it."""
    if s != 1.0:
        w, h = pimg.size
        pimg = pimg.resize((max(1, int(w * s)), max(1, int(h * s))), Image.BILINEAR)
        pad = pad * s
    paste_alpha(canvas, pimg, x - pad, y - pad, alpha)


def place(c, name, box, s, ox, oy, smax=None, alpha=1.0):
    """Panel of css `box` at display scale `s`, its content top-left at (ox, oy)."""
    smax = smax or s
    p, pad = panel(name, box, smax)
    put_panel(c, p, pad, ox, oy, s / smax, alpha)


def cam_place(c, name, box, states, z, smax, alpha=1.0):
    """Ease between camera states [(s, ox, oy), ...] with z in [0, len-1]."""
    j = min(len(states) - 2, int(z))
    f = z - j
    (s0, x0, y0), (s1, x1, y1) = states[j], states[j + 1]
    s = loglerp(s0, s1, f)
    # interpolate the screen position of the box centre so the zoom stays smooth
    cx0, cy0 = x0 + (box[2] - box[0]) * s0 / 2, y0 + (box[3] - box[1]) * s0 / 2
    cx1, cy1 = x1 + (box[2] - box[0]) * s1 / 2, y1 + (box[3] - box[1]) * s1 / 2
    cx, cy = lerp(cx0, cx1, f), lerp(cy0, cy1, f)
    ox, oy = cx - (box[2] - box[0]) * s / 2, cy - (box[3] - box[1]) * s / 2
    place(c, name, box, s, ox, oy, smax=smax, alpha=alpha)
    return s, ox, oy


def focus_state(box, s, fx, fy, sx, sy):
    """Camera state putting css point (fx, fy) at screen (sx, sy) at scale s."""
    return (s, sx - (fx - box[0]) * s, sy - (fy - box[1]) * s)


@lru_cache(None)
def keycap(label, size=96, pressed=False):
    pad = 30
    im = Image.new("RGBA", (size + pad * 2, size + pad * 2), (0, 0, 0, 0))
    sh = Image.new("L", im.size, 0)
    off = 4 if pressed else 12
    ImageDraw.Draw(sh).rounded_rectangle([pad, pad + off, pad + size, pad + size + off], 20, fill=110 if not pressed else 70)
    sh = sh.filter(ImageFilter.GaussianBlur(10))
    base = Image.new("RGBA", im.size, (0, 0, 0, 0))
    base.putalpha(sh)
    im = base
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([pad, pad, pad + size, pad + size], 20, fill=(38, 38, 42, 255))
    d.rounded_rectangle([pad + 3, pad + 2, pad + size - 3, pad + size - 8], 17, fill=(52, 52, 57, 255))
    f = font(FONT_MED, int(size * (0.42 if len(label) == 1 else 0.24)))
    if label in ("⌘", "↵"):
        f = ImageFont.truetype(SYMBOLS, int(size * 0.5))
    l, t, r, b = d.textbbox((0, 0), label, font=f)
    d.text((pad + (size - (r - l)) / 2 - l, pad + (size - 8 - (b - t)) / 2 - t), label, font=f, fill=(240, 240, 242, 255))
    return im, pad


def draw_keys(canvas, labels, x, y, t, t_in, t_press, size=96, gap=18, t_out=None):
    for i, lab in enumerate(labels):
        k = ease_back(ramp(t, t_in + i * 0.06, 0.4))
        pressed = t_press is not None and t_press + i * 0.03 <= t < t_press + 0.22 + i * 0.03
        im, pad = keycap(lab, size, pressed)
        a = ramp(t, t_in + i * 0.06, 0.2)
        if t_out is not None:
            a *= 1 - ramp(t, t_out, 0.25)
        dy = (1 - k) * 40 + (6 if pressed else 0)
        paste_alpha(canvas, im, x + i * (size + gap) - pad, y - pad + dy, a)


def blue_dot(d, x, y, r=7, a=255):
    d.ellipse([x - r, y - r, x + r, y + r], fill=BLUE + (int(a),))


def pointer(canvas, x, y, alpha=1.0):
    im = pointer_img()
    paste_alpha(canvas, im, x - 6, y - 4, alpha)


@lru_cache(None)
def pointer_img():
    s = 2
    im = Image.new("RGBA", (40 * s, 52 * s), (0, 0, 0, 0))
    pts = [(6, 4), (6, 38), (14, 30), (20, 44), (26, 41), (20, 28), (31, 28)]
    pts = [(x * s, y * s) for x, y in pts]
    sh = Image.new("L", im.size, 0)
    ImageDraw.Draw(sh).polygon([(x + 3, y + 5) for x, y in pts], fill=110)
    sh = sh.filter(ImageFilter.GaussianBlur(4))
    im.putalpha(sh)
    d = ImageDraw.Draw(im)
    d.polygon(pts, fill=(255, 255, 255, 255))
    inner = [(8, 9), (8, 33), (14.5, 26.5), (20.5, 40), (23.5, 38.5), (17.5, 25.5), (26, 25.5)]
    d.polygon([(x * s, y * s) for x, y in inner], fill=(15, 15, 18, 255))
    return im.resize((40, 52), Image.LANCZOS)


def ripple(canvas, x, y, t, tc):
    k = ramp(t, tc, 0.55)
    if 0 < k < 1:
        ov = Image.new("RGBA", (200, 200), (0, 0, 0, 0))
        r = 12 + 60 * ease_out(k)
        ImageDraw.Draw(ov).ellipse([100 - r, 100 - r, 100 + r, 100 + r], outline=BLUE + (int(220 * (1 - k)),), width=5)
        canvas.alpha_composite(ov, (int(x - 100), int(y - 100)))


def brackets(canvas, cx, cy, w, h, a):
    """The reference's bracket marks around a brand name."""
    d = ImageDraw.Draw(canvas)
    L = 34
    c = (255, 255, 255, int(235 * a))
    for sx in (-1, 1):
        for sy in (-1, 1):
            x, y = cx + sx * w / 2, cy + sy * h / 2
            d.rounded_rectangle([min(x, x - sx * L), y - 3, max(x, x - sx * L), y + 3], 3, fill=c)


@lru_cache(None)
def chip(label):
    f = font(FONT_MED, 30)
    tw = f.getlength(label) + 7 * len(label)
    w, h = int(tw + 96), 72
    im = Image.new("RGBA", (w + 40, h + 40), (0, 0, 0, 0))
    sh = Image.new("L", im.size, 0)
    ImageDraw.Draw(sh).rounded_rectangle([20, 26, 20 + w, 26 + h], 12, fill=40)
    im.putalpha(sh.filter(ImageFilter.GaussianBlur(8)))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([20, 20, 20 + w, 20 + h], 12, fill=(255, 255, 255, 255), outline=(225, 225, 222, 255))
    # tiny envelope glyph
    ex, ey = 44, 45
    d.rounded_rectangle([ex, ey, ex + 32, ey + 23], 4, outline=INK + (255,), width=3)
    d.line([(ex + 2, ey + 3), (ex + 16, ey + 13), (ex + 30, ey + 3)], fill=INK + (255,), width=3)
    x = ex + 48
    for ch in label:
        d.text((x, 39), ch, font=f, fill=INK + (255,))
        x += f.getlength(ch) + 7
    return im


def beat_text(c, t, l1, l2, sub=None, t0=0.15, out=None):
    words_in(c, l1, 120, 380, t, t0, FONT_DISPLAY, 76, INK + (255,), out_t=out)
    if l2:
        words_in(c, l2, 120, 470, t, t0 + 0.18, FONT_DISPLAY, 76, INK + (255,), out_t=out)
    if sub:
        words_in(c, sub, 122, 585 if l2 else 495, t, t0 + 0.5, FONT_MED, 28, GREY + (255,), stagger=0.035, rise=14,
                 out_t=out)


def draw_cloud(d, cx, cy, s, fill):
    for (dx, dy, r) in ((-0.55, 0.15, 0.42), (0.0, -0.18, 0.55), (0.55, 0.12, 0.42)):
        d.ellipse([cx + (dx - r) * s, cy + (dy - r) * s, cx + (dx + r) * s, cy + (dy + r) * s], fill=fill)
    d.rounded_rectangle([cx - 0.95 * s, cy + 0.0 * s, cx + 0.95 * s, cy + 0.55 * s], int(0.27 * s), fill=fill)


def laptop(d, x, y, w, h, a):
    d.rounded_rectangle([x, y, x + w, y + h], 22, fill=(22, 22, 26, int(255 * a)))
    d.rounded_rectangle([x + 18, y + 18, x + w - 18, y + h - 18], 12, fill=(36, 36, 42, int(255 * a)))
    d.rounded_rectangle([x - 30, y + h, x + w + 30, y + h + 18], 9, fill=(200, 200, 205, int(255 * a)))


def envelope(d, cx, cy, sz, col):
    d.rounded_rectangle([cx - sz, cy - sz * 0.7, cx + sz, cy + sz * 0.7], int(sz * 0.18),
                        fill=(255, 255, 255, col[3]), outline=col, width=4)
    d.line([(cx - sz + 4, cy - sz * 0.6), (cx, cy + sz * 0.05), (cx + sz - 4, cy - sz * 0.6)], fill=col, width=4)


@lru_cache(None)
def os_tile(name):
    w, h = 300, 120
    im = Image.new("RGBA", (w + 60, h + 60), (0, 0, 0, 0))
    sh = Image.new("L", im.size, 0)
    ImageDraw.Draw(sh).rounded_rectangle([30, 40, 30 + w, 40 + h], 20, fill=50)
    im.putalpha(sh.filter(ImageFilter.GaussianBlur(14)))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([30, 30, 30 + w, 30 + h], 20, fill=(255, 255, 255, 255), outline=(225, 225, 222, 255))
    f = font(FONT_DISPLAY, 40)
    tw = f.getlength(name)
    d.text((30 + (w - tw) / 2, 30 + 34), name, font=f, fill=INK + (255,))
    return im


@lru_cache(None)
def icon_img(size):
    return Image.open(ICON).convert("RGBA").resize((size, size), Image.LANCZOS)


def frame_at(timeline, T):
    for start, dur, fn in timeline:
        if start <= T < start + dur:
            return fn(T - start).convert("RGB")
    return Image.new("RGB", (W, H))


def run(timeline, argv=None):
    """CLI: storyboard.py OUT.mp4 renders the silent master (60 fps, CRF 14);
    storyboard.py PREFIX --preview 1.2,13.5 writes PREFIX-<t>.png stills."""
    argv = argv or sys.argv[1:]
    out = argv[0]
    if len(argv) > 2 and argv[1] == "--preview":
        for T in (float(v) for v in argv[2].split(",")):
            frame_at(timeline, T).save(f"{out}-{T:05.2f}.png")
        return
    total = timeline[-1][0] + timeline[-1][1]
    n = int(total * FPS)
    p = subprocess.Popen(["ffmpeg", "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
                          "-r", str(FPS), "-i", "-", "-c:v", "libx264", "-preset", "slow", "-crf", "14",
                          "-tune", "film", "-pix_fmt", "yuv420p", "-movflags", "+faststart", out], stdin=subprocess.PIPE)
    for i in range(n):
        p.stdin.write(frame_at(timeline, i / FPS).tobytes())
        if i % 600 == 0:
            print(f"{i}/{n}", flush=True)
    p.stdin.close()
    p.wait()
