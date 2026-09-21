#!/usr/bin/env python3
"""Render a verify_all.py run as one self-contained HTML report.

    report_all.py <run>/results.json [out.html]

Index → global summary → one section per feature (summary table by test type,
then a subsection per type). Failures expand inline with their evidence: trace,
screenshots, log tail, and for evals the question, the answer and the engine
trace in a collapsible block. A previous *-full run, when present, gives the
delta (newly failing / newly passing).
"""
import base64, html, json, pathlib, re, sys, collections

src = pathlib.Path(sys.argv[1]); run = src.parent
out = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else run / "informe.html"
data = json.loads(src.read_text())
meta, layers, records = data["meta"], data["layers"], data["records"]
features = data["features"]; types = data["types"]
def E(s): return html.escape(str(s)).replace("\ufffd", "&#xFFFD;")  # numbers and None land here too; a model's garbled byte stays visible as an entity
TYPE_HINT = {"unit": "Funciones puras y componentes aislados: cargo test (lib) y vitest", "integration": "src-tauri/tests/integration.rs contra FakeEmailProvider y BD en memoria", "contract": "Paridad de esquema, sobre JSON de la CLI, paridad i18n, serialización", "e2e": "Barrida WebDriver sobre la app real con BD demo (sweep.mjs)", "ui": "Medidas de layout y controles nativos en la app real", "oracle": "UI ↔ backend ↔ SQL sobre la BD demo (tagboard_check.mjs)", "eval": "Casos de chat con el modelo local, validados por métricas heurísticas", "doc": "Promesas de docs/site/ comprobadas contra la app real (docClaim en sweep.mjs)", "static": "tsc, biome, clippy, fmt, literales i18n, auditorías", "perf": "Presupuestos de tiempo de features.json"}
TYPE_LABEL = {"unit": "Unitarios", "integration": "Integración", "contract": "Contrato", "e2e": "End to end", "ui": "UI", "oracle": "Oráculo (UI ↔ backend ↔ BD)", "eval": "Evals de IA", "doc": "Documentación publicada", "static": "Calidad estática", "perf": "Rendimiento"}
STATUS_LABEL = {"ok": "OK", "fail": "FALLO", "skip": "N/A", "info": "INFO"}
slug = lambda s: re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")

# ---------- previous run for the delta ----------
prev = None
runs = sorted(p for p in run.parent.glob("*-" + run.name.rsplit("-", 1)[-1]) if p.is_dir() and p != run and (p / "results.json").exists())
if runs:
    prev = json.loads((runs[-1] / "results.json").read_text())
    prev_status = {(r["feature"], r["type"], r["name"]): r["status"] for r in prev["records"]}
    now_status = {(r["feature"], r["type"], r["name"]): r["status"] for r in records}
    newly_failing = [k for k, s in now_status.items() if s == "fail" and prev_status.get(k) == "ok"]
    newly_passing = [k for k, s in now_status.items() if s == "ok" and prev_status.get(k) == "fail"]

def counts(rs):
    c = collections.Counter(r["status"] for r in rs)
    return {s: c.get(s, 0) for s in ("ok", "fail", "skip", "info")}
def summary_table(rows, first_col):
    """rows: [(label, anchor, counts)]"""
    body = ""
    for label, anchor, c in rows:
        total = sum(c.values())
        cls = "bad" if c["fail"] else ("quiet" if total == 0 else "good")
        hint = TYPE_HINT.get(next((k for k, v in TYPE_LABEL.items() if v == label), ""), "")
        cell = (f'<a href="#{anchor}" title="{E(hint)}">{E(label)}</a>' if anchor else f'<span title="{E(hint)}">{E(label)}</span>') if hint else (f'<a href="#{anchor}">{E(label)}</a>' if anchor else E(label))
        body += f'<tr class="{cls}"><td>{cell}</td><td>{total}</td><td class="ok">{c["ok"]}</td><td class="fail">{c["fail"] or ""}</td><td class="skip">{c["skip"] or ""}</td><td class="info">{c["info"] or ""}</td></tr>'
    return f'<table class="sum"><thead><tr><th>{first_col}</th><th>Total</th><th>OK</th><th>Fallos</th><th>N/A</th><th>Info</th></tr></thead><tbody>{body}</tbody></table>'

