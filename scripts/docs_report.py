#!/usr/bin/env python3
"""Render a docs-check run (results.json) as the docs themselves, coloured.

Each page of the docs is one section: a summary table (fragments per kind of
validation and colour), the recommended actions, then the page as a reader
sees it, with every fragment — sentence, table row, code block — coloured

    green   validated and correct
    yellow  not validatable, or not validated yet
    red     an error: the check fails, or the agent contradicts it

and ending in a tag per kind of validation ([APP], [CLI], [TST]…). Hovering or
focusing a tag shows how it was validated, the evidence, and what to do.

Usage: docs_report.py <results.json> <informe.html>
Relative paths in the evidence (screenshots under app/) resolve against the
run dir, where the report is written.
"""

import html
import json
import pathlib
import re
import sys

TAGS = {
    "APP": "Prueba manejando la app (instalación limpia, bloqueo, buzón demo)",
    "CLI": "Ejecutando emailops-cli",
    "TST": "Tests del código que prueban el comportamiento",
    "COD": "Contraste con código o configuración",
    "GEN": "Generado desde el código",
    "REL": "Ficheros de la release publicada",
    "AGT": "Juicio del agente sobre la evidencia",
    "MAN": "Revisión manual, o el fragmento no afirma nada verificable",
    "SIN": "Ninguna comprobación lo cubre",
}
STATE = {
    "ok": "Validado", "fail": "Falla", "label": "Solo se vio el texto, no el comportamiento",
    "fixed": "Expectativa fijada en el caso, no leída de la doc",
    "undeclared": "La comprobación pasa, pero no declara qué frases cubre", "manual": "Revisión manual",
    "none": "Sin afirmación verificable", "pending": "No ejecutado en esta pasada",
    "skip": "No observable en esta máquina", "supported": "El agente lo respalda",
    "contradicted": "El agente lo contradice", "insufficient": "Evidencia insuficiente para el agente",
}
COLOR_OF = {"ok": "green", "supported": "green", "fail": "red", "contradicted": "red"}
COLORS = ("green", "yellow", "red")
COLOR_LABEL = {"green": "Verde", "yellow": "Amarillo", "red": "Rojo"}
# Yellow states that call for work; manual and none are accepted as they are.
ACTION_ORDER = ["red", "insufficient", "label", "fixed", "undeclared", "skip", "SIN", "pending"]


def E(s):
    return html.escape(str(s), quote=True)


def inline(md):
    """Inline markdown → HTML: code, bold, italics, links (to the report's own
    page sections, since the site's relative links mean nothing here)."""
    out, pos = [], 0
    for m in re.finditer(r"`([^`]+)`", md):
        out.append(_emph(md[pos:m.start()]))
        out.append(f"<code>{E(m.group(1))}</code>")
        pos = m.end()
    out.append(_emph(md[pos:]))
    return "".join(out)


def _emph(s):
    s = E(s)
    s = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", s, flags=re.S)
    s = re.sub(r"(?<![\w*])\*(\S(?:.*?\S)?)\*(?![\w*])", r"<em>\1</em>", s, flags=re.S)

    def link(m):
        target = re.sub(r"^(\.\./)+|/?#.*$|/$", "", html.unescape(m.group(2))).strip("/") or "top"
        return f'<a href="#p-{E(target)}">{m.group(1)}</a>'

    return re.sub(r"\[([^\]]+)\]\(([^)]+)\)", link, s)


# ── pure model: ids, summary, actions ───────────────────────────────────────

def numbered(page, pi):
    n = 0
    for it in page["items"]:
        for f in it.get("fragments", []):
            n += 1
            yield f"p{pi}-{n}", it, f


def vcolor(v):
    return COLOR_OF.get(v["state"], "yellow")


def summary(page):
    rows = {}
    total = {c: 0 for c in COLORS}
    for _, _, f in numbered(page, 0):
        if f["color"] == "none":
            continue
        total[f["color"]] += 1
        seen = set()
        for v in f["validations"] or [{"tag": "SIN", "state": "uncovered"}]:
            key = (v["tag"], vcolor(v))
            if key in seen:
                continue
            seen.add(key)
            rows.setdefault(v["tag"], {c: 0 for c in COLORS})[vcolor(v)] += 1
    ordered = {t: rows[t] for t in TAGS if t in rows}
    ordered["Total"] = total
    return ordered


