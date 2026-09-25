"""How report_all.py renders one chat eval case: its flow, its trace, its judge.

The AI trace arrives as the ChatTrace JSON the CLI emits, with `steps` already
planned by `services::chat::trace_steps`. The labels and details below mirror
`step_label` / `step_detail` there, so the verify report reads like the chat
eval report and `emailops-cli chat --trace`.
"""
import html, json

# A prompt can run to ~35k chars; a block beyond this is cut with a note.
BLOCK_CAP = 40000


def E(s): return html.escape(str(s))


def _llm(t, step):
    calls = t.get("llmCalls") or []
    i = step.get("index")
    return calls[i] if isinstance(i, int) and 0 <= i < len(calls) else None


def _tool(t, step):
    calls = t.get("toolCalls") or []
    i = step.get("index")
    return calls[i] if isinstance(i, int) and 0 <= i < len(calls) else None


def step_label(t, step):
    kind = step.get("type")
    if kind == "route":
        r = t.get("route") or {}
        kws = r.get("matchedKeywords") or []
        return f"route: {r.get('classifier', '?')}" + (f" (matched: {', '.join(kws)})" if kws else "")
    if kind == "retrieval":
        return "RAG retrieval"
    if kind == "help":
        h = t.get("help")
        return f"guides ({h.get('included', 0)} of {h.get('candidates', 0)} sections)" if h else "guides"
    if kind == "llm":
        c = _llm(t, step)
        if not c: return "llm call"
        return {"planner": "planner", "final_stream": "answer"}.get(c.get("kind"), f"llm round {c.get('round')}" if c.get("kind") == "tool_round" else f"llm call ({c.get('kind')})")
    if kind == "tool":
        c = _tool(t, step)
        return c.get("name", "tool") if c else "tool"
    return str(kind)


def step_detail(t, step):
    kind = step.get("type")
    if kind == "route":
        r = t.get("route") or {}
        mode, reason = r.get("mode", ""), r.get("reason", "")
        return f"{mode} · {reason}" if reason else mode
    if kind == "retrieval":
        r = t.get("retrieval")
        if not r: return ""
        return f"{r.get('vectorHits', 0)} vec + {r.get('ftsHits', 0)} fts → top {r.get('fusedTopK', 0)} · {r.get('elapsedMs', 0)} ms" + (" · vector fallback" if r.get("vectorFallback") else "")
    if kind == "help":
        h = t.get("help")
        if not h: return ""
        parts = [f"sim {h['topSimilarity']:.2f}"] if isinstance(h.get("topSimilarity"), (int, float)) else []
        return " · ".join(parts + [f"{h.get('elapsedMs', 0)} ms"])
    if kind == "llm":
        c = _llm(t, step)
        if not c: return ""
        parts = [f"{c.get('latencyMs', 0)} ms"]
        if c.get("prefillMs") is not None: parts.append(f"prefill {c['prefillMs']} ms")
        k = step.get("kvCache")
        if k: parts.append(f"KV cache {k['cached']}/{k['total']} tok ({k['pct']}%)")
        a = step.get("cacheAction")
        if a: parts.append(a.get("detail", ""))
        n = c.get("toolCallsRequested") or 0
        if n: parts.append("1 tool call" if n == 1 else f"{n} tool calls")
        if c.get("failed"): parts.append("FAILED")
        return " · ".join(parts)
    if kind == "tool":
        c = _tool(t, step)
        return f"{c.get('elapsedMs', 0)} ms · {c.get('resultChars', 0)} chars" if c else ""
    return ""


def flow_summary(t):
    """`route: … → planner → search_emails → answer`, or None without steps."""
    if not t or not t.get("steps"): return None
    return " → ".join(step_label(t, s) for s in t["steps"])


def _block(title, text):
    text = str(text)
    cut = f"\n… ({len(text) - BLOCK_CAP} caracteres más)" if len(text) > BLOCK_CAP else ""
    return f'<details class="blk"><summary>{E(title)} <span class="muted">{len(text)} caracteres</span></summary><pre>{E(text[:BLOCK_CAP] + cut)}</pre></details>'


def _blocks(t, step):
    kind = step.get("type")
    if kind == "llm":
        c = _llm(t, step) or {}
        return "".join(_block(k, c[k]) for k in ("input", "output") if c.get(k))
    if kind == "tool":
        c = _tool(t, step)
        if not c: return ""
        return _block("arguments", json.dumps(c.get("arguments"), ensure_ascii=False, indent=2)) + _block("result",c.get("resultPreview", ""))
    return ""


def steps_html(t):
    """One row per step (label, numbers) with its prompt / output / tool I/O
    as collapsed plain-text blocks: real line breaks, no JSON escaping."""
    rows = "".join(
        f'<li class="step"><div class="stephead"><span class="steplabel">{E(step_label(t, s))}</span><span class="muted">{E(step_detail(t, s))}</span></div>{_blocks(t, s)}</li>'
        for s in t.get("steps") or [])
    return f'<ol class="steps">{rows}</ol>'


def judge_checks(jr):
    """The judge's verdict as rows of the checks table: one per scored metric
    against the threshold, or one failed row when the judge itself errored."""
    if not jr: return []
    sc = jr.get("scores") or {}
    thr = jr.get("threshold", 0.7)
    rationale = sc.get("rationale") or ""
    if sc.get("error"):
        return [{"name": "juez", "expected": f"≥ {thr:.2f}", "actual": "error", "passed": False, "detail": sc["error"]}]
    return [{"name": f"juez · {k}", "expected": f"≥ {thr:.2f}", "actual": f"{v:.2f}", "passed": v >= thr, "detail": rationale}
            for k, v in sc.items() if k not in ("rationale", "error") and isinstance(v, (int, float))]
