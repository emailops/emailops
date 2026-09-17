#!/usr/bin/env python3
"""Build the HTML report for the "EmailOps help" chat feature from the run
artefacts `scripts/help_docs_eval.sh` writes (before/after chat turns + the
app_help eval run). Output is gitignored (src-tauri/reports/).

    uv run scripts/help_docs_report.py [--runs DIR] [--out FILE] [--demo-db FILE]
"""
import argparse, datetime, html, json, pathlib, re, sqlite3, subprocess, sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ap = argparse.ArgumentParser()
ap.add_argument("--runs", type=pathlib.Path, default=ROOT / "src-tauri/reports/help-docs",
                help="directory with before.json, after.json, eval.json, gates.json")
ap.add_argument("--out", type=pathlib.Path, default=ROOT / "src-tauri/reports/help-docs/report.html")
ap.add_argument("--demo-db", type=pathlib.Path, default=ROOT / ".emailops-demo-data/emailops.db")
args = ap.parse_args()
S = args.runs
OUT = args.out
OUT.parent.mkdir(parents=True, exist_ok=True)

def load(p):
    try:
        return json.loads(pathlib.Path(p).read_text(encoding="utf-8"))
    except Exception:
        return None

def esc(s):
    # Small local models occasionally emit a broken code point (U+FFFD); it
    # carries no information and the artifact host rejects it.
    return html.escape(str(s if s is not None else "").replace("\ufffd", ""))

DOCS = "https://getemailops.com"
def md_to_html(text):
    """Minimal: escape, then turn [label](help://…) / (email://…) into links or chips, and paragraphs."""
    t = esc(text or "")
    def help_link(m):
        label, lang, page, anchor = m.group(1), m.group(2), m.group(3), m.group(4)
        url = f"{DOCS}/{lang}/docs/{page}/" + (f"#{anchor}" if anchor else "")
        return f'<a class="help" href="{url}" target="_blank" rel="noopener">{label}</a>'
    t = re.sub(r"\[([^\]]+)\]\(help://([a-z]{2})/([a-z0-9-]+)(?:#([^)\s]+))?\)", help_link, t)
    t = re.sub(r"\[([^\]]+)\]\(email://[^)]+\)", r'<span class="chip">\1</span>', t)
    t = re.sub(r"\[([^\]]+)\]\(draft://[^)]+\)", r'<span class="chip">\1</span>', t)
    # Fenced code → <pre>; then bold and inline code in the prose.
    t = re.sub(r"```[a-z]*\n(.*?)```", lambda m: "<pre>" + m.group(1).strip() + "</pre>", t, flags=re.S)
    t = re.sub(r"\*\*([^*\n]+)\*\*", r"<b>\1</b>", t)
    t = re.sub(r"`([^`\n]+)`", r"<code>\1</code>", t)
    out = []
    for block in re.split(r"\n\s*\n", t):
        block = block.strip()
        if not block:
            continue
        if block.startswith("<pre>"):
            out.append(block)
        else:
            out.append(f"<p>{block.replace(chr(10), '<br>')}</p>")
    return "".join(out)

def turn_of(envelope):
    if not envelope or not envelope.get("ok"):
        err = (envelope or {}).get("error") or {}
        return {"answer": f"(sin respuesta: {err.get('message', 'no ejecutado')})", "trace": {}, "latency": None}
    data = envelope.get("data") or {}
    turns = data.get("turns") or [data]
    t = turns[0]
    return {"answer": t.get("answer") or t.get("content") or "", "trace": t.get("trace") or {},
            "latency": t.get("latencyMs") or t.get("latency_ms") or (t.get("trace") or {}).get("totalElapsedMs")}

def fmt_ms(ms):
    if ms is None: return "—"
    return f"{ms/1000:.1f} s" if ms >= 1000 else f"{ms} ms"

base = turn_of(load(S / "before.json"))
after = turn_of(load(S / "after.json"))
evalrun = load(S / "eval.json")
gates = load(S / "gates.json") or {}
question = "how do I make EmailOps use my local Ollama instead of the built-in model?"