def excerpt(f, n=90):
    t = " ".join(re.sub(r"\*\*|`|^\|\s*|\s*\|$", "", f["text"]).split())
    return t if len(t) <= n else t[: n - 1] + "…"


def actions(page, pi=0):
    groups = {}

    def add(kind, key, text, fid):
        g = groups.setdefault((kind, key), {"severity": "red" if kind == "red" else "yellow",
                                            "kind": kind, "text": text, "targets": []})
        g["targets"].append(fid)

    for fid, it, f in numbered(page, pi):
        if f["color"] == "none":
            continue
        if f["color"] == "red":
            v = next(v for v in f["validations"] if vcolor(v) == "red")
            add("red", (it.get("claim"), v["detail"], v["fix"]),
                f"Corregir «{excerpt(f)}» — {v['detail']}. {v['fix']}".strip(), fid)
            continue
        if not f["validations"]:
            add("SIN", it.get("claim"), f"Cubrir las frases sin validar de [{it.get('claim')}]: añadir un caso que "
                f"las cite en covers, marcarlas como manuales con motivo, o dejar que las juzgue el agente.", fid)
            continue
        if f["color"] == "green":
            continue
        for v in f["validations"]:
            s = v["state"]
            if s == "pending":
                add("pending", "", "Ejecutar make docs-check ARGS=--with-app para validar contra la app.", fid)
            elif s == "insufficient":
                add(s, fid, f"«{excerpt(f)}»: {v['fix'] or STATE[s]}", fid)
            elif s in ("label", "fixed", "undeclared", "skip"):
                add(s, v["where"], f"{STATE[s]} — {v['fix'] or ''} ({v['where']})".replace(" ()", ""), fid)
    return sorted(groups.values(), key=lambda g: ACTION_ORDER.index(g["kind"]))


# ── HTML ────────────────────────────────────────────────────────────────────

def popover(tag, vs):
    parts = [f'<span class="pop" role="tooltip"><b class="ptitle">[{tag}] {E(TAGS[tag])}</b>']
    if not vs:
        parts.append('<span class="pst c-yellow">Ninguna comprobación cubre este fragmento.</span>'
                     '<span class="ph">Pasos sugeridos</span><span class="pb">Añadir un caso de app que lo cite en '
                     '<code>covers</code>, una entrada manual en claims.toml con el motivo, o dejar que lo juzgue el '
                     'agente.</span>')
    for v in vs:
        c = vcolor(v)
        parts.append(f'<span class="pst c-{c}">{E(STATE.get(v["state"], v["state"]))}</span>')
        parts.append(f'<span class="ph">Cómo se ha validado</span><span class="pb">{E(v["how"])}</span>')
        if v.get("where"):
            parts.append(f'<span class="pb where">Validado por <code>{E(v["where"])}</code></span>')
        ev = v.get("evidence", {})
        observed = ev.get("observed") or v.get("detail")
        if observed or ev.get("screen") or ev.get("shots"):
            parts.append('<span class="ph">Evidencia</span>')
            if observed:
                parts.append(f'<span class="pb">{E(observed)}</span>')
            if ev.get("screen") and c != "green":
                parts.append(f'<span class="screen">{E(ev["screen"][:700])}</span>')
            for s in ev.get("shots", [])[:1]:
                parts.append(f'<a class="shot" href="{E(s)}" target="_blank"><img loading="lazy" src="{E(s)}" alt="captura"></a>')
        if c != "green" and v.get("fix"):
            parts.append(f'<span class="ph">Pasos sugeridos</span><span class="pb">{E(v["fix"])}</span>')
    parts.append("</span>")
    return "".join(parts)


def tags_html(f):
    by = {}
    for v in f["validations"]:
        by.setdefault(v["tag"], []).append(v)
    out = []
    for t in f["tags"]:
        vs = by.get(t, [])
        c = "yellow" if not vs else ("red" if any(vcolor(v) == "red" for v in vs)
                                     else "green" if any(vcolor(v) == "green" for v in vs) else "yellow")
        out.append(f'<span class="tag c-{c}" tabindex="0">[{t}]{popover(t, vs)}</span>')
    return " ".join(out)


def fragment_html(f, fid):
    return f'<span class="frag c-{f["color"]}" id="{fid}">{inline(f["text"])} {tags_html(f)}</span>'