def img(path):
    p = pathlib.Path(path)
    if not p.exists(): return f'<p class="muted">captura no disponible: {E(p.name)}</p>'
    return f'<figure><img loading="lazy" src="data:image/png;base64,{base64.b64encode(p.read_bytes()).decode()}" alt="{E(p.name)}"><figcaption>{E(p.name)}</figcaption></figure>'

# Models per (feature, type) section. A section run on one model names it in its
# header, so its rows and evidence blocks do not repeat it.
_models_by_section = {}
for _r in records:
    if _r["type"] == "eval":
        _models_by_section.setdefault((_r["feature"], _r["type"]), set()).add((_r.get("evidence") or {}).get("model") or "")
def per_row_model(r): return len(_models_by_section.get((r["feature"], r["type"]), set()) - {""}) > 1

def evidence(r):
    ev = r.get("evidence") or {}; parts = []
    if r["type"] == "eval":
        model_txt = f'<span class="lbl">Modelo</span>{E(ev.get("model") or "?")} <span class="muted">· validación: ' if per_row_model(r) else '<span class="lbl">Validación</span><span class="muted">'
        parts.append(f'<p>{model_txt}{E(ev.get("judge") or "heurística")}</span></p>')
        parts.append(f'<div class="qa"><div class="q"><span class="lbl">Pregunta</span>{E(ev.get("question", ""))}</div><div class="a"><span class="lbl">Respuesta</span>{E(ev.get("answer", "") or "(vacía)")}</div></div>')
        if ev.get("expected_output"): parts.append(f'<div class="qa"><div class="q"><span class="lbl">Golden</span>{E(ev["expected_output"])}</div></div>')
        jr = ev.get("judge_report")
        if jr:
            sc = jr.get("scores") or {}
            parts.append('<p><span class="lbl">Juez</span>' + E(jr.get("model", "")) + " · " + ", ".join(f"{k} {v:.2f}" for k, v in sc.items() if isinstance(v, (int, float))) + (f' · <em>{E(sc.get("rationale") or "")}</em>' if sc.get("rationale") else "") + (f' · <b class="fail">error: {E(sc["error"])}</b>' if sc.get("error") else "") + "</p>")
        checks = ev.get("checks") or []
        if checks:
            parts.append('<table class="checks"><thead><tr><th>Check</th><th>Esperado</th><th>Obtenido</th><th>Detalle</th></tr></thead><tbody>' + "".join(
                f'<tr class="{"bad" if not c["passed"] else "good"}"><td>{E(c["name"])}</td><td>{E(str(c["expected"]))}</td><td>{E(str(c["actual"]))}</td><td>{E(str(c.get("detail", "")))}</td></tr>' for c in checks) + "</tbody></table>")
        if ev.get("ai_trace") is not None:
            parts.append(f'<details><summary>Traza del motor de IA</summary><pre>{E(json.dumps(ev["ai_trace"], ensure_ascii=False, indent=1)[:60000])}</pre></details>')
    if ev.get("expect"): parts.append(f'<p><span class="lbl">Esperado</span>{E(ev["expect"])}</p>')
    # A failed doc claim has two possible culprits — the app changed, or the
    # page was always wrong — so the case says which edit it expects rather
    # than leaving the reader to guess from an assertion message.
    if ev.get("proposed_fix"):
        page = f' <span class="muted">({E(ev["page"])} ×4 idiomas)</span>' if ev.get("page") else ""
        parts.append(f'<p class="fix"><span class="lbl">Corrección propuesta</span>{E(ev["proposed_fix"])}{page}</p>')
    if ev.get("trace"): parts.append(f'<details open><summary>Traza</summary><pre>{E(ev["trace"])}</pre></details>')
    if ev.get("log_tail"): parts.append(f'<details><summary>Últimas líneas de app.log</summary><pre>{E(ev["log_tail"])}</pre></details>')
    for s in ev.get("shots") or []: parts.append(img(s))
    return "".join(parts)

