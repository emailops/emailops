---
title: 'AI features'
description: 'Chat with your mailbox, generate replies, classify mail, extract tasks — all running on a model you control.'
weight: 40
---

Every AI feature below runs through whichever backend you selected, and each one can be
turned off individually. With the default in-app backend, no prompt or email ever leaves
your machine.

## Choosing a backend {#choosing-a-backend}

**Settings → AI Backend & Models** controls where inference happens:

- **In-app (local)** — an embedded llama.cpp runtime. Nothing to install, no daemon, no
  network traffic. This is the default. It uses your GPU automatically where there is one —
  Metal on Apple Silicon, Vulkan on Windows and Linux — and the CPU where there is not.
- **Ollama (local)** — an Ollama server you already run at `http://localhost:11434`. Useful
  if you keep a shared model library, or on Intel Macs where the embedded runtime is absent.
- **OpenRouter (remote)** — a paid cloud API. Requires an API key, supports a monthly budget
  cap, and sends email content to a third party — so it stays off unless you enable it.

### The model catalog {#the-model-catalog}

The in-app backend downloads models from a curated catalog, each pinned to a verified
checksum:

| Model | Download size | Memory needed to run it |
|---|---|---|
| Qwen 3.5 4B *(recommended)* | ~3.0 GB | 8 GB |
| Qwen 3.5 4B Q8 | ~4.6 GB | 12 GB |
| Qwen 3.5 9B | ~5.7 GB | 16 GB |
| Gemma 4 12B Instruct | ~6.7 GB | 16 GB |
| Qwen 3.5 27B | ~17.6 GB | 24 GB |
| Qwen 3.6 35B A3B | ~22.4 GB | 32 GB |
| Nomic Embed Text v1.5 *(embeddings, bundled)* | ~84 MB | 1 GB |

The right-hand column is peak memory while answering — weights plus the context window —
which is always more than the download. **Which** memory it has to fit in depends on your
hardware:

- **Apple Silicon** — unified memory, shared between CPU and GPU, reached through Metal.
  Compare the figure against your Mac's total memory.
- **A GPU on Windows or Linux** — the card's **VRAM**, not your system RAM, reached through
  Vulkan. An 8 GB card runs the 8 GB row and nothing above it, however much RAM the machine
  has.
- **No GPU** — system RAM, on the CPU. It works; it is just slower.

Models too large for your system memory are greyed out in the picker. Bigger models answer
better and run slower — start with the recommended one and move up only if the hardware has
headroom. Full requirements are in [Installation](../installation/#with-local-ai).

### Performance knobs

- **Keep model loaded** — how long the model stays resident between turns (default 30
  minutes). Higher values skip the slow reload; `0` evicts it immediately and frees the
  memory for other apps.
- **Context window** — how many tokens the model can attend to per turn. Larger fits more
  retrieved email, and costs more memory — this is the knob to turn down first when a model
  only just fits.
- **Thinking mode** — chain-of-thought reasoning on supported models. Slower, more accurate,
  and you can show or hide the reasoning trace.
- **Limit AI processing to recent emails** — skip embedding and classification for mail
  older than N days.

## Chat with your mailbox

Ask questions in natural language — *"what did the lawyer say about the contract?"*,
*"summarise this thread"*, *"who still owes me an answer?"* — and get an answer with the
source emails cited. Answers stream in as they are generated.

Under the hood, chat combines retrieval (semantic search over your embedded mail) with
tool calls (direct lookups against the database). The routing mode is configurable:

- **Always RAG first** — the default; retrieve context, then answer.
- **Auto** — a heuristic picks retrieval or tools per question.
- **Always tools first** — go straight to structured lookups.

Advanced users can edit the system prompt and the retrieval prompts (query rewriting,
reranking) in **Settings → AI Backend & Models → Chat prompts**.

## AI drafts

An **AI Draft** button next to Reply All writes a reply grounded in the thread you are
looking at. Configure a **persona** (one sentence on who the AI writes as), a **writing
style**, and default tone and length — or replace the whole prompt template. Drafts land in
the composer for you to review before anything is sent.

## Classification {#classification}

Every incoming email is tagged along three axes — **priority**, **intent** and **topic** —
so the inbox effectively sorts itself and smart filters have something to filter on.

Classification works in two layers:

1. **Rules** match on sender or subject patterns (`*@*.beehiiv.com`, `*invoice*`) and assign
   tags instantly, with no model call.
2. **The model** handles everything the rules do not, using an instruction prompt you can
   edit.

You control which Gmail categories are classified, can reclassify everything after changing
the prompt, and can catch up on unclassified mail on demand.

## Semantic search

Emails are embedded locally so search can match meaning, not just keywords — describe what
you remember and EmailOps finds it. This also powers "find similar" and the retrieval step
in chat. Pick which categories get embedded, and rebuild the index from scratch after
changing the embedding model, in **Settings → AI Search**.

## Translation

Translate buttons appear on emails written in another language and in the compose window.
The translation prompt is editable like the others.

## Tasks

*Experimental.* EmailOps scans mail for action items, commitments and deadlines and collects
them in a Tasks panel. Because real commitments usually live in what **you** wrote, there is
a "learn only from emails I wrote" mode. You can exclude senders and tags (newsletters are
excluded by default), cap tasks per email, limit how far back extraction goes, and backfill
older mail on demand.

## Memory

*Experimental.* Facts the assistant learns about your contacts, domains and projects are
stored as long-term context so chat does not start from zero every time. Candidate facts are
scored and promoted past a threshold; low-scoring ones expire. Everything it has learned is
inspectable, and the whole subsystem has a master off switch.

## Lenses

*Experimental.* Schema-typed views over your mailbox — saved, AI-extracted structured
projections (think "all invoices with amount and due date") that you create and run from the
sidebar.

## Turning it all off

**Settings → AI Backend & Models → AI Features** is a master switch. Turn it off and
EmailOps runs as a plain email client: no chat, no classification, no embeddings, no model
loaded. Your existing local AI data is preserved in case you switch it back on.