def cells(row):
    return [c.strip() for c in row.strip().strip("|").split("|")]


def item_html(it, ids):
    """One block: runs of text fragments become paragraphs, rows a table,
    code a <pre>; each keeps its colour and tags."""
    out, text, rows = [], [], []

    def flush_text():
        if text:
            out.append("<p>" + " ".join(text) + "</p>")
            text.clear()

    def flush_rows():
        if rows:
            out.append('<table class="doc">' + "".join(rows) + "</table>")
            rows.clear()

    for f in it["fragments"]:
        fid = ids[id(f)]
        if f["kind"] == "text":
            flush_rows()
            text.append(fragment_html(f, fid))
        elif f["kind"] == "header":
            flush_text()
            rows.append("<tr>" + "".join(f"<th>{inline(c)}</th>" for c in cells(f["text"])) + "<th></th></tr>")
        elif f["kind"] == "row":
            flush_text()
            rows.append(f'<tr class="frag c-{f["color"]}" id="{fid}">'
                        + "".join(f"<td>{inline(c)}</td>" for c in cells(f["text"]))
                        + f'<td class="tags">{tags_html(f)}</td></tr>')
        else:
            flush_text()
            flush_rows()
            body = re.sub(r"^```[^\n]*\n|\n?```$", "", f["text"])
            out.append(f'<div class="frag code c-{f["color"]}" id="{fid}"><pre><code>{E(body)}</code></pre>'
                       f'<div class="tags">{tags_html(f)}</div></div>')
    flush_text()
    flush_rows()
    return "".join(out)


def page_html(page, pi):
    ids = {id(f): fid for fid, _, f in numbered(page, pi)}
    anchor = page["page"].removesuffix(".md").replace("_index", "top")
    out = [f'<section class="page" id="p-{E(anchor)}"><h2>{E(page["title"])} <small>{E(page["page"])}</small></h2>']

    s = summary(page)
    out.append('<table class="summary"><tr><th>Validación</th>' + "".join(
        f'<th class="c-{c}">{COLOR_LABEL[c]}</th>' for c in COLORS) + "</tr>")
    for tag, counts in s.items():
        label = f'[{tag}] <span class="hint">{E(TAGS[tag])}</span>' if tag in TAGS else "<b>Total</b>"
        out.append(f"<tr><td>{label}</td>" + "".join(
            f'<td class="n{" z" if not counts[c] else ""}">{counts[c]}</td>' for c in COLORS) + "</tr>")
    out.append("</table>")

    acts = actions(page, pi)
    out.append('<div class="actions"><h3 class="ah">Acciones recomendadas</h3>')
    if acts:
        out.append("<ol>")
        for a in acts:
            links = " ".join(f'<a href="#{t}">{n}</a>' for n, t in enumerate(a["targets"][:10], 1))
            if len(a["targets"]) > 10:
                links += f' <span class="meta">y {len(a["targets"]) - 10} más</span>'
            if len(a["targets"]) > 1:
                links = f'{len(a["targets"])} fragmentos: {links}'

            out.append(f'<li class="a-{a["severity"]}">{E(a["text"])} <span class="go">→ {links}</span></li>')
        out.append("</ol>")
    else:
        out.append('<p class="none">Nada que hacer en esta página.</p>')
    out.append("</div><div class=\"doc\">")

    items = page["items"]
    i = 0
    while i < len(items):
        it = items[i]
        if it["kind"] == "heading":
            level = min(it["level"] + 1, 6)
            out.append(f"<h{level}>{inline(it['text'])}</h{level}>")
            i += 1
        elif it["kind"] == "item":
            ordered = bool(re.match(r"\d", (it["fragments"] or [{}])[0].get("prefix", "") if it["fragments"] else ""))
            tagname = "ol" if ordered else "ul"
            out.append(f"<{tagname}>")
            while i < len(items) and items[i]["kind"] == "item":
                out.append(f"<li>{item_html(items[i], ids)}</li>")
                i += 1
            out.append(f"</{tagname}>")
        elif it["kind"] == "quote":
            out.append(f"<blockquote>{item_html(it, ids)}</blockquote>")
            i += 1
        else:
            out.append(item_html(it, ids))
            i += 1
    out.append("</div></section>")
    return "".join(out)


