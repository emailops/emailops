"""Storyboard of the shipped Lens short (contact data from web-form emails).

Copy this file for a new short and change the facts and the shots. Run from
the directory that holds `frames/` (the captures):

    uv run --no-project --with pillow python lens_contacts.py out.mp4
    uv run --no-project --with pillow python lens_contacts.py preview 3 12.8

Captures used (names from the capture script, CSS rects from rects.json):
f01 inbox with two web-form notifications, f02 one notification open,
f03 before clicking Lentes, f04 empty Lentes view, f05 "Nueva lente" chooser,
f07 the request typed in the chat, f09 the form the chat filled, f10 the same
form scrolled to its columns, f11 the new Lens before running, f12 running,
f13 the filled table.
"""
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import short_fx as fx  # noqa: E402
from short_fx import GREEN, RED, STAGE, V  # noqa: E402

fx.FRAMES_DIR = Path(os.environ.get("FRAMES", "frames"))

# ── Screen facts, CSS px, measured on the captures ────────────────────────
EMAIL_FIELDS = [((401, 142, 603, 159), "email"), ((343, 164, 428, 181), "teléfono"),
                ((343, 186, 472, 203), "empresa")]
INBOX_LEADS = [(262, 65, 1418, 110), (262, 695, 1418, 740)]
FORM_COLS = [(400, 176, 1020, 290), (400, 304, 1020, 418), (400, 432, 1020, 546)]
PROMPT = ["Crea una lente para la cuenta", "ulises@emailopslabs.dev que, de los",
          "correos del formulario de contacto de mi", "web, extraiga la dirección de email, el",
          "teléfono y la empresa de quien me", "escribe"]
CHAT_BOX, CHAT_TEXT = (1441, 686, 1722, 817), (1450, 695)
ROWS = [152 + 46 * i for i in range(5)]
TABLE_V = V(1052, 240, 556, 300)
TABLE6_V = V(1052, 263, 556, 346)
NEW_ROW = [(268, "25 sept 2026"), (392, "Nuevo mensaje del formulario de contacto: Marta…"),
           (783, "marta@bodegasoler.es"), (1012, "+34 633 20 71 58"), (1168, "Bodegas Soler")]
TOP = V(900, 450, 1800, 900)


def caption(fr, text, t):
    fx.text_block(fr, text, 150, t, size=64)


