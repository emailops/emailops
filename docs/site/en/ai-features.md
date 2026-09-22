---
title: 'AI features'
description: 'Chat with your mailbox, generate replies, classify mail, extract tasks — all running on a model you control.'
weight: 40
nav:
  choosing-a-backend: settings/ai
  the-model-catalog: settings/ai
  performance-knobs: settings/ai
  chat-with-your-mailbox: view/chat
  ai-drafts: settings/aidrafts
  classification: settings/classification
  tag-board: view/tagboard
  semantic-search: settings/aisearch
  translation: settings/aitranslation
  tasks: settings/tasks
  memory: settings/memory
  lenses: settings/lenses
  turning-it-all-off: settings/ai
---

Every AI feature below runs through whichever backend you selected, and each one can be
turned off individually. With the default in-app backend, no prompt or email ever leaves
your machine.

## Choosing a backend {#choosing-a-backend}

**Settings → AI Backend & Models** controls where inference happens:

- **In-app (local)** — an embedded llama.cpp runtime. Nothing to install, no daemon, no
  network traffic. This is the default. It uses your GPU automatically where there is one —
  Metal on Apple Silicon, Vulkan on Windows and Linux — and the CPU where there is not. On a
  Mac it requires Apple Silicon (M1 or newer); on an Intel Mac it stays unavailable.
- **Ollama (local)** — an Ollama server you already run at `http://localhost:11434`. Useful
  if you keep a shared model library. Note that on an Intel Mac it gets no GPU acceleration
  either, so it will be slow.
- **OpenRouter (remote)** — a paid cloud API. Requires an API key, supports a monthly budget
  cap, and sends email content to a third party — so it stays off unless you enable it.

### The model catalog {#the-model-catalog}

The in-app backend downloads models from a curated catalog, each pinned to a verified
checksum:

| Model | Download size | Memory needed to run it |
|---|---|---|
| Qwen 3.5 4B | ~3.0 GB | 8 GB |
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

Models too large for your system memory are greyed out in the picker. One model carries a
**Recommended** badge, chosen for the machine you are on: EmailOps looks at your system memory
and, if you have a discrete graphics card, at its memory too, then suggests the largest model
that fits comfortably. A laptop and a workstation will therefore see different suggestions.
Bigger models answer better and run slower, so the badge is a starting point rather than a
rule. Full requirements are in [Installation](../installation/#with-local-ai).

### Performance knobs {#performance-knobs}

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

## Chat with your mailbox {#chat-with-your-mailbox}

Ask questions in natural language — *"what did the lawyer say about the contract?"*,
*"summarise this thread"*, *"who still owes me an answer?"* — and get an answer with the
source emails cited. Answers stream in as they are generated.

Chat lives in a resizable panel docked to the right of the inbox, so you can keep reading
while you ask; there is also a full-page view for longer sessions. With an email open the
panel offers that thread as context via a removable chip: questions about that email are
answered from the thread, while a question about the rest of your mailbox (*"what came in
today?"*) still searches it. That context applies to a single question and is never saved
onto the conversation, so you can move between emails inside one chat.

Chat searches one account at a time, and a picker names which one — so an answer is never
silently drawn from the wrong mailbox. Each account keeps its own conversation for as long
as the app is open, so switching accounts returns you to where you left off rather than to
a blank chat.

Chat also answers questions about EmailOps itself — *"how do I connect Ollama?"*, *"where is
my data stored?"*, *"what does the Tag Board show?"* — from these guides, in your language,
without searching your mailbox. The answer links the guide section it used, and following
the link opens the matching setting or view. With an email open as context, chat answers
from that thread only, so remove the chip to ask about the app. Turn this off with
**Answer questions about EmailOps** in **Settings → AI Backend & Models**; chat then only
knows your mailbox.

Every answer has a **Show reasoning** panel that lists what happened, in order: which route
the question took and what decided it, the query planner, the mailbox search, the guide
sections used, each model call with its timing, and each tool call with its arguments and
result.

Under the hood, chat combines retrieval (semantic search over your embedded mail) with
tool calls (direct lookups against the database). The routing mode is configurable:

- **Always RAG first** — the default; retrieve context, then answer.
- **Auto** — a heuristic decides per question whether to retrieve first.
- **Always tools first** — skip retrieval and start from structured lookups.

In every mode the tools stay available; the mode only decides whether retrieval runs before
the answer.

Advanced users can edit the system prompt and the retrieval prompts (query rewriting,
reranking) in **Settings → AI Backend & Models → Chat prompts**.

## AI drafts {#ai-drafts}

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

## Tag Board {#tag-board}

The **Tag Board** (under **Views** in the sidebar, next to the inbox) turns those tags into a
board. Pick one dimension — **Company**, **Priority**, **Intent** or **Topic** — and every
tag value becomes a block listing its threads; in **All accounts** you get one block per
account and tag. A thread sits in exactly one block, under the tag of its most recent
classified message.

Blocks are ordered by how much attention a tag actually gets — how often you reply to and
read its threads, weighted towards recent activity — with promotions and notifications
ranked last. The smart filters in the sidebar follow the same order. Drag blocks to reorder
them (the order is remembered per dimension), hide a tag from its ⋮ menu — the next tag moves
up to take its place, and the filter leaves the sidebar too — and bring hidden tags back with
the **Show hidden tags** link.

The toolbar narrows the board by time (**Today**, **Yesterday**, **Last 7 days**, or a custom
date range), by Gmail category, by tag name, and with the same **Hide junk messages** switch
as the inbox; two icons set the block width. Clicking a card opens the thread in the reading
pane, its ⋮ menu offers the same actions as an inbox row, and the chat icon in the reading
pane starts a conversation with that thread as context.

The board needs classification: it is empty until mail has been tagged, and it is not shown
while AI features are off.

## Semantic search {#semantic-search}

Emails are embedded locally so search can match meaning, not just keywords — describe what
you remember and EmailOps finds it. This also powers "find similar" and the retrieval step
in chat. Pick which categories get embedded, and rebuild the index from scratch after
changing the embedding model, in **Settings → AI Search**.

## Translation {#translation}

Translate buttons appear on emails written in another language and in the compose window.
The translation prompt is editable like the others.

## Tasks {#tasks}

*Experimental.* EmailOps scans mail for action items, commitments and deadlines and collects
them in a Tasks panel. Because real commitments usually live in what **you** wrote, there is
a "learn only from emails I wrote" mode. You can exclude senders and tags (newsletters are
excluded by default), cap tasks per email, limit how far back extraction goes, and backfill
older mail on demand.

## Memory {#memory}

*Experimental.* Facts the assistant learns about your contacts, domains and projects are
stored as long-term context so chat does not start from zero every time. Candidate facts are
scored and promoted past a threshold; low-scoring ones expire. Everything it has learned is
inspectable, and the whole subsystem has a master off switch.

## Lenses {#lenses}

*Experimental.* Schema-typed views over your mailbox — saved, AI-extracted structured
projections (think "all invoices with amount and due date") that you create and run from the
sidebar.

## Turning it all off {#turning-it-all-off}

**Settings → AI Backend & Models → AI Features** is a master switch. Turn it off and
EmailOps runs as a plain email client: no chat, no classification, no embeddings, no model
loaded. Your existing local AI data is preserved in case you switch it back on.