def rows_html(rs, expand_fail=True):
    out = ""
    for r in rs:
        st = r["status"]; cls = {"ok": "good", "fail": "bad", "skip": "quiet", "info": "note"}[st]
        dur = f'{r["duration_ms"] / 1000:.1f} s' if r.get("duration_ms") else ""
        hint = r.get("desc") or ""
        name_cell = f'<span class="hint" title="{E(hint)}">{E(r["name"])}</span>' if hint else E(r["name"])
        cat = (r.get("evidence") or {}).get("category")  # chat eval cases: what the question exercises
        if cat: name_cell += f' <span class="muted">· {E(cat)}</span>'
        model = (r.get("evidence") or {}).get("model") if per_row_model(r) else None
        det = E(r["detail"] or "") + (f' <span class="muted">· modelo {E(model)}</span>' if model else "")
        out += f'<tr class="{cls}"><td><span class="chip {st}">{STATUS_LABEL[st]}</span></td><td class="name">{name_cell}</td><td class="det">{det}</td><td class="dur">{dur}</td></tr>'
        if st in ("fail", "info") and (r.get("evidence") or {}) and (expand_fail or st == "info"):
            ev = evidence(r)
            if ev: out += f'<tr class="ev"><td colspan="4">{ev}</td></tr>'
        elif r["type"] == "eval" and st == "ok":
            ev = r.get("evidence") or {}
            checks = ev.get("checks") or []
            tbl = '<table class="checks"><thead><tr><th>Check</th><th>Esperado</th><th>Obtenido</th></tr></thead><tbody>' + "".join(f'<tr><td>{E(c["name"])}</td><td>{E(str(c["expected"]))}</td><td>{E(str(c["actual"]))[:160]}</td></tr>' for c in checks) + "</tbody></table>"
            jr = ev.get("judge_report") or {}; sc = jr.get("scores") or {}
            jtxt = ("<p><span class=\"lbl\">Juez</span>" + E(jr.get("model", "")) + " · " + ", ".join(f"{k} {v:.2f}" for k, v in sc.items() if isinstance(v, (int, float))) + (f" · <em>{E(sc.get('rationale') or '')}</em>" if sc.get("rationale") else "") + "</p>") if jr else ""
            gold = f'<div class="q"><span class="lbl">Golden</span>{E(ev["expected_output"])}</div>' if ev.get("expected_output") else ""
            out += f'<tr class="ev"><td colspan="4"><details><summary>Pregunta, golden, checks, juez y traza</summary><div class="qa"><div class="q"><span class="lbl">Pregunta</span>{E(ev.get("question", ""))}</div>{gold}<div class="a"><span class="lbl">Respuesta</span>{E((ev.get("answer") or "")[:1500])}</div></div>{jtxt}{tbl}' + (f'<details><summary>Traza del motor de IA</summary><pre>{E(json.dumps(ev["ai_trace"], ensure_ascii=False, indent=1)[:60000])}</pre></details>' if ev.get("ai_trace") is not None else "") + '</details></td></tr>'
    return out

# ---------- build ----------
by_feat = collections.defaultdict(list)
for r in records: by_feat[r["feature"]].append(r)
feat_order = [f for f in features if f in by_feat] + [f for f in by_feat if f not in features]

def toc_count(rs):
    c = counts(rs); return f'<span class="n">{c["ok"]}</span>' + (f' <span class="f">{c["fail"]} ✗</span>' if c["fail"] else "")
index_html = "".join(
    f'<li><a href="#f-{slug(f)}">{E(f)}</a> {toc_count(by_feat[f])}<ul>' +
    "".join(f'<li><a href="#f-{slug(f)}-{t}">{TYPE_LABEL[t]}</a> {toc_count([r for r in by_feat[f] if r["type"] == t])}</li>' for t in types if any(r["type"] == t for r in by_feat[f])) + "</ul></li>"
    for f in feat_order)

global_rows = [(f, f"f-{slug(f)}", counts(by_feat[f])) for f in feat_order]
type_rows = [(TYPE_LABEL[t], None, counts([r for r in records if r["type"] == t])) for t in types if any(r["type"] == t for r in records)]

gaps = []
for f in features:
    have = {r["type"] for r in by_feat.get(f, [])}
    missing = [TYPE_LABEL[t] for t in ("unit", "integration", "contract", "e2e", "eval") if t not in have]
    if missing: gaps.append((f, missing))

delta_html = ""
if prev:
    delta_html = f'<h2 id="delta">Respecto a la pasada anterior</h2><p class="muted">{E(prev["meta"].get("started", ""))} · {E(prev["meta"].get("commit", ""))}</p>'
    delta_html += f'<p><b>{len(newly_failing)}</b> pasaron a fallo, <b>{len(newly_passing)}</b> pasaron a OK.</p>'
    if newly_failing: delta_html += "<ul class=\"bad\">" + "".join(f"<li>{E(k[0])} › {E(TYPE_LABEL[k[1]])} › {E(k[2])}</li>" for k in newly_failing[:50]) + "</ul>"
    if newly_passing: delta_html += "<ul class=\"good\">" + "".join(f"<li>{E(k[0])} › {E(TYPE_LABEL[k[1]])} › {E(k[2])}</li>" for k in newly_passing[:50]) + "</ul>"