def shots():
    S = []

    def shot(dur, fn, srt=None):
        S.append((dur, fn, srt))

    def pov(t):
        fr = fx.base()
        p = fx.Panel(fr, (0, 520, 1080, 1000), fx.img("f01-inbox-before-row"),
                     fx.cam([(0, V(840, 420, 1160, 900)), (3.6, V(760, 400, 960, 800))], t))
        for i, b in enumerate(INBOX_LEADS):
            fx.highlight(fr, p.rect_of(*b), t - 1.1 - 0.3 * i, pad=2)
        fx.pill(fr, "POV", 60, 120, 1, RED, size=40)          # on screen from frame 0: it is the thumbnail
        fx.text_block(fr, "Tu web te manda **clientes nuevos** cada día", 210, 1, size=72)
        return fr
    shot(3.8, pov, "POV: tu web te manda clientes nuevos cada día")

    def before(t):
        fr = fx.base()
        p = fx.Panel(fr, (0, 420, 1080, 1060), fx.img("f02-email"),
                     fx.cam([(0, V(700, 300, 900, aspect=1080 / 1060)), (0.8, V(470, 170, 400, aspect=1080 / 1060))], t))
        for i, (b, lab) in enumerate(EMAIL_FIELDS):
            fx.highlight(fr, p.rect_of(*b), t - 1.0 - 0.7 * i, color=RED, label=lab)
        fx.pill(fr, "ANTES", 60, 120, t, RED, size=40)
        y = 1540
        for i, s in enumerate(["Copiar el email…", "…el teléfono…", "…la empresa…"]):
            if t > 1.0 + 0.7 * i:
                fx.text_block(fr, s, y, t - 1.0 - 0.7 * i, size=50)
                y += 64
        fx.text_block(fr, "…y así con **cada cliente.**", 200, t - 3.3, size=66, hl=RED)
        return fr
    shot(5.0, before, "Antes: copiar el email, el teléfono, la empresa… y así con cada cliente")

    def after(t):
        fr = fx.base()
        fx.pill(fr, "DESPUÉS", 60, 120, t, GREEN, size=40)
        y = fx.text_block(fr, "Una **Lente** de EmailOps", 760, t - 0.1, size=92, hl=GREEN)
        fx.text_block(fr, "La IA local lee tus correos y rellena una tabla", max(1000, y + 60), t - 0.5, size=54, color=fx.BLUE)
        return fr
    shot(2.6, after, "Después: una Lente de EmailOps. La IA local lee tus correos y rellena una tabla")

    def lentes(t):
        fr = fx.base()
        p = fx.Panel(fr, STAGE, fx.img("f03-before-lentes"), fx.cam([(0, TOP), (0.9, V(280, 470, 560))], t))
        fx.pointer(fr, p.pt(fx.path([(0.3, (700, 300)), (1.2, (117, 517))], t)), (t - 1.35) / 0.5)
        caption(fr, "Abres **Lentes**", t)
        return fr
    shot(2.0, lentes, "Abres Lentes")

    def new_lens(t):
        fr = fx.base()
        p = fx.Panel(fr, STAGE, fx.img("f04-lenses-empty"), fx.cam([(0, TOP), (0.7, V(1250, 300, 700))], t))
        fx.pointer(fr, p.pt(fx.path([(0.2, (900, 500)), (0.9, (1348, 24))], t)), (t - 1.0) / 0.5)
        caption(fr, "Nueva lente, **con el chat**", t)
        return fr
    shot(1.5, new_lens, "Nueva lente, con el chat")

    def chooser(t):
        fr = fx.base()
        p = fx.Panel(fr, STAGE, fx.img("f05-chooser"), fx.cam([(0, V(1250, 300, 700)), (0.6, V(900, 540, 620))], t))
        fx.pointer(fr, p.pt(fx.path([(0.1, (1348, 24)), (0.9, (798, 482))], t)), (t - 1.05) / 0.5)
        caption(fr, "Nueva lente, **con el chat**", 1)
        return fr
    shot(1.6, chooser)

    def ask(t):
        fr = fx.base()
        p = fx.Panel(fr, STAGE, fx.img("f07-typed"), fx.cam([(0, V(1580, 700, 520)), (4.3, V(1600, 740, 420))], t))
        n = int(fx.total_chars(PROMPT) * min(1, max(0, (t - 0.3) / 3.8)))
        if n < fx.total_chars(PROMPT):
            fx.typing(fr, p, CHAT_BOX, CHAT_TEXT, PROMPT, n)
        fx.pointer(fr, p.pt(fx.path([(4.3, (1600, 640)), (5.0, (1761, 806))], t)), (t - 5.2) / 0.5)
        caption(fr, "Le pides lo que **quieres sacar**", t)
        return fr
    shot(5.9, ask, "Le pides lo que quieres sacar")

    def form(t):
        fr = fx.base()
        name = "f09-form-filled" if t < 2.4 else "f10-columns"
        p = fx.Panel(fr, STAGE, fx.img(name),
                     fx.cam([(0, V(1600, 300, 480)), (1.0, V(1600, 300, 480)), (1.9, V(710, 450, 760)),
                             (2.6, V(710, 500, 700))], t))
        if name == "f10-columns":
            for i, b in enumerate(FORM_COLS):
                fx.highlight(fr, p.rect_of(*b), t - 2.6 - 0.25 * i, color=GREEN, pad=4)
            fx.pointer(fr, p.pt(fx.path([(3.4, (900, 600)), (4.0, (953, 817))], t)), (t - 4.2) / 0.5)
        if t < 2.4:
            caption(fr, "La IA prepara la **Lente**", t)
        else:
            caption(fr, "Email, teléfono, empresa: **Crear**", t - 2.4)
        return fr
    shot(4.8, form, "La IA prepara la Lente. Email, teléfono, empresa: crear")

    def run(t):
        fr = fx.base()
        p = fx.Panel(fr, STAGE, fx.img("f11-lens-created" if t < 1.5 else "f12-running"),
                     fx.cam([(0, V(900, 300, 900)), (0.6, V(969, 280, 620))], t))
        fx.pointer(fr, p.pt(fx.path([(0.1, (953, 600)), (0.9, (969, 31))], t)), (t - 1.1) / 0.5)
        caption(fr, "Y la **ejecutas**", t)
        return fr
    shot(2.2, run, "Y la ejecutas")

    def split(t):
        fr = fx.base()
        top = fx.Panel(fr, (0, 300, 1080, 700), fx.img("f02-email"), V(470, 172, 440, aspect=1080 / 700))
        for b, lab in EMAIL_FIELDS:
            fx.highlight(fr, top.rect_of(*b), 1, color=RED, label=lab)
        bottom = fx.Panel(fr, (0, 1060, 1080, 760), fx.img("f13-table"),
                          fx.cam([(0, V(1060, 262, 620, 440)), (2.0, TABLE_V)], t))
        fx.table_reveal(fr, bottom, t, 0.5, ROWS, 46, 772, 1418, step=0.3)
        fx.draw(fr).line((60, 1030, 1020, 1030), fill=(51, 65, 85, 255), width=3)
        fx.pill(fr, "ANTES", 60, 320, t, RED, size=34)
        fx.pill(fr, "DESPUÉS, con IA local", 60, 1080, t - 0.3, GREEN, size=34)
        fx.text_block(fr, "De 5 correos a **1 tabla**", 120, t - 0.2, size=84, hl=GREEN)
        return fr
    shot(6.0, split, "De 5 correos a 1 tabla")

    def new_mail(t):
        fr = fx.base()
        table = fx.insert_row(fx.img("f13-table"), (t - 0.8) / 0.7, (256, 152, 1418, 5), 46, NEW_ROW)
        p = fx.Panel(fr, (0, 560, 1080, 900), table, fx.cam([(0, TABLE_V), (0.7, TABLE6_V)], t))
        fx.highlight(fr, p.rect_of(776, 154, 1330, 196), t - 1.7, color=GREEN, label="nuevo", pad=4)
        fx.text_block(fr, "¿Llega otro cliente?", 230, t, size=80)
        fx.text_block(fr, "Su fila aparece **sola.**", 340, t - 1.6, size=80, hl=GREEN)
        return fr
    shot(4.4, new_mail, "¿Llega otro cliente? Su fila aparece sola.")

    shot(3.8, fx.end_card, "EmailOps. Cliente de correo libre con IA local. getemailops.com")
    return S


if __name__ == "__main__":
    fx.main(shots(), sys.argv[1:])
