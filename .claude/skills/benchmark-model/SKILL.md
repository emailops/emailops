---
name: benchmark-model
description: Measure a chat model against the models EmailOps already runs — accuracy, speed and peak memory across the classifier, query planner, chat and draft evals — and say which one should ship. Use whenever the user names a model they want to try, compare, or switch to (a GGUF, a HuggingFace repo, "is X better than what we run", "should we move to Y", "benchmark Z"), or asks how a model performs on their mailbox. Also use before changing the default chat model, and whenever a model appears to produce empty, garbled or obviously poor answers — that is far more often an integration mismatch (chat template, thinking-token priming, an unsupported quant) than a bad model, and this skill checks those first so a working model is not dismissed on a bug.
argument-hint: <model name, GGUF url, or "X vs Y">
allowed-tools: Bash, Read, Edit, Write, Grep, Glob, WebSearch, WebFetch
---

# Benchmark a chat model

You are deciding whether a model should replace or join what EmailOps runs
today. The deliverable is a table someone can act on — accuracy, speed and
memory, side by side — plus a recommendation and the caveats that make the
numbers honest.

The single most important thing this skill protects against: **a model that
looks bad because of how it was wired in, not because of how it is.** That has
happened twice in this repo already (Gemma 4's chat template, and the
thinking-token priming). Phase 2 exists entirely for that reason — do not skip
it, and never report "model X is weak" without having run it.

## Phase 0 — Frame and preflight

Establish, in a couple of lines each:

- **Which models.** The candidate, and what it is being compared against —
  default to the model in `make cli-fast ARGS="doctor --json"` plus any other
  local ones worth a column (`ls "$EMAILOPS_DEMO_DIR/models/chat"`).
- **Does it exist in the shape the user thinks?** Users name models from
  memory ("the 27B q4"). Check what is actually published before planning
  around it — quant names, file sizes, licence. A model may ship only formats
  stock llama.cpp cannot read (see Phase 1).
- **Room to work.** A chat GGUF is 3-22 GB and `src-tauri/target` grows past
  60 GB. Check `df -h .` before downloading. If disk is short, say so and ask
  before deleting anything — `src-tauri/target` is the usual candidate and it
  is the user's call, not yours.
- **RAM.** `sysctl -n hw.memsize`. A 22 GB model on a 16 GB machine is not a
  benchmark, it is a swap test.

## Phase 1 — Get the weights, and prove they are the weights

Models live at `<data_dir>/models/chat/<model_id>.gguf` — the id is the file
name, and `--model <id>` on every harness resolves through it
(`model_manager::model_path`). The benchmark uses the demo data dir, which the
Makefile defines as `$(CURDIR)/.emailops-demo-data`; export it once so the
snippets below work outside `make`:

```bash
export EMAILOPS_DEMO_DIR="$PWD/.emailops-demo-data"
```
 No catalog entry is needed to benchmark; the
catalog (`ai/model_catalog.rs`) is for the in-app downloader, and adding an
entry is a separate, shippable decision.

Download to the demo data dir, then **verify the checksum** against the
publisher's API before spending an hour measuring a corrupt file:

```bash
curl -s "https://huggingface.co/api/models/<org>/<repo>/tree/main" \
  | python3 -c "import json,sys; [print(f['path'], f.get('size'), (f.get('lfs') or {}).get('oid')) for f in json.load(sys.stdin)]"
shasum -a 256 "$EMAILOPS_DEMO_DIR/models/chat/<id>.gguf"
```

Read the GGUF header too — `general.architecture`, `general.file_type` and the
context length tell you what you are about to run, and the architecture is
what decides the template and priming questions in Phase 2.

**If stock llama.cpp cannot load the file** (a vendor's private quant type, a
custom activation runtime), that is a fork port, not a download. It is usually
feasible — a vendor fork's public C API tends to be a strict superset of the
upstream the bindings were generated from, so only `llama-cpp-sys-2`'s own C++
shim needs patching, and `--config 'patch.crates-io...'` applies it per command
without changing a committed file (`LLAMA_PATCH_DIR` in the harness). But it is
hours of work and the fork then serves every model, not just the candidate.
Surface that cost and let the user choose before starting.

## Phase 2 — One case first: is it wired in correctly?

Before any sweep, run **one** case and look at the answer:

```bash
EMAILOPS_DATA_DIR="$EMAILOPS_DEMO_DIR" \
  src-tauri/target/debug/examples/tag_classification_eval \
  --prod-db "$EMAILOPS_DEMO_DIR/emailops.db" --model <id> \
  --case en_billing_invoice_overdue --json
```

A sweep on a mis-wired model costs an hour and produces a confident, wrong
verdict. These are the failure signatures and what they actually mean:

