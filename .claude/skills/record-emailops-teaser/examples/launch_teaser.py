"""The EmailOps launch teaser (v8, 74 s): the worked example for this skill.

    TEASER_FRAMES=<work dir> uv run --no-project --with pillow --with numpy \
        python examples/launch_teaser.py out/silent.mp4            # full render
    ... python examples/launch_teaser.py pv/f --preview 12,16.8    # stills

Frame folders (relative to TEASER_FRAMES) and what each holds; every capture
comes from the synthetic demo (or a scratch copy of it):
  frames-app/  k01-palette, k02-palette-invoice (⌘K palette, "invoice")
  frames4/     i00-empty, i01-type-NNN, i03-gen-001 (AI Draft idea typed, Generating…)
  frames5/     r-draft, s01-send-visible, v00-inbox, v01..v04 views (Tag Board, Attachments, Calendar)
  frames6/     t21-detected, t25-translated, sidebar-strip.png (stitched SMART FILTERS)
  frames7/     e01b-before-share, e02-share-filled (long EO Doc + share dialog)
  frames10/    u00-empty-chat, u01-type-NNN, u02-wait-NNN, u03-answer (All accounts + chat)
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from teaser_fx import *  # noqa: F401,F403
import teaser_fx as fx  # noqa: E402

FRAMES = fx.FRAMES
FA, F4, F5, F6, F7, F10 = "frames-app/", "frames4/", "frames5/", "frames6/", "frames7/", "frames10/"

CHIPS = [("WORK", "ulises@emailopslabs.dev", 360, 300), ("PERSONAL", "ulises@fastmail.com", 1380, 250),
         ("SIDE PROJECT", "", 820, 760)]


DOTS = [(220, 520), (610, 180), (1120, 420), (1640, 560), (1500, 860), (980, 960), (300, 880), (1760, 200)]


HEAD_DOTS = [(560, 420), (1360, 400), (590, 660), (1340, 680)]


OPEN_LINES = [("agate", "Your inbox knows", "everything about you."),
              ("ocean", "Your clients. Your invoices.", "Your family."),
              ("dusk", "Why send it to", "someone else's AI?")]


OPEN_D = 2.0


def sc_open(t):
    i = min(2, int(t / OPEN_D))
    lt = t - i * OPEN_D
    name, l1, l2 = OPEN_LINES[i]
    c = field(lt + i * 0.4, **FIELDS[name])
    c.alpha_composite(new_img((W, H), (0, 0, 0, 115 if name == "agate" else 55)))
    white = (255, 255, 255, 255)
    words_in(c, l1, W / 2, 430, lt, 0.1, FONT_DISPLAY, 84, white, align="center", stagger=0.08)
    words_in(c, l2, W / 2, 535, lt, 0.35, FONT_DISPLAY, 84, white, align="center", stagger=0.08)
    return c


def sc_grid(t):
    """3.6 -> 8.2: chips + dots, converge, headline."""
    c = GRID.copy()
    d = draw(c)
    conv = ease_io(ramp(t, 1.25, 0.75))  # chips & dots fly to centre
    for i, (lab, addr, x, y) in enumerate(CHIPS):
        k = ease_back(ramp(t, 0.1 + i * 0.18, 0.45))
        im = chip(lab)
        cx, cy = lerp(x, W / 2, conv), lerp(y, H / 2, conv)
        a = ramp(t, 0.1 + i * 0.18, 0.2) * (1 - ramp(t, 1.6, 0.35))
        s = lerp(0.7, 1, k) * lerp(1, 0.6, conv)
        if s != 1:
            im = im.resize((int(im.width * s), int(im.height * s)), Image.LANCZOS)
        paste_alpha(c, im, cx - lw(im) / 2, cy - lh(im) / 2, a)
        mono = text_img(addr or " ", FONT_REG, 24, (140, 140, 146, 255))
        paste_alpha(c, mono, x - lw(mono) / 2, y - 84, a * (1 - conv))
    for j, (x, y) in enumerate(DOTS):
        a = 255 * ramp(t, 0.3 + j * 0.05, 0.2)
        if t < 2.0:
            px, py = lerp(x, W / 2, conv), lerp(y, H / 2, conv)
            blue_dot(d, px, py, 6, a * (1 - ramp(t, 1.85, 0.15)))
    # dots burst from centre into the four headline corners
    k = ease_out(ramp(t, 1.95, 0.55))
    for (x, y) in HEAD_DOTS:
        if t >= 1.95:
            blue_dot(d, lerp(W / 2, x, k), lerp(H / 2, y, k), 9, 255 * (1 - ramp(t, 4.3, 0.3)))
    words_in(c, "Every account. One inbox.", W / 2, H / 2 - 52, t, 2.1, FONT_DISPLAY, 92, INK + (255,),
             align="center", stagger=0.09, out_t=4.3)
    return c


UBOX = (0, 0, 1800, 875)


U_WIDE = (0.95, W / 2 - 1800 * 0.95 / 2, 540 - 875 * 0.95 / 2)


def _u_chat():
    return focus_state(UBOX, 1.35, 1610, 430, 1330, 540)


def sc_window(t):
    """The unified inbox, "All accounts" selected, the chat panel open and empty."""
    c = GRID.copy()
    k = ease_out(ramp(t, 0.0, 0.8))
    wide = (U_WIDE[0], U_WIDE[1], U_WIDE[2] + (1 - k) * 600)
    s, ox, oy = cam_place(c, F10 + "u00-empty-chat", UBOX, [wide, wide], 0, smax=1.35, alpha=ramp(t, 0, 0.3))
    a = ramp(t, 0.9, 0.3)
    if a > 0:
        X0, Y0 = ox + 10 * s, oy + 262 * s
        X1, Y1 = ox + 246 * s, oy + 305 * s
        pulse = 1 + 0.06 * math.sin(max(0, t - 0.9) * 8)
        cx, cy = (X0 + X1) / 2, (Y0 + Y1) / 2
        hw, hh = (X1 - X0) / 2 * pulse + 8, (Y1 - Y0) / 2 * pulse + 8
        ov = new_img((W, H), (0, 0, 0, 0))
        dd = draw(ov)
        dd.rounded_rectangle([cx - hw, cy - hh, cx + hw, cy + hh], 12, outline=BLUE + (int(255 * a),), width=4)
        lab = text_img("Unified inbox", FONT_MED, 26, (255, 255, 255, 255))
        pw = lw(lab) + 32
        dd.rounded_rectangle([cx - pw / 2, cy - hh - 58, cx + pw / 2, cy - hh - 14], 22, fill=BLUE + (int(255 * a),))
        c.alpha_composite(ov)
        paste_alpha(c, lab, cx - lw(lab) / 2, cy - hh - 53, a)
    return c


def sc_chat(t):
    """Start on the unified inbox, push onto the chat, type the question, the answer streams in."""
    c = GRID.copy()
    z = ease_io(ramp(t, 0.3, 1.1))
    if t < 1.6:
        name = "u00-empty-chat"
    elif t < 3.9:
        n = 1 + int(clamp((t - 1.6) / 2.2) * 30)
        name = f"u01-type-{n:03d}"
    elif t < 5.6:
        n = 1 + int(clamp((t - 3.9) / 1.7) * 16)
        name = f"u02-wait-{n:03d}"
    else:
        name = "u03-answer"
    cam_place(c, F10 + name, UBOX, [U_WIDE, _u_chat()], z, smax=1.35)
    if z > 0:
        left = grid_strip(700)
        left.putalpha(int(255 * z))
        c.alpha_composite(left)
    beat_text(c, t, "Ask your", "inbox.", "Answers link the emails they came from.", t0=1.2)
    draw_keys(c, ["↵"], 130, 760, t, 3.3, 3.95)
    return c


def sc_privacy(t):
    """All AI runs on your computer; nothing goes to a cloud AI."""
    c = GRID.copy()
    beat_text(c, t, "All AI runs on", "your computer.", "No email ever travels to an AI in the cloud.")
    note = text_img("Remote models are off unless you turn them on.", FONT_REG, 22, (150, 150, 156, 255))
    paste_alpha(c, note, 122, 632, ramp(t, 1.0, 0.4))
    ov = new_img((W, H), (0, 0, 0, 0))
    d = draw(ov)
    k = ease_back(ramp(t, 0.3, 0.55), 1.1)
    a = ramp(t, 0.3, 0.25)
    # the computer: a dark card
    X, Y, BW, BH = 1000, 330, 700, 420
    Y += (1 - k) * 40
    sh = new_img((W, H), 0, "L")
    draw(sh).rounded_rectangle([X, Y + 18, X + BW, Y + BH + 18], 28, fill=int(80 * a))
    c.alpha_composite(Image.merge("RGBA", (new_img((W, H), 10, "L"),) * 3 + (sh.filter(blur(26)),)))
    d.rounded_rectangle([X, Y, X + BW, Y + BH], 28, fill=(22, 22, 26, int(255 * a)))
    lab = text_img("Your computer", FONT_MED, 26, (150, 150, 158, 255))
    # envelope
    ex, ey = X + 110, Y + 200
    d.rounded_rectangle([ex - 60, ey - 42, ex + 60, ey + 42], 10, outline=(245, 245, 247, int(255 * a)), width=5)
    d.line([(ex - 56, ey - 36), (ex, ey + 6), (ex + 56, ey - 36)], fill=(245, 245, 247, int(255 * a)), width=5)
    # local model chip
    mx, my = X + BW - 170, Y + 200
    d.rounded_rectangle([mx - 80, my - 80, mx + 80, my + 80], 22, outline=(52, 199, 89, int(255 * a)), width=5)
    for i in range(4):
        o = -48 + i * 32
        for (x0, y0, x1, y1) in ((mx + o, my - 98, mx + o, my - 82), (mx + o, my + 82, mx + o, my + 98),
                                 (mx - 98, my + o, mx - 82, my + o), (mx + 82, my + o, mx + 98, my + o)):
            d.line([(x0, y0), (x1, y1)], fill=(52, 199, 89, int(255 * a)), width=5)
    chipl = text_img("AI", FONT_DISPLAY, 54, (52, 199, 89, 255))
    # data flowing envelope -> chip
    for j in range(5):
        ph = (t * 0.9 + j / 5) % 1
        if t > 0.9:
            px = lerp(ex + 80, mx - 100, ph)
            d.ellipse([px - 7, my - 7, px + 7, my + 7], fill=BLUE + (int(255 * a * math.sin(ph * math.pi)),))
    # the cloud, outside, with a broken dashed link and a slash
    cx, cy = 1560, 190
    ca = a * ramp(t, 1.1, 0.4)
    draw_cloud(d, cx, cy, 70, (205, 205, 210, int(255 * ca)))
    for i in range(6):
        y0 = Y - 10 - i * 22
        if i in (2, 3):
            continue
        d.line([(1500 - i * 6, y0), (1500 - i * 6 - 4, y0 - 12)], fill=(190, 190, 196, int(255 * ca)), width=4)
    sl = ease_out(ramp(t, 1.5, 0.35))
    if sl > 0:
        d.line([(cx - 95, cy + 70), (lerp(cx - 95, cx + 95, sl), lerp(cy + 70, cy - 70, sl))], fill=(225, 60, 60, 255), width=9)
    c.alpha_composite(ov)
    paste_alpha(c, lab, X + 40, Y + 32, a)
    paste_alpha(c, chipl, mx - lw(chipl) / 2, my - 36, a)
    nolab = text_img("Cloud AI", FONT_MED, 24, (160, 160, 166, 255))
    paste_alpha(c, nolab, cx - lw(nolab) / 2, cy + 70, ca)
    return c


def sc_classify2(t):
    """Wide on the inbox, then zoom onto the AI tags (intent / topic), then the company badges."""
    c = GRID.copy()
    box = (560, 124, 1620, 708)
    wide = (1.0, 790, 560 - (box[3] - box[1]) / 2)
    tags = focus_state(box, 2.0, 1535, 250, 1300, 520)
    comp = focus_state(box, 2.0, 640, 250, 1300, 520)
    k = ease_out(ramp(t, 0.05, 0.6))
    z = ease_io(ramp(t, 1.0, 1.0)) + ease_io(ramp(t, 2.6, 0.9))
    s, ox, oy = cam_place(c, F5 + "v00-inbox", box, [(wide[0], wide[1] + (1 - k) * 140, wide[2]), tags, comp], z,
                          smax=2.0, alpha=k)
    # highlight the column the camera is on: AI tags around z=1, companies around z=2
    for (x0, x1, zc, label) in ((1462, 1608, 1.0, "intent · topic"), (568, 680, 2.0, "company")):
        a = clamp(1 - abs(z - zc) / 0.35)
        if a <= 0:
            continue
        X0, X1 = ox + (x0 - box[0]) * s, ox + (x1 - box[0]) * s
        Y0, Y1 = max(oy, 0) + 6, min(oy + (box[3] - box[1]) * s, H) - 6
        ov = new_img((W, H), (0, 0, 0, 0))
        dd = draw(ov)
        dd.rounded_rectangle([X0, Y0, X1, Y1], 18, fill=BLUE + (int(30 * a),), outline=BLUE + (int(235 * a),), width=4)
        lab = text_img(label, FONT_MED, 30, (255, 255, 255, 255))
        pw = lw(lab) + 36
        PY = Y1 - 70
        dd.rounded_rectangle([(X0 + X1) / 2 - pw / 2, PY, (X0 + X1) / 2 + pw / 2, PY + 48], 24,
                             fill=BLUE + (int(255 * a),))
        c.alpha_composite(ov)
        paste_alpha(c, lab, (X0 + X1) / 2 - lw(lab) / 2, PY + 6, a)
    # keep the words on clean paper while the zoomed panel fills the frame
    a = min(1.0, z)
    if a > 0:
        left = grid_strip(680)
        left.putalpha(int(255 * a))
        c.alpha_composite(left)
    beat_text(c, t, "Classified", "by AI.", "Company, intent and topic.")
    return c


@lru_cache(None)
def strip_img(scale):
    """The stitched sidebar strip (2x CSS) at `scale` logical px per CSS px, physical size."""
    im = Image.open(FRAMES / (F6 + "sidebar-strip.png")).convert("RGBA")
    return im.resize((P(im.width / 2 * scale), P(im.height / 2 * scale)), Image.LANCZOS)


def sc_sidetags(t):
    c = GRID.copy()
    beat_text(c, t, "Filter by", "any tag.", "Every tag becomes a filter in the sidebar.")
    s = 1.8
    st = strip_img(s)
    vw, vh = lw(st), 800
    k = ease_out(ramp(t, 0.05, 0.6))
    y = lerp(200, 1180 - vh / s, ease_io(ramp(t, 0.7, 2.6))) * s
    view = st.crop((0, P(y), st.width, P(y) + P(vh)))
    view.putalpha(rounded_mask(view.size, 20))
    pad = 60
    sh = new_img((vw + pad * 2, vh + pad * 2), 0, "L")
    draw(sh).rounded_rectangle([pad, pad + 18, pad + vw, pad + vh + 18], 20, fill=90)
    black = new_img((vw + pad * 2, vh + pad * 2), (10, 12, 20, 0))
    black.putalpha(sh.filter(blur(30)))
    black.alpha_composite(view, (P(pad), P(pad)))
    paste_alpha(c, black, 1120 - pad + (1 - k) * 120, 540 - vh / 2 - pad, k)
    return c


DBOX = (270, 0, 1760, 520)


def sc_idea(t):
    """Type the idea -> Generate with AI -> the draft."""
    c = GRID.copy()
    beat_text(c, t, "Type the idea.", "Get the reply.", "Drafted from the thread, locally.")
    sA, oA = 1.5, (668 - 0, 520 - 195 * 1.5)
    sB, oB = 0.84, (700, 250)
    z = ease_io(ramp(t, 3.0, 0.8))
    s = loglerp(sA, sB, z)
    k = ease_out(ramp(t, 0.0, 0.6))
    ox, oy = lerp(oA[0], oB[0], z) + (1 - k) * 160, lerp(oA[1], oB[1], z)
    if t < 4.05:
        n = int(clamp((t - 0.6) / 2.3) * 31)
        name = F4 + ("i00-empty" if n == 0 else f"i01-type-{n:03d}")
        place(c, name, DBOX, s, ox, oy, smax=1.5, alpha=k)
    else:
        place(c, F4 + "i03-gen-001", DBOX, s, ox, oy, smax=1.5)
        place(c, F5 + "r-draft", DBOX, s, ox, oy, smax=1.5, alpha=ramp(t, 5.0, 0.25))
    # pointer to "Generate with AI" (css 1660, 195)
    tx, ty = oB[0] + (1660 - DBOX[0]) * sB, oB[1] + (195 - DBOX[1]) * sB
    m = ease_io(ramp(t, 3.4, 0.6))
    pa = ramp(t, 3.3, 0.2) * (1 - ramp(t, 4.6, 0.3))
    ripple(c, tx, ty, t, 4.0)
    pointer(c, lerp(1500, tx, m), lerp(900, ty, m), pa)
    return c


def sc_translate(t):
    c = GRID.copy()
    beat_text(c, t, "Read any", "language.", "Translated by the AI on your computer.")
    s = 1.0
    k = ease_out(ramp(t, 0.05, 0.6))
    ox, oy = 690 + (1 - k) * 140, 340
    place(c, F6 + "t21-detected", TBOX, s, ox, oy, alpha=k)
    place(c, F6 + "t25-translated", TBOX, s, ox, oy, alpha=ramp(t, 2.0, 0.3))
    tx, ty = ox + (483 - TBOX[0]) * s, oy + (158 - TBOX[1]) * s
    m = ease_io(ramp(t, 0.7, 0.7))
    pa = ramp(t, 0.6, 0.2) * (1 - ramp(t, 2.3, 0.3))
    ripple(c, tx, ty, t, 1.5)
    pointer(c, lerp(1500, tx, m), lerp(900, ty, m), pa)
    return c


TBOX = (256, 0, 1500, 390)


def sc_search2(t):
    c = GRID.copy()
    beat_text(c, t, "Find anything.", "Instantly.", "Everything is indexed on your computer.")
    draw_keys(c, ["⌘", "K"], 130, 760, t, 0.3, 0.9)
    s = 1.3
    k = ease_back(ramp(t, 0.95, 0.45), 1.1)
    ss = lerp(0.92, 1.0, k)
    cx, top = 1290, 230
    for n, box, t0, t1 in ((FA + "k01-palette", (560, 84, 1240, 304), 0.95, 1.55),
                           (FA + "k02-palette-invoice", (560, 84, 1240, 576), 1.35, 99)):
        p, pad = panel(n, box, s)
        w = (box[2] - box[0]) * s * ss
        a = ramp(t, t0, 0.12) * (1 - ramp(t, t1, 0.12))
        put_panel(c, p, pad, cx - w / 2, top, ss, a)
    return c


VIEWS = [(F5 + "v01-tag-company", "Tag Board · company"), (F5 + "v02-tag-intent", "Tag Board · intent"),
         (F5 + "v02-tag-topic", "Tag Board · topic"), (F5 + "v03-attachments", "Attachments"),
         (F5 + "v04-calendar", "Calendar")]


def sc_views(t):
    c = GRID.copy()
    beat_text(c, t, "Different views", "of your inbox.", "Tag Board, attachments, calendar.")
    box = (256, 0, 1800, 820)
    s = 0.9
    k = ease_out(ramp(t, 0.05, 0.6))
    per = 1.0
    i = min(len(VIEWS) - 1, int(max(0, t - 0.7) / per))
    lt = max(0, t - 0.7) - i * per
    ox, oy = 790 + (1 - k) * 160, 170
    if i > 0 and lt < 0.25:
        place(c, VIEWS[i - 1][0], box, s, ox, oy)
    a = k if i == 0 else ramp(lt, 0, 0.25)
    sl = (1 - ease_out(ramp(lt, 0, 0.35))) * 60 if i > 0 else 0
    place(c, VIEWS[i][0], box, s, ox + sl, oy, alpha=a)
    lab = text_img(VIEWS[i][1], FONT_MED, 24, BLUE + (255,))
    paste_alpha(c, lab, ox, oy - 48, k * (ramp(lt, 0, 0.2) if i > 0 else 1))
    return c


DOCBOX = (545, 0, 1759, 545)


DLGBOX = (676, 270, 1124, 640)


def sc_docs(t):
    c = GRID.copy()
    s = 0.95
    k = ease_out(ramp(t, 0.05, 0.6))
    ox, oy = 760 + (1 - k) * 140, 270 - lerp(0, 40, ease_io(ramp(t, 0.6, 2.0)))
    place(c, F7 + "e01b-before-share", DOCBOX, s, ox, oy, alpha=k)
    beat_text(c, t, "Shared docs.", "No cloud.", "Edit together. Changes travel by email.")
    tx, ty = ox + (1414 - DOCBOX[0]) * s, oy + (26 - DOCBOX[1]) * s
    m = ease_io(ramp(t, 1.7, 0.7))
    pa = ramp(t, 1.6, 0.2) * (1 - ramp(t, 2.8, 0.3))
    ripple(c, tx, ty, t, 2.5)
    pointer(c, lerp(1500, tx, m), lerp(950, ty, m), pa)
    dk = ease_back(ramp(t, 2.7, 0.45), 1.1)
    if t > 2.7:
        ds = 1.05 * lerp(0.92, 1.0, dk)
        w, h = (DLGBOX[2] - DLGBOX[0]) * ds, (DLGBOX[3] - DLGBOX[1]) * ds
        place(c, F7 + "e02-share-filled", DLGBOX, ds, 1590 - w / 2, 600 - h / 2, smax=1.05, alpha=ramp(t, 2.7, 0.2))
    return c


def sc_sync(t):
    """Shared docs sync through your own email; nothing is kept in a cloud."""
    c = GRID.copy()
    beat_text(c, t, "Synced by email.", "Not by a cloud.", "Shared docs sync through your own mail account.")
    note = text_img("No EmailOps servers. No third-party storage.", FONT_REG, 24, (140, 140, 146, 255))
    paste_alpha(c, note, 122, 636, ramp(t, 1.0, 0.4))
    a = ramp(t, 0.3, 0.3)
    k = ease_out(ramp(t, 0.3, 0.6))
    ov = new_img((W, H), (0, 0, 0, 0))
    d = draw(ov)
    L = (940 - (1 - k) * 60, 560, 300, 200)
    R = (1500 + (1 - k) * 60, 560, 300, 200)
    laptop(d, *L, a)
    laptop(d, *R, a)
    xa, xb = L[0] + L[2] / 2, R[0] + R[2] / 2

    def arc(u):
        return lerp(xa, xb, u), 540 - math.sin(u * math.pi) * 170

    pts = [arc(i / 40) for i in range(41)]
    for i in range(0, 40, 2):
        d.line([pts[i], pts[i + 1]], fill=(150, 160, 175, int(220 * a)), width=4)
    if t > 0.9:
        for j in range(3):
            for direction in (1, -1):
                ph = (t * 0.45 + j / 3 + (0.5 if direction < 0 else 0)) % 1
                x, y = arc(ph if direction > 0 else 1 - ph)
                envelope(d, x, y, 26, BLUE + (int(255 * a * math.sin(ph * math.pi)),))
    cx, cy = (xa + xb) / 2, 230
    ca = a * ramp(t, 1.2, 0.4)
    draw_cloud(d, cx, cy, 64, (205, 205, 210, int(255 * ca)))
    sl = ease_out(ramp(t, 1.6, 0.35))
    if sl > 0:
        d.line([(cx - 90, cy + 66), (lerp(cx - 90, cx + 90, sl), lerp(cy + 66, cy - 66, sl))],
               fill=(225, 60, 60, 255), width=9)
    c.alpha_composite(ov)
    for (x, y, w, h), name in ((L, "You"), (R, "Your teammate")):
        lab = text_img(name, FONT_MED, 26, INK + (255,))
        paste_alpha(c, lab, x + w / 2 - lw(lab) / 2, y + h + 34, a)
        doc = text_img("EO Doc", FONT_MED, 24, (200, 200, 206, 255))
        paste_alpha(c, doc, x + w / 2 - lw(doc) / 2, y + h / 2 - 14, a)
    mail = text_img("your email", FONT_MED, 24, BLUE + (255,))
    paste_alpha(c, mail, (xa + xb) / 2 - lw(mail) / 2, 450, ramp(t, 1.0, 0.4))
    return c


def sc_platforms(t):
    c = GRID.copy()
    beat_text(c, t, "Runs on Mac,", "Windows or Linux.*", "Free and open source. Apache-2.0.")
    for i, n in enumerate(("macOS", "Windows", "Linux")):
        k = ease_back(ramp(t, 0.5 + i * 0.15, 0.45), 1.3)
        im = os_tile(n)
        paste_alpha(c, im, 1080 + (1 - k) * 60, 300 + i * 160, ramp(t, 0.5 + i * 0.15, 0.2))
    for j, line in enumerate(("*Built-in AI on Windows and Linux needs a GPU (Vulkan, or NVIDIA CUDA) to run at full speed.",
                              "On Mac it needs Apple Silicon; Intel Macs can use Ollama.")):
        foot = text_img(line, FONT_REG, 24, (140, 140, 146, 255))
        paste_alpha(c, foot, 122, 950 + j * 36, ramp(t, 1.2 + j * 0.15, 0.4))
    return c


def sc_end(t):
    """31.8 -> 37.0"""
    c = field(t + 2, **{**FIELDS["meadow"], "push": 0.025})
    # soft darkening so white type reads
    c.alpha_composite(new_img((W, H), (0, 0, 0, 70)))
    k = ease_out(ramp(t, 0.25, 0.8))
    ic = icon_img(118)
    word = text_img("EmailOps", FONT_DISPLAY, 112, (255, 255, 255, 255))
    total = lw(ic) + 26 + lw(word)
    x = W / 2 - total / 2
    y = 420 + (1 - k) * 24
    paste_alpha(c, ic, x, y - 4, k)
    paste_alpha(c, word, x + lw(ic) + 26, y - 6, k)
    words_in(c, "Your email, understood. Privately.", W / 2, 580, t, 0.9, FONT_MED, 40, (255, 255, 255, 255),
             align="center", stagger=0.07, rise=12)
    pill = text_img("Local AI  ·  Private  ·  Free and open source", FONT_MED, 32, (255, 255, 255, 255))
    pa = ramp(t, 1.5, 0.5)
    pw, ph = lw(pill) + 56, 64
    ov = new_img((W, H), (0, 0, 0, 0))
    draw(ov).rounded_rectangle([W / 2 - pw / 2, 650, W / 2 + pw / 2, 650 + ph], 32,
                                         fill=(255, 255, 255, int(38 * pa)), outline=(255, 255, 255, int(120 * pa)), width=2)
    c.alpha_composite(ov)
    paste_alpha(c, pill, W / 2 - lw(pill) / 2, 662, pa)
    url = text_img("getemailops.com", FONT_MED, 28, (235, 235, 235, 255))
    paste_alpha(c, url, W / 2 - lw(url) / 2, 760, ramp(t, 2.0, 0.5) * 0.9)
    foot = text_img("macOS · Windows · Linux", FONT_REG, 24, (225, 225, 225, 255))
    paste_alpha(c, foot, W / 2 - lw(foot) / 2, 980, ramp(t, 2.3, 0.5) * 0.8)
    # final fade
    f = ramp(t, 4.6, 0.8)
    if f > 0:
        c.alpha_composite(new_img((W, H), (0, 0, 0, int(255 * f))))
    return c


TIMELINE = [
    (0.0, 6.0, sc_open),
    (6.0, 4.6, sc_grid),
    (10.6, 3.4, sc_window),
    (14.0, 7.4, sc_chat),
    (21.4, 4.4, sc_privacy),
    (25.8, 4.4, sc_classify2),
    (30.2, 3.8, sc_sidetags),
    (34.0, 6.8, sc_idea),
    (40.8, 4.4, sc_translate),
    (45.2, 3.2, sc_search2),
    (48.4, 5.6, sc_views),
    (54.0, 5.6, sc_docs),
    (59.6, 4.8, sc_sync),
    (64.4, 4.2, sc_platforms),
    (68.6, 5.4, sc_end),
]


if __name__ == "__main__":
    run(TIMELINE)
