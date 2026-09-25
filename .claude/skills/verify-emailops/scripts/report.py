#!/usr/bin/env python3
"""Render a sweep run (results.json + findings.json + PNGs) as one self-contained HTML report.

usage: report.py <run_dir>/sweep [out.html]
"""
import base64, html, json, sys, datetime, pathlib

sweep = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else sweep.parent / "informe.html"
results = json.loads((sweep / "results.json").read_text())
# A Tag Board oracle run (tagboard_check.mjs) next to the sweep joins the table.
tagboard = sweep.parent / "tagboard" / "results.json"
if tagboard.exists():
    results += [dict(r, feature=f"Tag Board · {r.get('kind', 'oráculo')}") for r in json.loads(tagboard.read_text())]
findings = json.loads((sweep / "findings.json").read_text()) if (sweep / "findings.json").exists() else []
meta = json.loads((sweep / "meta.json").read_text()) if (sweep / "meta.json").exists() else {}

def img(name):
    p = sweep / name
    if not p.exists(): return ""
    b = base64.b64encode(p.read_bytes()).decode()
    return f'<figure><img src="data:image/png;base64,{b}" alt="{html.escape(name)}" loading="lazy"><figcaption>{html.escape(name)}</figcaption></figure>'

E = html.escape
counts = {k: sum(r["status"] == k for r in results) for k in ("ok", "fail", "skip")}
sev_label = {"bug": "Fallo", "ux": "Usabilidad", "watch": "Vigilar", "minor": "Menor"}
sev_count = {}
for f in findings: sev_count[f["sev"]] = sev_count.get(f["sev"], 0) + 1

features = []
for r in results:
    if r["feature"] not in features: features.append(r["feature"])

rows_html = ""
for feat in features:
    rs = [r for r in results if r["feature"] == feat]
    rows_html += f'<tr class="feat"><th colspan="4">{E(feat)} <span class="muted">{sum(r["status"]=="ok" for r in rs)}/{len(rs)} ok</span></th></tr>'
    for r in rs:
        rows_html += (f'<tr><td><span class="chip {r["status"]}">{ {"ok":"OK","fail":"FALLO","skip":"N/A"}[r["status"]] }</span></td>'
                      f'<td>{E(r["step"])}</td><td class="exp">{E(r["expect"])}</td><td class="det">{E(r["detail"].removeprefix("FAIL: ").removeprefix("SKIP: "))}</td></tr>')

find_html = ""
for f in findings:
    shots = "".join(img(s) for s in f.get("shots", []))
    repro = "".join(f"<li>{E(s)}</li>" for s in f.get("repro", []))
    find_html += f'''
<article class="finding sev-{f["sev"]}" id="{f["id"]}">
  <header><span class="sev">{sev_label.get(f["sev"], f["sev"])}</span><span class="fid">{f["id"]}</span><h3>{E(f["title"])}</h3></header>
  <dl>
    <dt>Dónde</dt><dd>{E(f["where"])}</dd>
    <dt>Qué ocurre</dt><dd>{E(f["what"])}</dd>
    <dt>Esperado</dt><dd>{E(f["expected"])}</dd>
    <dt>Reproducir</dt><dd><ol>{repro}</ol></dd>
    <dt>Dónde mirar</dt><dd><code>{E(f["code"])}</code></dd>
  </dl>
  {f'<div class="shots">{shots}</div>' if shots else ''}
</article>'''