sections = ""
for f in feat_order:
    rs = by_feat[f]; c = counts(rs)
    type_tbl = summary_table([(TYPE_LABEL[t], f"f-{slug(f)}-{t}", counts([r for r in rs if r["type"] == t])) for t in types if any(r["type"] == t for r in rs)], "Tipo de test")
    sub = ""
    for t in types:
        trs = [r for r in rs if r["type"] == t]
        if not trs: continue
        fails = [r for r in trs if r["status"] == "fail"]; rest = [r for r in trs if r["status"] != "fail"]
        # failures first and expanded; long passing lists collapse
        body = rows_html(fails)
        passing = rows_html(rest, expand_fail=False)
        if len(rest) > 25:
            sub_tbl = f'<table class="tests"><tbody>{body}</tbody></table><details><summary>{len(rest)} tests sin fallo</summary><table class="tests"><tbody>{passing}</tbody></table></details>'
        else:
            sub_tbl = f'<table class="tests"><tbody>{body}{passing}</tbody></table>'
        head = ""
        if t == "eval":
            ai = meta.get("ai") or {}; models = sorted({(r.get("evidence") or {}).get("model") or "" for r in trs} - {""})
            judge = next(((r.get("evidence") or {}).get("judge") for r in trs if (r.get("evidence") or {}).get("judge")), "ninguno")
            ev_meta = meta.get("evals") or {}
            harness = next(((r.get("evidence") or {}).get("harness") for r in trs if (r.get("evidence") or {}).get("harness")), "")
            if harness:
                head = f'<p class="evalmeta"><span class="lbl">Modelo</span>{E(", ".join(models) or "?")}<br><span class="lbl">Validación</span>{E(judge)}<br><span class="lbl">Harness</span>{E(harness)}</p>'
            else:
                head = f'<p class="evalmeta"><span class="lbl">Modelo</span>{E(", ".join(models) or ai.get("model", "?"))} <span class="muted">({E(ai.get("provider", "?"))}, embeddings {E(ai.get("embeddingModel", "?"))})</span><br><span class="lbl">Juez</span>{E(ev_meta.get("judge_model", "?"))} <span class="muted">· umbral 0,70 por métrica · una respuesta se acepta solo si superan los checks heurísticos y el juez</span><br><span class="lbl">Golden</span>cada caso lleva `expected_output` cuando la respuesta es determinable a partir de la BD demo; el juez la usa como referencia<br><span class="lbl">Validación</span>{E(judge)}<br><span class="lbl">Checks</span>answer_nonempty (respuesta no vacía), route (ruta elegida: ToolsFirst / RAG), tools_called (herramientas invocadas en orden), answer_contains / answer_not_contains (anclas de texto o enlaces email:// draft://), expected_tool_args_contains (argumentos de la herramienta). Cada caso lista los suyos en el desplegable.</p>'
        sub += f'<details class="type" id="f-{slug(f)}-{t}"{" open" if fails else ""}><summary>{TYPE_LABEL[t]} <span class="muted">{len(trs)} · {len(fails)} fallos</span></summary>{head}{sub_tbl}</details>'
    sections += f'<details class="feature" id="f-{slug(f)}"{" open" if c["fail"] else ""}><summary>{E(f)} <span class="muted">{c["ok"]} ok · {c["fail"]} fallos · {c["skip"]} n/a</span></summary>{type_tbl}{sub}</details>'

dbm = meta.get("db") or {}; demo = dbm.get("demo") or {}; snap = dbm.get("snapshot") or {}
db_html = ""
if snap:
    db_html += f'<p><span class="lbl">Snapshot</span><code>{E(snap.get("path", ""))}</code> · copia de la BD de producción hecha el {E(snap.get("copied_at", "?"))} ({E(snap.get("size_gb", "?"))} GB) · {E(snap.get("accounts", "?"))} cuentas activas · {E(snap.get("emails", "?"))} correos en {E(snap.get("threads", "?"))} hilos. <b>Datos reales del buzón: este informe no debe salir de esta máquina.</b></p>'