CSS = """
:root{--bg:#fbfaf7;--fg:#1d1f23;--muted:#666b73;--line:#e2dfd8;--card:#fff;--code:#f1efe9;
--g:rgba(46,160,67,.16);--gl:#2e8b47;--y:rgba(214,160,0,.20);--yl:#9a7300;--r:rgba(220,53,69,.18);--rl:#c0392b;
--pop:#fff;--shadow:0 6px 24px rgba(0,0,0,.14)}
@media (prefers-color-scheme:dark){:root:not([data-theme=light]){--bg:#15171a;--fg:#e6e3dd;--muted:#9aa0a8;
--line:#2c3036;--card:#1c1f23;--code:#23272c;--g:rgba(63,185,80,.20);--gl:#56d364;--y:rgba(210,153,34,.22);
--yl:#e3b341;--r:rgba(248,81,73,.22);--rl:#ff7b72;--pop:#23272c;--shadow:0 6px 24px rgba(0,0,0,.5)}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.6 -apple-system,
BlinkMacSystemFont,"Segoe UI",sans-serif}main{max-width:980px;margin:0 auto;padding:24px 16px 80px}
h1{font-size:1.7rem;margin:.2em 0}.meta{color:var(--muted);font-size:.9rem}code{background:var(--code);
padding:.05em .3em;border-radius:4px;font-size:.88em}a{color:inherit}
.page{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:18px 20px;margin:28px 0}
.page>h2{margin-top:0}.page>h2 small{color:var(--muted);font-weight:400;font-size:.6em}
table{border-collapse:collapse;margin:10px 0}td,th{border:1px solid var(--line);padding:4px 9px;text-align:left;
vertical-align:top}table.summary td.n{text-align:right;font-variant-numeric:tabular-nums}td.z{color:var(--muted)}
th.c-green{color:var(--gl)}th.c-yellow{color:var(--yl)}th.c-red{color:var(--rl)}.hint{color:var(--muted);font-size:.85em}
.actions ol{padding-left:1.4em}.actions li{margin:.35em 0}.a-red::marker{color:var(--rl);font-weight:700}
.a-yellow::marker{color:var(--yl)}.go a{margin-right:.35em}.none{color:var(--muted)}
.doc{margin-top:14px;border-top:1px dashed var(--line);padding-top:6px}
.frag{border-radius:4px;padding:1px 2px;box-decoration-break:clone;-webkit-box-decoration-break:clone}
.frag.c-green{background:var(--g)}.frag.c-yellow{background:var(--y)}.frag.c-red{background:var(--r)}
tr.frag.c-green td{background:var(--g)}tr.frag.c-yellow td{background:var(--y)}tr.frag.c-red td{background:var(--r)}
.frag.code{display:block;padding:6px 8px;margin:8px 0}.frag.code pre{margin:0;white-space:pre-wrap;overflow-x:auto}
.tag{position:relative;display:inline-block;font:600 .72rem/1.2 ui-monospace,Menlo,monospace;padding:1px 3px;
border-radius:3px;cursor:help;border:1px solid currentColor;vertical-align:.08em}
.tag.c-green{color:var(--gl)}.tag.c-yellow{color:var(--yl)}.tag.c-red{color:var(--rl)}
.pop{display:none;position:absolute;z-index:20;top:1.6em;left:0;width:min(30rem,86vw);background:var(--pop);
color:var(--fg);border:1px solid var(--line);border-radius:8px;box-shadow:var(--shadow);padding:10px 12px;
font:400 .85rem/1.45 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;white-space:normal;text-align:left}
.pop.flip{left:auto;right:0}.tag:hover .pop,.tag:focus .pop,.tag:focus-within .pop{display:block}
.pop span,.pop b,.pop a{display:block}.ptitle{margin-bottom:4px}.pst{font-weight:700;margin-top:8px}
.pst.c-green{color:var(--gl)}.pst.c-yellow{color:var(--yl)}.pst.c-red{color:var(--rl)}
.ph{color:var(--muted);font-size:.78rem;text-transform:uppercase;letter-spacing:.04em;margin-top:6px}
.where{color:var(--muted)}.screen{font:.75rem/1.35 ui-monospace,Menlo,monospace;background:var(--code);
max-height:9em;overflow:auto;padding:4px 6px;border-radius:4px;margin-top:4px}
.shot img{max-width:100%;border:1px solid var(--line);border-radius:4px;margin-top:6px}
.legend{display:flex;flex-wrap:wrap;gap:6px 18px;font-size:.88rem}.sw{display:inline-block;width:1em;height:1em;
border-radius:3px;vertical-align:-.15em;margin-right:4px}.bar{display:flex;height:10px;border-radius:5px;
overflow:hidden;margin:6px 0 2px}.bar i{display:block}.toc{columns:2 16rem;padding-left:1.2em}
.guards td:first-child{white-space:nowrap}.st-ok{color:var(--gl);font-weight:700}.st-fail{color:var(--rl);font-weight:700}
@media (max-width:640px){.page{padding:14px 12px}table.doc{display:block;overflow-x:auto}}
"""