page = f'''<title>Validación UI de EmailOps</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500&display=swap">
<style>
:root{{--bg:#F6F7F5;--panel:#FFFFFF;--ink:#1C2430;--muted:#5D6675;--line:#D9DED8;--accent:#1F6F8B;--ok:#2E7D4F;--fail:#B23A3A;--skip:#A66A00;--ux:#7A4FB5;--watch:#A66A00;--minor:#5D6675;--chipbg:#EEF1EC;color-scheme:light}}
@media (prefers-color-scheme: dark){{:root:not([data-theme="light"]){{--bg:#151A20;--panel:#1E252E;--ink:#E7EAEE;--muted:#9AA4B2;--line:#2E3843;--accent:#6FB6D0;--ok:#63C48A;--fail:#E27272;--skip:#E0A43C;--ux:#B79BE0;--watch:#E0A43C;--minor:#9AA4B2;--chipbg:#273039;color-scheme:dark}}}}
:root[data-theme="dark"]{{--bg:#151A20;--panel:#1E252E;--ink:#E7EAEE;--muted:#9AA4B2;--line:#2E3843;--accent:#6FB6D0;--ok:#63C48A;--fail:#E27272;--skip:#E0A43C;--ux:#B79BE0;--watch:#E0A43C;--minor:#9AA4B2;--chipbg:#273039;color-scheme:dark}}
body{{background:var(--bg);color:var(--ink);font-family:"IBM Plex Sans",system-ui,-apple-system,sans-serif;font-size:15px;line-height:1.55;margin:0;padding-block:32px 64px;padding-inline:clamp(16px,4vw,40px)}}
main{{max-width:1080px;margin:0 auto}}
h1{{font-size:clamp(26px,3.4vw,36px);font-weight:600;letter-spacing:-.01em;margin:0 0 6px;text-wrap:balance}}
h2{{font-size:20px;font-weight:600;margin:40px 0 12px;text-wrap:balance}}
h3{{font-size:17px;font-weight:600;margin:0;text-wrap:balance}}
p{{max-width:70ch}}
code,.mono{{font-family:"IBM Plex Mono",ui-monospace,Menlo,monospace;font-size:.92em}}
.muted{{color:var(--muted);font-weight:400}}
.eyebrow{{text-transform:uppercase;letter-spacing:.08em;font-size:12px;color:var(--muted);font-weight:500}}
.meta{{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:12px 24px;margin:20px 0 0;padding:16px 0;border-top:1px solid var(--line);border-bottom:1px solid var(--line)}}
.meta dt{{font-size:12px;color:var(--muted);text-transform:uppercase;letter-spacing:.06em}}
.meta dd{{margin:2px 0 0;font-variant-numeric:tabular-nums}}
.tiles{{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:12px;margin:20px 0}}
.tile{{background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:14px 16px}}
.tile b{{display:block;font-size:30px;font-weight:600;font-variant-numeric:tabular-nums;line-height:1.1}}
.tile.ok b{{color:var(--ok)}}.tile.fail b{{color:var(--fail)}}.tile.skip b{{color:var(--skip)}}
.chip{{display:inline-block;font-size:11px;font-weight:600;letter-spacing:.05em;padding:2px 8px;border-radius:999px;background:var(--chipbg);color:var(--ink);white-space:nowrap}}
.chip.ok{{color:var(--ok)}}.chip.fail{{color:var(--fail)}}.chip.skip{{color:var(--skip)}}
.finding{{background:var(--panel);border:1px solid var(--line);border-left:4px solid var(--minor);border-radius:6px;padding:18px 20px;margin:14px 0}}
.finding.sev-bug{{border-left-color:var(--fail)}}.finding.sev-ux{{border-left-color:var(--ux)}}.finding.sev-watch{{border-left-color:var(--watch)}}
.finding header{{display:flex;align-items:baseline;gap:10px;flex-wrap:wrap;margin-bottom:10px}}
.finding .sev{{font-size:11px;font-weight:600;letter-spacing:.06em;text-transform:uppercase}}
.sev-bug .sev{{color:var(--fail)}}.sev-ux .sev{{color:var(--ux)}}.sev-watch .sev{{color:var(--watch)}}.sev-minor .sev{{color:var(--minor)}}
.fid{{font-family:"IBM Plex Mono",monospace;color:var(--muted);font-size:13px}}
.finding dl{{display:grid;grid-template-columns:110px 1fr;gap:6px 16px;margin:0}}
.finding dt{{color:var(--muted);font-size:13px;padding-top:2px}}.finding dd{{margin:0;max-width:78ch}}
.finding ol{{margin:0;padding-left:18px}}
.shots{{display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:14px;margin-top:16px}}
figure{{margin:0}}figure img{{width:100%;height:auto;border:1px solid var(--line);border-radius:4px;display:block}}
figcaption{{font-family:"IBM Plex Mono",monospace;font-size:12px;color:var(--muted);margin-top:4px}}
.tablewrap{{overflow-x:auto;border:1px solid var(--line);border-radius:6px;background:var(--panel)}}
table{{border-collapse:collapse;width:100%;font-size:14px}}
th,td{{text-align:left;vertical-align:top;padding:8px 12px;border-top:1px solid var(--line)}}
tr.feat th{{background:var(--chipbg);font-weight:600;padding-top:10px}}
td.exp{{color:var(--muted);max-width:34ch}}td.det{{font-family:"IBM Plex Mono",monospace;font-size:12.5px;max-width:52ch;word-break:break-word}}
ul.limits{{max-width:78ch;padding-left:20px}}
pre{{background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:12px 14px;overflow-x:auto;font-size:13px}}
@media (max-width:640px){{.finding dl{{grid-template-columns:1fr}}.finding dt{{padding-top:8px}}}}
</style>
<main>
<div class="eyebrow">Informe de validación en la interfaz real</div>
<h1>Validación UI de EmailOps</h1>
<p class="muted">Pasada automática sobre la app de escritorio en ejecución, conducida por el WebDriver embebido, con captura por paso. {E(meta.get("date",""))}</p>
<dl class="meta">
  <div><dt>Build</dt><dd>{E(meta.get("build","v0.6.7"))}</dd></div>
  <div><dt>Datos</dt><dd>{E(meta.get("data","BD demo sintética, 2 cuentas IMAP, 79 correos"))}</dd></div>
  <div><dt>Modelo local</dt><dd>{E(meta.get("model","qwen3.5-4b-q4_k_m (llama.cpp embebido)"))}</dd></div>
  <div><dt>Ventana</dt><dd>{E(meta.get("window","1200 × 800, macOS, WKWebView"))}</dd></div>
  <div><dt>Evidencia</dt><dd class="mono">{E(str(sweep))}</dd></div>
</dl>

<div class="tiles">
  <div class="tile"><span class="eyebrow">Pasos</span><b>{len(results)}</b></div>
  <div class="tile ok"><span class="eyebrow">Correctos</span><b>{counts["ok"]}</b></div>
  <div class="tile fail"><span class="eyebrow">Fallos</span><b>{counts["fail"]}</b></div>
  <div class="tile skip"><span class="eyebrow">No aplicables</span><b>{counts["skip"]}</b></div>
  <div class="tile"><span class="eyebrow">Hallazgos</span><b>{len(findings)}</b><span class="muted">{", ".join(f"{v} {sev_label[k].lower()}" for k,v in sev_count.items())}</span></div>
</div>

<h2>Hallazgos</h2>
<p>{E(meta.get("findings_intro", "Ordenados por gravedad. Cada uno lleva los pasos para reproducirlo a mano, la captura tomada en el momento del fallo y el fichero donde mirar."))}</p>
{find_html}

<h2>Todos los pasos</h2>
<div class="tablewrap"><table>
<thead><tr><th>Estado</th><th>Paso</th><th>Qué se esperaba</th><th>Observado</th></tr></thead>
<tbody>{rows_html}</tbody>
</table></div>

<h2>Qué cubre y qué no</h2>
<ul class="limits">
  <li><b>Cubierto:</b> inbox (lista, abrir y cerrar hilo, menú de fila), búsqueda (resultados, vacío, limpiar), cambio de cuenta y vista unificada, Tag Board (bloques, rango, buscador, abrir hilo), Compose (campos, Send deshabilitado, envío sin credenciales, borrador y descarte), Ajustes (todas las pestañas), chat (pregunta con respuesta correcta en 5 s, nuevo chat) y la apertura de Attachments, Drafts, Sent, Spam, Deleted, Contacts, Dashboard, Tasks y Memory.</li>
  <li><b>No aplicable con la BD demo:</b> pestañas de categoría (solo Gmail), Calendar (solo Gmail/Outlook), envío real y sincronización (las cuentas demo no tienen credenciales).</li>
  <li><b>No cubierto:</b> AI Draft y traducción, gestos de arrastrar y soltar (reordenar bloques, mover correos a carpetas), acciones sobre adjuntos, menús nativos, comportamiento en pantallas estrechas.</li>
  <li><b>Cómo repetirlo:</b> desde la raíz del repo, con la instancia de verificación levantada.</li>
</ul>
<pre>V=.claude/skills/verify-emailops/scripts/verify.sh
$V launch                                 # instancia aislada, BD demo, WebDriver en 4445
node .claude/skills/verify-emailops/scripts/sweep.mjs "$(readlink src-tauri/reports/verify/current)"
python3 .claude/skills/verify-emailops/scripts/report.py "$(readlink src-tauri/reports/verify/current)/sweep"
$V cleanup</pre>
</main>
'''
out.write_text(page)
print(f"{out} ({out.stat().st_size // 1024} KB)")