if demo:
    db_html += f'<p><span class="lbl">BD demo</span><code>{E(demo.get("path", ""))}</code> · {E(demo.get("accounts", "?"))} cuentas activas · {E(demo.get("emails", "?"))} correos en {E(demo.get("threads", "?"))} hilos · {E(demo.get("tags", "?"))} tags · {E(demo.get("events", "?"))} eventos de calendario · {E(demo.get("tasks", "?"))} tareas · {E(demo.get("drafts", "?"))} borradores · {E(demo.get("embeddings", "?"))} chunks de embeddings. Datos sintéticos (persona Ulises / EmailOps Labs), sin correo real.</p>'
db_html += '<div class="tablewrap"><table class="tests"><thead><tr><th>Tipo de test</th><th>Base de datos</th></tr></thead><tbody>' + "".join(f'<tr><td>{E(TYPE_LABEL.get(t, t))}</td><td class="det">{E(v)}</td></tr>' for t, v in (dbm.get("by_type") or {}).items()) + "</tbody></table></div>"
layers_html = "".join(f'<tr class="{ {"ok": "good", "error": "bad", "skipped": "quiet"}[l["status"]] }"><td>{E(l["layer"])}</td><td>{E(l["status"])}</td><td class="dur">{l.get("seconds", "")}</td><td class="det">{E(l.get("error", "") or "")}</td></tr>' for l in layers)
dirty = meta.get("dirty") or []
gc = counts(records)