| Symptom | Almost certainly | Where to look |
|---|---|---|
| `call failed: empty reply`, every case | The model is a reasoning model that never got the no-think primer, so it spent the whole generation budget inside `<think>` and `strip_reasoning` collapsed the reply to `""` | `no_think_priming` / `is_qwen3_model_path` (`ai/llama_cpp/runtime.rs`) — the primer is chosen by **file name** (`starts_with("qwen3")`), not by architecture |
| Load fails with `ffi error -1` on the template | The GGUF's chat template uses delimiters `llama_chat_apply_template` does not know | `looks_like_gemma4_template` + the hand-rolled render in `runtime.rs` |
| Loads, answers fluent nonsense | An unsupported quant read as a known one, or a missing activation transform | The vendor's model card — quant ids and any required fork |
| `Decode Error -3` on every turn | Embedded runtime on hardware it cannot use | `ai::gpu_plan::embedded_runtime_supported` |

For the priming case the workaround costs nothing and keeps the comparison
fair — hard-link the file under a name the heuristic recognises, and say in
the report that you did:

```bash
ln -f <id>.gguf qwen35-<id>.gguf   # same inode, no extra disk
```

Report the wiring gap as a finding in its own right. "Any Qwen-family GGUF not
named `qwen3*` returns empty replies" is a product bug worth more than the
benchmark that uncovered it.

## Phase 3 — Sweep

```bash
make bench-models ARGS="--models <a>,<b> --tier smoke --drafts 6 --repeats 1"
```

It runs the four evals that move a reply — classifier (145 labelled synthetic
cases), query planner (21), chat (32 at `smoke`) and drafts — and writes a
text table and an HTML page under `reports/bench/`. Binaries are built once
and run directly, so the sampled RSS belongs to the process that loads the
model. Expect 30-60 minutes per model; run it in the background and say so.

Fairness is the whole value of the exercise, so hold everything else still:

- **One build for every column.** Rebuilding between models lets a toolchain
  change masquerade as a model difference.
- **Same context window and same priming.** Both are per-model-file decisions
  — confirm both models actually got them.
- **Same corpus, same tier, same repeat count**, and note them in the report.
- The Rust side is a debug build, so latencies are comparable **between
  columns** but are not absolute numbers to quote elsewhere.

## Phase 4 — Read the results honestly

Three things decide a model, and they rarely point the same way:

- **Accuracy.** Differences of a case or two out of 145 are noise; say so
  rather than ranking on them. Look for where a model is *systematically*
  better — one axis, one suite — and check whether the suites that failed are
  the same cases for both (they usually are, which makes the comparison fair
  and the failures a corpus question, not a model one).
- **Speed.** A mixture-of-experts model with few active parameters will beat a
  dense model of similar size by 2-3x. That is architecture, not quality.
- **Memory.** Peak RSS is what decides whether a model is even an option on a
  16 GB machine. A model that matches on quality at half the memory has a real
  argument even when it is slower.

State the recommendation for **this** machine and separately for the smallest
machine the app supports — they often differ, and that difference is usually
the most useful sentence in the report.

Carry the caveats into the report itself, not just the chat: the judge
confound below, the debug build, the tier sizes, and any wiring workaround.

## Harness quirks worth knowing

These are properties of the evals, not bugs to fix mid-benchmark:

- **`chat_eval` writes no JSON** — only an HTML report and `[eval] OK/FAIL`
  lines on stderr. The harness counts those lines.
- **`draft_eval` judges with the same model that generated**, and takes the
  model from `EMAILOPS_EVAL_MODEL` rather than `--model`. Each model therefore
  grades its own homework: report only its deterministic metrics (word
  overlap, latency, errors) and say why the judge scores are absent.
- **Eval binaries must exit through `services::ai::shutdown_and_exit`** or
  ggml's Metal destructor aborts at teardown and a finished run reports
  failure. `chat_eval` and `draft_eval` still return from `main`, so ignore
  their exit codes and read their output.
- **The planner corpus was calibrated on a small model.** Bigger models
  failing 2 of 21 is a signal about the cases, not necessarily the model.

## Report

Lead with the table, then the reading. Keep it to what changes a decision:

```
| métrica | <modelo A> | <modelo B> |
  accuracy per suite · ms per unit · peak RSS

Calidad: <tie / where each wins, with the sample size>
Velocidad: <factor, and the architectural reason>
Memoria: <peak RSS, and which machines that rules in or out>
Recomendación: <for this machine> / <for the smallest supported machine>
Avisos: <wiring workarounds, debug build, tier sizes, judge confound>
```

Give the text and HTML paths as `file:///…` URLs.

If the conclusion is "switch the default model", that is a durable decision:
append an entry to `docs/DECISIONS.md` with the numbers and what was rejected.
A benchmark that changes nothing still earns a note in `NEXT-SESSION.md` —
"we measured X and it was not worth it" saves the next person the hour.
