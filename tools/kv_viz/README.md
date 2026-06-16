# KV cache visualizer

Single-file browser tool that renders the evolution of llama.cpp's KV cache
across chat turns. Use it to (1) **internalize the three-sequence design** the
embedded backend uses (working prefix, generation scratch, system anchor) and
(2) **debug specific cases** where prefill was cold when it shouldn't have been.

## Quick start

```bash
open tools/kv_viz/index.html      # macOS — or just open the file in any browser
```

Then either:
- Pick a **canned example** from the dropdown (the route-flip example reproduces
  the screenshot bug from the chat where the visualizer was designed).
- **Paste actor log lines** — every line matching `llamacpp kv: …` is parsed.
  Include the surrounding `chat: route: …` / `chat: stage: …` / `chat: llm
  round N` lines so the parser can name the prompts.
- **Drop a bench JSON** (`src-tauri/reports/bench/kv_xconv_*.json`) onto the
  drop hint — coarser than the logs (no `sys` / `stable` info), but works as
  a one-click drop.

## Where to capture actor logs

The visualizer reads the `llamacpp kv:` lines the actor emits on every call.
They are at `info` level, source `ai`, so they show up in:

- **`make dev` output panel** — filter source = `ai`, look for `llamacpp kv:`.
- **`make cli-bench` / `make cli-kv-xconv`** — stderr of the run (`tee` to a
  file, e.g. `make cli-kv-xconv 2>&1 | tee /tmp/kv.log`, then paste).
- **Any `cargo run` of `emailops-cli chat …`** — stderr.

## What you see

### Concepts (inline glossary)

Sourced from the docstrings on `planner.rs`, `actor.rs`, `runtime.rs`. Reads
top-to-bottom as a primer; serves as in-context tooltips elsewhere.

### Prompt inventory

Every cached or uncached call gets a **short name** like:

| Pattern | Name |
|---|---|
| RagFirst final stream, single chat | `c_rag` |
| RagFirst final stream, multi-chat | `c1_rag`, `c2_rag`, … |
| ToolsFirst tool round N | `c_tool_round0`, `c_tool_round1`, … |
| ToolsFirst final synthesis (no model round answered) | `c_tool_synth` |
| Query rewrite (uncached, retrieve stage, <300 tok) | `a_rewrite` |
| Rerank (uncached, retrieve stage, ≥300 tok) | `a_rerank` |
| 1-token startup call | `warmup` |
| Anything else uncached | `a1`, `a2`, … |

The description column auto-summarizes what changed vs the previous cached
prompt: route flips, system-prefix growth, pure suffix extensions, etc.

### Call timeline + cache state after each call

One row per LLM call. For cached calls, three sub-rows:

```
prompt  [🟩 cached LCP ][🟧 decoded on seq 0 ][🟨 volatile tail]   ↤ anchor end (dashed)
seq 0   ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰     ← cache state after the call
seq 2   ▰▰▰▰▰▰▰▰▰▰▰▰                  ← system anchor after the call
```

Plus a Δ line summarizing what changed vs the previous event. Watch for
`🔥 wiped & reseeded` badges — that's the route-flip cold prefill.

Click any row to drill into the **Anatomy** and **Selected call details**
panels below.

### Prompt anatomy & comparison

The anatomy panel breaks one prompt into stability tiers (stable across turns?
across conversations? across routes?). The comparison panel does the same for
two prompts side-by-side and auto-summarizes the divergence (which is how you
spot "the tools= injection adds ~2800 tokens" by eye).

## Encoding

Color legend (also rendered inline under "Concepts"):

| Swatch | Meaning |
|---|---|
| 🟩 Green | cached LCP — reused from seq 0, free |
| 🟧 Orange | decoded on seq 0 — becomes new cache |
| 🟨 Yellow | volatile tail — decoded on seq 1 only, thrown away |
| 🔵 Blue | seq 2 system anchor |
| ⬛ Slate | aux uncached prompt (runs on seq 1 only) |

Badges on each row:

| Badge | Meaning |
|---|---|
| 🌱 fresh | first cached call after a wipe or warmup |
| 🪴 grown | seq 0 extended (LCP > 0, Extend plan) |
| 🌳 anchor hit | RestartFromAnchor — system anchor matched |
| 🔥 wiped & reseeded | ColdPrefill that took out a populated anchor (the bug case) |
| ❄️ evicted resident | aux call with `evict=true` clobbered the resident chat prefix |

## Limitations (v0)

- The bench-JSON drop path is coarser than the log-paste path: the bench
  envelope doesn't carry `sys`, `stable`, or the explicit plan name. The
  log path is preferred when both are available.
- The "cells overlap" math assumes seq 0 and seq 2 always share their
  leading-token cells. That matches the actor's invariant (Extend keeps the
  system prefix, RestartFromAnchor rebuilds from it, ColdPrefill drops both)
  but it's an approximation, not a direct read of the ggml KV.
- Token positions ≠ cells. The page visualizes the planner's mental model,
  which is what decides reuse — not the C++ ggml internals.

## Why this exists

Reading the actor's `llamacpp kv: cached=… sys_cached=… plan=…` lines as a
text stream is hard. Reading them as horizontal bars next to "the anchor was
wiped here" annotations is easy. This page is the bridge.