PRIVATE = bool(meta.get("private"))
TITLE = "Verificación privada de EmailOps" if PRIVATE else "Verificación completa de EmailOps"
page = f'''<title>{TITLE}</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500&display=swap">
<style>
:root{{--bg:#F6F7F5;--panel:#FFFFFF;--ink:#1C2430;--muted:#5D6675;--line:#D9DED8;--accent:#1F6F8B;--ok:#2E7D4F;--fail:#B23A3A;--skip:#A66A00;--info:#4A5AA8;--chipbg:#EEF1EC;--badbg:#FBEDED;--evbg:#FAFBF9;color-scheme:light}}
@media (prefers-color-scheme: dark){{:root:not([data-theme="light"]){{--bg:#151A20;--panel:#1E252E;--ink:#E7EAEE;--muted:#9AA4B2;--line:#2E3843;--accent:#6FB6D0;--ok:#63C48A;--fail:#E27272;--skip:#E0A43C;--info:#9BA9E8;--chipbg:#273039;--badbg:#3A2323;--evbg:#1A2028;color-scheme:dark}}}}
:root[data-theme="dark"]{{--bg:#151A20;--panel:#1E252E;--ink:#E7EAEE;--muted:#9AA4B2;--line:#2E3843;--accent:#6FB6D0;--ok:#63C48A;--fail:#E27272;--skip:#E0A43C;--info:#9BA9E8;--chipbg:#273039;--badbg:#3A2323;--evbg:#1A2028;color-scheme:dark}}
body{{background:var(--bg);color:var(--ink);font-family:"IBM Plex Sans",system-ui,sans-serif;font-size:14.5px;line-height:1.5;margin:0;padding-block:28px 64px;padding-inline:clamp(16px,4vw,40px)}}
.layout{{display:grid;grid-template-columns:280px minmax(0,1fr);gap:32px;max-width:1500px;margin:0 auto}}
nav.toc{{position:sticky;top:16px;align-self:start;max-height:calc(100vh - 32px);overflow-y:auto;font-size:13px;padding-right:8px;border-right:1px solid var(--line)}}
nav.toc h2{{font-size:12px;text-transform:uppercase;letter-spacing:.08em;color:var(--muted);border:0;margin:0 0 8px;padding:0}}
nav.toc ul{{list-style:none;padding:0;margin:0}} nav.toc>ul>li{{margin:6px 0}} nav.toc li li{{padding-left:14px;font-size:12px}} nav.toc a{{text-decoration:none}} nav.toc a:hover{{text-decoration:underline}}
nav.toc .n{{color:var(--muted)}} nav.toc .f{{color:var(--fail);font-weight:600}}
main{{min-width:0}}
details.feature>summary{{cursor:pointer;list-style:none;font-size:20px;font-weight:600;margin:32px 0 10px;padding-top:8px;border-top:1px solid var(--line)}} details.feature>summary::-webkit-details-marker{{display:none}}
details.feature>summary::before,details.type>summary::before{{content:"▸";display:inline-block;width:1em;color:var(--muted);transition:transform .15s}} details[open]>summary::before{{transform:rotate(90deg)}}
details.type>summary{{cursor:pointer;list-style:none;font-size:15px;font-weight:600;margin:20px 0 8px}} details.type>summary::-webkit-details-marker{{display:none}}
@media (max-width:900px){{.layout{{grid-template-columns:1fr}} nav.toc{{position:static;max-height:none;border:0}}}}
h1{{font-size:clamp(24px,3vw,34px);font-weight:600;margin:0 0 4px;text-wrap:balance}}
h2{{font-size:20px;font-weight:600;margin:44px 0 10px;padding-top:8px;border-top:1px solid var(--line);text-wrap:balance}}
h3{{font-size:15px;font-weight:600;margin:24px 0 8px}}
a{{color:var(--accent)}} .up{{font-size:12px;font-weight:400;margin-left:8px}}
.muted{{color:var(--muted);font-weight:400;font-size:.92em}} .lbl{{display:inline-block;min-width:88px;color:var(--muted);font-size:12px;text-transform:uppercase;letter-spacing:.06em}}
.fix{{border-left:3px solid var(--accent);padding:6px 0 6px 10px;margin:8px 0}} .fix .lbl{{min-width:auto;margin-right:8px}}
.eyebrow{{text-transform:uppercase;letter-spacing:.08em;font-size:12px;color:var(--muted);font-weight:500}}
.meta{{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:10px 24px;margin:16px 0;padding:14px 0;border-top:1px solid var(--line);border-bottom:1px solid var(--line)}}
.meta dt{{font-size:11px;color:var(--muted);text-transform:uppercase;letter-spacing:.06em}} .meta dd{{margin:2px 0 0;font-family:"IBM Plex Mono",monospace;font-size:13px;word-break:break-all}}
.tiles{{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:10px;margin:18px 0}}
.tile{{background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:12px 14px}} .tile b{{display:block;font-size:28px;font-weight:600;font-variant-numeric:tabular-nums;line-height:1.1}}
.tile.ok b{{color:var(--ok)}} .tile.fail b{{color:var(--fail)}} .tile.skip b{{color:var(--skip)}}
table{{border-collapse:collapse;width:100%;font-size:13.5px;background:var(--panel)}} th,td{{text-align:left;vertical-align:top;padding:6px 10px;border-top:1px solid var(--line)}} thead th{{background:var(--chipbg);font-weight:600;border-top:0}}
table.sum td:not(:first-child){{text-align:right;font-variant-numeric:tabular-nums;width:72px}} table.sum td.fail{{color:var(--fail);font-weight:600}} table.sum td.ok{{color:var(--ok)}} table.sum td.skip,table.sum td.info{{color:var(--muted)}}
.tablewrap{{overflow-x:auto;border:1px solid var(--line);border-radius:6px;margin:8px 0}}
table.tests td.name{{font-family:"IBM Plex Mono",monospace;font-size:12.5px;max-width:48ch;word-break:break-word}} table.tests td.det{{color:var(--muted);max-width:56ch;word-break:break-word}} table.tests td.dur{{text-align:right;color:var(--muted);white-space:nowrap;font-variant-numeric:tabular-nums}}
tr.bad td{{background:var(--badbg)}} tr.ev td{{background:var(--evbg);padding:12px 14px}}
.hint{{border-bottom:1px dotted var(--muted);cursor:help}} .evalmeta{{background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:8px 12px;max-width:none}}
.chip{{display:inline-block;font-size:11px;font-weight:600;letter-spacing:.05em;padding:2px 8px;border-radius:999px;background:var(--chipbg);white-space:nowrap}} .chip.ok{{color:var(--ok)}} .chip.fail{{color:var(--fail)}} .chip.skip{{color:var(--skip)}} .chip.info{{color:var(--info)}}
pre{{background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:10px 12px;overflow-x:auto;font-size:12px;max-height:480px;white-space:pre-wrap;word-break:break-word}}
details summary{{cursor:pointer;color:var(--accent);margin:6px 0}}
.qa{{display:grid;gap:8px;margin-bottom:10px}} .qa .q,.qa .a{{background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:8px 12px;white-space:pre-wrap}}
table.checks{{margin:8px 0}} table.checks tr.bad td{{background:var(--badbg)}}
figure{{margin:10px 0 0}} figure img{{width:100%;max-width:900px;height:auto;border:1px solid var(--line);border-radius:4px;display:block}} figcaption{{font-family:"IBM Plex Mono",monospace;font-size:12px;color:var(--muted);margin-top:4px}}
ul.index{{columns:2;column-gap:32px;padding-left:18px}} ul.index ul{{padding-left:16px;font-size:13px;margin:2px 0 6px}} ul.index>li{{break-inside:avoid;margin-bottom:6px}}
ul.bad li{{color:var(--fail)}} ul.good li{{color:var(--ok)}}
@media (max-width:720px){{ul.index{{columns:1}}}}
</style>
<div class="layout">
<nav class="toc" aria-label="Índice">
<h2>Índice</h2>
<ul><li><a href="#top">Resumen global</a></li>{index_html}<li><a href="#capas">Capas ejecutadas</a></li><li><a href="#bd">Bases de datos</a></li><li><a href="#huecos">Huecos de cobertura</a></li>{"<li><a href=\"#delta\">Respecto a la pasada anterior</a></li>" if prev else ""}</ul>
</nav>
<main id="top">
<div class="eyebrow">{"Verificación privada" if PRIVATE else "Verificación completa"} · {E(meta.get("tier", ""))}</div>
<h1>{TITLE}</h1>
{'<p class="evalmeta" style="border-color:var(--fail)"><b>Privado.</b> Casos de <code>private-evals/</code> sobre un snapshot del buzón real: remitentes, asuntos y respuestas son datos personales. No publicar ni compartir.</p>' if PRIVATE else ''}
<p class="muted">Todas las capas de prueba en una pasada, atribuidas por feature. Inicio {E(meta.get("started", ""))}, fin {E(meta.get("finished", ""))}.</p>
<dl class="meta">
  <div><dt>Worktree</dt><dd>{E(meta.get("worktree", ""))}</dd></div>
  <div><dt>Rama</dt><dd>{E(meta.get("branch", ""))}</dd></div>
  <div><dt>Commit</dt><dd>{E(meta.get("commit", ""))}</dd></div>
  <div><dt>Árbol</dt><dd>{"limpio" if not dirty else E(f"{len(dirty)} cambios sin commitear: " + ", ".join(d.split()[-1] for d in dirty[:6]))}</dd></div>
  <div><dt>Evidencia</dt><dd>{E(str(run))}</dd></div>
</dl>
<div class="tiles">
  <div class="tile"><span class="eyebrow">Tests</span><b>{len(records)}</b></div>
  <div class="tile ok"><span class="eyebrow">OK</span><b>{gc["ok"]}</b></div>
  <div class="tile fail"><span class="eyebrow">Fallos</span><b>{gc["fail"]}</b></div>
  <div class="tile skip"><span class="eyebrow">N/A</span><b>{gc["skip"]}</b></div>
  <div class="tile"><span class="eyebrow">Info</span><b>{gc["info"]}</b></div>
</div>

<h2 id="resumen">Resumen global</h2>
<div class="tablewrap">{summary_table(global_rows, "Feature")}</div>
<div class="tablewrap">{summary_table(type_rows, "Tipo de test")}</div>
{delta_html}
{sections}

<h2 id="capas">Capas ejecutadas</h2>
<div class="tablewrap"><table class="tests"><thead><tr><th>Capa</th><th>Estado</th><th>s</th><th>Error</th></tr></thead><tbody>{layers_html}</tbody></table></div>

<h2 id="bd">Bases de datos</h2>
{db_html}
<h2 id="huecos">Huecos de cobertura</h2>
<p class="muted">Tipos de test sin ningún caso atribuido a la feature. No es un fallo: es dónde no hay red.</p>
<div class="tablewrap"><table class="tests"><thead><tr><th>Feature</th><th>Sin tests de</th></tr></thead><tbody>{"".join(f"<tr><td>{E(f)}</td><td>{E(', '.join(m))}</td></tr>" for f, m in gaps)}</tbody></table></div>
</main>
</div>
<script>
// A link into a collapsed section opens it (and its parents) before jumping.
function openHash(){{const id=location.hash.slice(1); if(!id) return; let el=document.getElementById(id); if(!el) return; for(let p=el; p; p=p.parentElement) if(p.tagName==='DETAILS') p.open=true; el.scrollIntoView({{block:'start'}});}}
window.addEventListener('hashchange', openHash); openHash();
</script>
'''
out.write_text(page)
print(f"{out} ({out.stat().st_size // 1024} KB) · {gc}")