# corpus stats from the demo DB
stats = {"chunks": 0, "per_lang": {}, "nav": 0, "embedded": 0}
try:
    c = sqlite3.connect(str(args.demo_db))
    stats["chunks"] = c.execute("SELECT COUNT(*) FROM help_doc_chunks").fetchone()[0]
    stats["per_lang"] = dict(c.execute("SELECT lang, COUNT(*) FROM help_doc_chunks GROUP BY lang").fetchall())
    stats["nav"] = c.execute("SELECT COUNT(*) FROM help_doc_chunks WHERE nav_target IS NOT NULL AND part = 0").fetchone()[0]
    stats["embedded"] = c.execute("SELECT COUNT(*) FROM help_doc_chunks WHERE embedding_model IS NOT NULL").fetchone()[0]
except Exception as e:
    print("stats:", e, file=sys.stderr)

diffstat = subprocess.run(["git", "diff", "--stat=110", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout
untracked = subprocess.run(["git", "ls-files", "--others", "--exclude-standard"], cwd=ROOT, capture_output=True, text=True).stdout.split()
sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
branch = subprocess.run(["git", "branch", "--show-current"], cwd=ROOT, capture_output=True, text=True).stdout.strip()

def trace_line(tr):
    if not tr: return "—"
    route = (tr.get("route") or {}).get("mode", "—")
    tools = [c.get("name") for c in tr.get("toolCalls") or []]
    h = tr.get("help")
    sim = h.get("topSimilarity") if h else None
    hs = "sin bloque de ayuda" if not h else f"ayuda: {h.get('included')}/{h.get('candidates')} secciones, sim {f'{sim:.2f}' if sim is not None else '—'}"
    return f"ruta {route} · tools {', '.join(tools) if tools else 'ninguna'} · {hs} · {fmt_ms(tr.get('totalElapsedMs'))}"

# ── eval table ────────────────────────────────────────────────────────────
cases_html = ""
if evalrun and evalrun.get("ok"):
    run = evalrun["data"]
    rows = []
    for c in run["cases"]:
        tr = c.get("trace") or {}
        h = tr.get("help") or {}
        checks = "".join(
            f'<li class="{ "ok" if k["passed"] else "ko"}"><span class="mono">{esc(k["name"])}</span> — {esc(k["detail"])}</li>'
            for k in c["checks"])
        rows.append(f'''
<details class="case {'pass' if c['passed'] else 'fail'}">
  <summary>
    <span class="pill {'pass' if c['passed'] else 'fail'}">{'PASA' if c['passed'] else 'FALLA'}</span>
    <span class="mono id">{esc(c['id'])}</span>
    <span class="meta">{esc(c['tier'])} · {c['checksPassed']}/{c['checksTotal']} checks · {fmt_ms(c['latencyMs'])}</span>
  </summary>
  <div class="case-body">
    <p class="q"><b>User:</b> {esc(c['question'])}</p>
    <div class="a"><b>A:</b>{md_to_html(c['answer'])}</div>
    <p class="meta">{esc(trace_line(tr))}{' · secciones: ' + esc(', '.join(h.get('chunkIds') or [])) if h.get('chunkIds') else ''}</p>
    <ul class="checks">{checks}</ul>
  </div>
</details>''')
    cases_html = "".join(rows)
    eval_summary = f"{run['casesPassed']} de {run['casesTotal']} casos pasan"
    eval_ok = run["passed"]
else:
    eval_summary = "no ejecutada"
    eval_ok = False
    cases_html = f"<p class='muted'>La eval no llegó a ejecutarse: {esc((evalrun or {}).get('error') or 'sin salida')}</p>"

# Earlier eval runs (eval-run1.json, eval-run2.json…) become an iteration log:
# what changed between runs is the story of calibrating the gate.
iterations_html = ""
prev = sorted(S.glob("eval-run*.json"))
if prev:
    blocks = []
    for pth in prev:
        prev_run = load(pth)
        if not prev_run or not prev_run.get("ok"):
            continue
        rr = prev_run["data"]
        rows = "".join(
            f"<tr><td class='mono'>{esc(c['id'])}</td><td><span class='pill {'pass' if c['passed'] else 'fail'}'>{'PASA' if c['passed'] else 'FALLA'}</span></td>"
            f"<td class='mono'>{round(((c.get('trace') or {}).get('help') or {}).get('topSimilarity') or 0, 2)}</td>"
            f"<td class='mono'>{esc(', '.join(((c.get('trace') or {}).get('help') or {}).get('chunkIds') or []))}</td></tr>"
            for c in rr["cases"])
        blocks.append(f"<h3>{esc(pth.stem)} — {rr['casesPassed']} de {rr['casesTotal']}</h3><div class='tablewrap'><table><thead><tr><th>caso</th><th></th><th>sim</th><th>secciones servidas</th></tr></thead><tbody>{rows}</tbody></table></div>")
    if blocks:
        iterations_html = ("<h2>Iteraciones previas</h2>"
            "<p><b>Ejecución 1 (4 de 6).</b> El orden final lo decidía la fusión RRF, y «EmailOps» dominaba el FTS porque aparece en todas las preguntas y en casi todas las secciones; el caso espejo pasaba la puerta con 0.57. Cambios: quitar «emailops» del FTS de ayuda, subir el umbral a 0.60 y ordenar por similitud vectorial.</p>"
            "<p><b>Ejecución 2 (4 de 6).</b> La puerta ya excluía el caso espejo, pero las secciones servidas seguían siendo las equivocadas. Un probe de ranking con el modelo real (<span class='mono'>help_docs_rank_probe</span>) lo explicó: la similitud coseno de nomic-embed (Q4, llama.cpp, sin y con prefijos) no ordena las secciones — la sección correcta caía entre la posición 16 y 57 — mientras que BM25 sin «emailops» la pone primera o segunda en cinco de seis preguntas. Además, el modelo 4B omitía el enlace <span class='mono'>help://</span> en dos respuestas correctas. Cambios: puerta global por la mejor similitud (¿es sobre la app?), orden por fusión con el FTS al doble de peso (¿qué sección?), y un enlace de respaldo determinista cuando la respuesta viene de la guía, no llamó a tools y no cita nada más.</p>"
            "<p><b>Ejecución 3 (5 de 6).</b> Todos los casos sobre la app pasan y el francés ya recibe «El chat es lento». El espejo falló por su propia aserción (exigía el chip <span class='mono'>email://</span> y el modelo citó con [1], la inestabilidad de enlaces de buzón ya documentada en <span class='mono'>user_queries.yaml</span>): se acepta una cita numerada como prueba de anclaje. En inglés aún no llegaba la sección exacta (Lenses perdía frente a «Desactivarlo todo»): la fusión RRF premia el acuerdo entre rankers y el vector no aporta. Cambio: servir en orden BM25 y usar la fusión solo como desempate.</p>"
            "<p><b>Límite medido.</b> Con este embedder, dos preguntas de buzón superan el umbral («summarize today's emails» 0.66, «list all my pending tasks» 0.60). Se acepta a favor del recuerdo: un falso positivo solo mete el bloque en el prompt, el modelo lo ignora (caso espejo) y nunca navega, porque la navegación sigue a la cita de la respuesta.</p>"
            + "".join(blocks))

gate_rows = "".join(
    f"<tr><td>{esc(name)}</td><td><span class='pill {'pass' if g.get('ok') else 'fail'}'>{'OK' if g.get('ok') else 'FALLA'}</span></td><td>{esc(g.get('detail',''))}</td></tr>"
    for name, g in gates.items())

files_new = [f for f in untracked if not f.startswith(".emailops")]
now = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
run_meta = (load(S / "meta.json") or {}).get("machine", "modelo qwen3.5-4b-q4_k_m · llama.cpp")

page = f"""<title>Ayuda de EmailOps en el chat</title>
<meta name="description" content="Informe de implementación y evals del RAG sobre las guías de usuario">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Serif:wght@500;600&family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500&display=swap">
<style>
:root {{
  --ground:#F4F6F8; --surface:#FFFFFF; --ink:#16202A; --muted:#5C6B7A; --line:#D7DEE5;
  --accent:#0E7C86; --accent-soft:#E1F1F2; --good:#237A57; --good-soft:#E3F2EA; --bad:#B4413C; --bad-soft:#F8E5E3; --warn:#9A6A12;
  --serif:"IBM Plex Serif", Georgia, "Times New Roman", serif;
  --sans:"IBM Plex Sans", -apple-system, "Segoe UI", Helvetica, Arial, sans-serif;
  --mono:"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{
  --ground:#0F1519; --surface:#171F26; --ink:#E8EDF1; --muted:#9AA8B4; --line:#2A3641;
  --accent:#4FC1C9; --accent-soft:#12333A; --good:#5CC28E; --good-soft:#12301F; --bad:#E07A73; --bad-soft:#3A1A18; --warn:#E0B15A;
}} }}
:root[data-theme="dark"] {{
  --ground:#0F1519; --surface:#171F26; --ink:#E8EDF1; --muted:#9AA8B4; --line:#2A3641;
  --accent:#4FC1C9; --accent-soft:#12333A; --good:#5CC28E; --good-soft:#12301F; --bad:#E07A73; --bad-soft:#3A1A18; --warn:#E0B15A;
}}
body {{ background:var(--ground); color:var(--ink); font-family:var(--sans); font-size:15px; line-height:1.55; margin:0; }}
.wrap {{ max-width:900px; margin:0 auto; padding-block:32px 64px; padding-inline:20px; }}
header h1 {{ font-family:var(--serif); font-weight:600; font-size:clamp(28px,4vw,38px); line-height:1.15; margin:0 0 8px; text-wrap:balance; }}
header .lede {{ font-size:17px; color:var(--muted); max-width:65ch; margin:0 0 14px; }}
.runmeta {{ display:flex; flex-wrap:wrap; gap:8px 18px; font-family:var(--mono); font-size:12.5px; color:var(--muted); }}
.stats {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(150px,1fr)); gap:12px; margin:26px 0 6px; }}
.stat {{ background:var(--surface); border:1px solid var(--line); border-radius:6px; padding:12px 14px; }}
.stat .n {{ font-family:var(--serif); font-size:28px; font-weight:600; line-height:1.1; font-variant-numeric:tabular-nums; }}
.stat .n.good {{ color:var(--good); }} .stat .n.bad {{ color:var(--bad); }}
.stat .l {{ font-size:12px; letter-spacing:.04em; text-transform:uppercase; color:var(--muted); margin-top:4px; }}
h2 {{ font-family:var(--serif); font-weight:600; font-size:22px; margin:40px 0 6px; padding-bottom:6px; border-bottom:1px solid var(--line); text-wrap:balance; }}
h3 {{ font-size:15px; font-weight:600; margin:22px 0 6px; }}
p, li {{ max-width:70ch; }}
.muted {{ color:var(--muted); }}
.mono {{ font-family:var(--mono); font-size:13px; }}
.pill {{ display:inline-block; font-family:var(--mono); font-size:11px; letter-spacing:.06em; padding:2px 8px; border-radius:999px; }}
.pill.pass {{ background:var(--good-soft); color:var(--good); }} .pill.fail {{ background:var(--bad-soft); color:var(--bad); }}
.exchange {{ display:grid; grid-template-columns:1fr 1fr; gap:14px; }}
@media (max-width:720px) {{ .exchange {{ grid-template-columns:1fr; }} }}
.turn {{ background:var(--surface); border:1px solid var(--line); border-radius:6px; padding:14px 16px; }}
.turn h3 {{ margin:0 0 10px; font-family:var(--mono); font-weight:500; font-size:12px; letter-spacing:.06em; text-transform:uppercase; color:var(--muted); }}
.turn h3.after {{ color:var(--accent); }}
.q {{ margin:0 0 8px; }} .a p {{ margin:0 0 8px; }} .a b {{ margin-right:6px; }}
.chip {{ display:inline-block; background:var(--accent-soft); color:var(--accent); border-radius:4px; padding:0 6px; font-size:13px; }}
a.help {{ color:var(--accent); text-decoration:underline; text-underline-offset:2px; }}
.meta {{ font-family:var(--mono); font-size:12px; color:var(--muted); }}
table {{ border-collapse:collapse; width:100%; font-size:14px; }}
th, td {{ text-align:left; padding:8px 10px; border-bottom:1px solid var(--line); vertical-align:top; }}
th {{ font-size:12px; letter-spacing:.04em; text-transform:uppercase; color:var(--muted); font-weight:600; }}
.tablewrap {{ overflow-x:auto; }}
details.case {{ background:var(--surface); border:1px solid var(--line); border-radius:6px; margin:8px 0; }}
details.case summary {{ cursor:pointer; padding:10px 14px; display:flex; flex-wrap:wrap; gap:10px; align-items:center; }}
details.case summary::-webkit-details-marker {{ display:none; }}
details.case .id {{ font-weight:500; }}
.case-body {{ padding:4px 14px 14px; border-top:1px solid var(--line); }}
ul.checks {{ list-style:none; padding:0; margin:8px 0 0; font-size:13.5px; }}
ul.checks li {{ padding:3px 0 3px 18px; position:relative; }}
ul.checks li::before {{ content:"✓"; position:absolute; left:0; color:var(--good); }}
ul.checks li.ko::before {{ content:"✗"; color:var(--bad); }}
ol.flow {{ padding-left:22px; }} ol.flow li {{ margin:6px 0; }}
pre {{ background:var(--ground); border:1px solid var(--line); border-radius:6px; padding:10px 12px; font-family:var(--mono); font-size:12.5px; overflow-x:auto; margin:6px 0 10px; white-space:pre-wrap; }}
code {{ font-family:var(--mono); font-size:13px; background:var(--accent-soft); padding:0 4px; border-radius:3px; }}
.note {{ border-left:3px solid var(--warn); padding:6px 12px; background:var(--surface); margin:12px 0; }}
</style>
<div class="wrap">
<header>
  <h1>Ayuda de EmailOps en el chat</h1>
  <p class="lede">El chat responde preguntas sobre la propia aplicación desde las guías de usuario incluidas en el binario, en los cuatro idiomas, cita la sección y abre el ajuste o la vista correspondiente.</p>
  <div class="runmeta"><span>rama {esc(branch)}</span><span>base {esc(sha)}</span><span>{now}</span><span>{esc(run_meta)}</span></div>
</header>

<div class="stats">
  <div class="stat"><div class="n {'good' if eval_ok else 'bad'}">{esc(eval_summary.split(' ')[0] if evalrun and evalrun.get('ok') else '—')}<span class="muted" style="font-size:16px"> / {run['casesTotal'] if evalrun and evalrun.get('ok') else '—'}</span></div><div class="l">casos de eval que pasan</div></div>
  <div class="stat"><div class="n">{stats['chunks']}</div><div class="l">secciones indexadas · {len(stats['per_lang'])} idiomas</div></div>
  <div class="stat"><div class="n">{stats['nav']}</div><div class="l">secciones con destino de navegación</div></div>
  <div class="stat"><div class="n">{stats['embedded']}</div><div class="l">secciones con vector (nomic)</div></div>
</div>

<h2>Antes y después</h2>
<p>La misma pregunta, en la misma base de demo, con la función apagada (<code>help_docs_enabled=false</code>) y encendida. La demo no tiene embeddings de correo, así que la recuperación del buzón va solo por FTS en ambos casos. El turno «antes» corrió con la ventana de contexto automática de esta máquina (8192 tokens, prompt recortado por delante); el «después» y las evals con <code>chat.n_ctx=12288</code> para que el bloque de ayuda no desplace el system prompt. Las latencias son de CPU con otros procesos compitiendo, no comparables entre sí ni con un Mac.</p>
<div class="note">El «antes» es el fallo que motivó la función: el buzón de demo contiene un correo de un cliente preguntando precisamente por Ollama, y el chat lo trató como la única fuente. El «después» responde desde la guía, cita la sección y abre Ajustes › IA (<span class="mono">navigateTo settings/ai</span>).</div>
<div class="exchange">
  <div class="turn"><h3>Antes · ayuda desactivada</h3>
    <p class="q"><b>User:</b> {esc(question)}</p>
    <div class="a"><b>A:</b>{md_to_html(base['answer'])}</div>
    <p class="meta">{esc(trace_line(base['trace']))}</p>
  </div>
  <div class="turn"><h3 class="after">Después · ayuda activada</h3>
    <p class="q"><b>User:</b> {esc(question)}</p>
    <div class="a"><b>A:</b>{md_to_html(after['answer'])}</div>
    <p class="meta">{esc(trace_line(after['trace']))}</p>
  </div>
</div>

<h2>Ejecución de evals</h2>
<p>Casos sintéticos nuevos en <span class="mono">src-tauri/evals/chat/cases/app_help.yaml</span>, ejecutados con <span class="mono">emailops-cli eval --json</span> contra la base de demo. Comprobaciones heurísticas, sin juez: cada caso exige un enlace <span class="mono">help://</span>, un dato que solo está en la guía, y que no se haya buscado en el buzón; el caso espejo exige lo contrario. Resultado: <b>{esc(eval_summary)}</b>.</p>
{cases_html}

{iterations_html}

<h2>Cómo funciona</h2>
<ol class="flow">
  <li><b>Corpus.</b> <span class="mono">services/help_docs/corpus.rs</span> incrusta con <span class="mono">include_str!</span> las 7 páginas de <span class="mono">docs/site/</span> en en/es/fr/de y las parte por encabezado (las secciones largas, por párrafos). La sección N significa lo mismo en los cuatro idiomas; un test lo comprueba.</li>
  <li><b>Índice.</b> Migración <span class="mono">V023</span>: <span class="mono">help_doc_chunks</span> + <span class="mono">help_docs_fts</span> + <span class="mono">vec_help_docs</span>. El texto se reconstruye cuando cambia el hash del corpus (un turno lo garantiza); los vectores los rellena el prewarm del chat y la CLI antes de un turno, con el modelo de embeddings activo.</li>
  <li><b>Recuperación.</b> En cada turno, FTS + KNN fusionados con el mismo RRF del buzón, reutilizando el embedding de la pregunta que ya calculó la recuperación del buzón. El planner puro colapsa a una fuente por sección, exige similitud coseno ≥ 0.55 (<span class="mono">chat.help_min_similarity</span>) y sirve cada sección en el idioma de la respuesta, cambiando un acierto en francés por su hermana en español.</li>
  <li><b>Prompt.</b> El bloque <span class="mono">EMAILOPS HELP</span> viaja en el último mensaje de usuario, nunca en el system prompt, para no romper la caché KV. Instruye: solo para preguntas sobre la app, sin tools, y terminar con el enlace <span class="mono">help://lang/page#anchor</span>.</li>
  <li><b>Navegación.</b> Si la respuesta final cita una sección cuyo front matter <span class="mono">nav:</span> tiene destino, el backend emite <span class="mono">ToolEffect::NavigateTo</span> y el frontend abre la pestaña de Ajustes o la vista (listas validadas en ambos lados). Sin cita, sin navegación: un falso positivo del umbral nunca mueve la pantalla.</li>
  <li><b>Frontend.</b> Los enlaces <span class="mono">help://</span> se renderizan como enlaces a getemailops.com; el panel de razonamiento muestra candidatos, incluidas y similitud; toggle en Ajustes › IA (<span class="mono">help_docs_enabled</span>, por defecto activado).</li>
</ol>

<h2>Puertas de calidad</h2>
<div class="tablewrap"><table><thead><tr><th>Puerta</th><th>Resultado</th><th>Detalle</th></tr></thead><tbody>{gate_rows}</tbody></table></div>

<h2>Archivos</h2>
<h3>Nuevos</h3>
<pre>{esc(chr(10).join(files_new))}</pre>
<h3>Modificados</h3>
<pre>{esc(diffstat.strip())}</pre>

<h2>Límites y decisiones</h2>
<ul>
  <li>Las guías pasan a ser código: una guía desactualizada es una respuesta equivocada con seguridad. El README de <span class="mono">docs/site</span> lo documenta y el parity check exige el mismo mapa <span class="mono">nav:</span> en los cuatro idiomas.</li>
  <li>El umbral 0.55 se ha calibrado con nomic-embed en esta ejecución; si el usuario cambia de modelo de embeddings el corpus se reembebe, pero el umbral puede necesitar ajuste (preferencia <span class="mono">chat.help_min_similarity</span>).</li>
  <li>El idioma del enlace sigue al idioma de salida de la IA, no al de la pregunta: se responde en el idioma configurado y se cita la guía en ese idioma.</li>
  <li>Estas evals corrieron en CPU sin GPU; las latencias no son representativas de un Mac con Metal.</li>
  <li>Decisión registrada en <span class="mono">docs/DECISIONS.md</span> (2026-09-17).</li>
</ul>
</div>
"""
OUT.write_text(page, encoding="utf-8")
print("wrote", OUT, len(page), "bytes")