JS = """
// Keep a popover inside the viewport: open it leftwards near the right edge.
document.querySelectorAll('.tag').forEach(t=>{const f=()=>{const p=t.querySelector('.pop');p.classList.remove('flip');
const r=p.getBoundingClientRect();if(r.right>innerWidth-8)p.classList.add('flip')};t.addEventListener('mouseenter',f);
t.addEventListener('focus',f)});
"""


def render(data):
    m = data["meta"]
    pages = data["pages"]
    total = {c: 0 for c in COLORS}
    for pi, p in enumerate(pages):
        for c, n in summary(p)["Total"].items():
            total[c] += n
    n = sum(total.values()) or 1
    bar = "".join(f'<i style="width:{100 * total[c] / n:.2f}%;background:var(--{c[0]}l)"></i>' for c in COLORS)
    head = [f"<h1>{E(m['title'])}</h1>",
            f'<p class="meta">{E(m["date"])} · {E(m["branch"])}@{E(m["commit"])} · '
            f'{"con la app" if m["with_app"] else "sin la app (casos de app pendientes)"} · '
            f'{"con juicio del agente" if m.get("judged") else "sin juicio del agente"}</p>',
            f'<div class="bar">{bar}</div><p class="meta">{sum(total.values())} fragmentos — '
            + " · ".join(f"{total[c]} {COLOR_LABEL[c].lower()}s" for c in COLORS) + "</p>",
            '<div class="legend">'
            + "".join(f'<span><i class="sw" style="background:var(--{c[0]})"></i>{COLOR_LABEL[c]}: {d}</span>'
                      for c, d in (("green", "validado y correcto"), ("yellow", "no validable o aún sin validar"),
                                   ("red", "error")))
            + "</div><p class=\"meta\">Pasa el ratón por una etiqueta (o enfócala con el tabulador) para ver cómo "
              "se validó, la evidencia y qué hacer. Los casos y los arreglos propuestos los redacta un agente; los "
              "veredictos de [AGT] son su juicio, el resto los decide código determinista.</p>",
            '<div class="legend">' + "".join(f"<span><b>[{t}]</b> {E(d)}</span>" for t, d in TAGS.items()) + "</div>"]

    head.append('<h2>Guardas de estructura</h2><table class="guards"><tr><th>Guarda</th><th>Estado</th><th>Qué comprueba</th></tr>')
    for s in data["structure"]:
        extra = f"<br><b>{E(s['detail'])}</b><br>{E(s['fix'])}" if s["status"] != "ok" else ""
        head.append(f'<tr><td>{E(s["name"])}</td><td class="st-{s["status"]}">{"OK" if s["status"] == "ok" else "FALLO"}</td>'
                    f'<td>{E(s["how"])}{extra}</td></tr>')
    head.append("</table><h2>Páginas</h2><ol class=\"toc\">")
    for p in pages:
        t = summary(p)["Total"]
        anchor = p["page"].removesuffix(".md").replace("_index", "top")
        head.append(f'<li><a href="#p-{E(anchor)}">{E(p["title"])}</a> <span class="meta">'
                    f'{t["green"]} / {t["yellow"]} / {t["red"]}</span></li>')
    head.append("</ol>")

    body = "".join(page_html(p, pi) for pi, p in enumerate(pages))
    return (f'<!doctype html><html lang="es"><head><meta charset="utf-8"><meta name="viewport" '
            f'content="width=device-width,initial-scale=1"><title>Verificación de la doc</title><style>{CSS}</style>'
            f'</head><body><main>{"".join(head)}{body}</main><script>{JS}</script></body></html>')


if __name__ == "__main__":
    src, dst = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    dst.write_text(render(json.loads(src.read_text())), encoding="utf-8")
