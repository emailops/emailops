---
title: 'Getting started'
description: 'The first-run wizard: choose an AI backend, download a model and connect your first mailbox.'
weight: 20
---

<!-- claim:start-intro-1 -->
The first time you open EmailOps a wizard of up to four steps runs — three if you choose a
plain email client. It takes a couple of minutes, most
of which is a model download in the background.

## 1. AI on or off

<!-- claim:start-1-ai-1 -->
EmailOps inspects your hardware and recommends whether to enable local AI. Pick:

- **Use AI** — chat, drafts, classification and semantic search all run on this machine. <!-- claim:start-1-ai-2 -->
- **Plain email client** — no model is downloaded and no AI call is ever made. You can turn
  AI on later in **Settings → AI Backend & Models**, and turn it off again just as easily. <!-- claim:start-1-ai-3 -->

## 2. AI backend and model

<!-- claim:start-2-ai-1 -->
If you enabled AI, choose where inference happens:

| Backend | What it means |
|---|---|
| **In-app** | The default. A llama.cpp runtime embedded in EmailOps. No daemon, no setup, no network. |
| **Ollama** | Uses your existing Ollama server at `http://localhost:11434`. |
| **OpenRouter** | Sends prompts to a paid cloud API. Opt-in, per feature, and off by default. |

<!-- claim:start-2-ai-2 -->
With the in-app backend, pick a chat model from the built-in catalog. EmailOps preselects the
largest model your machine can comfortably run, so the recommendation depends on the memory
it finds — on a 16 GB machine that is **Qwen 3.5 4B**, about 3 GB to download and roughly
8 GB of memory to run; a roomier machine is offered a larger model from the same catalog.
Every recommended model supports the tool-calling that chat relies on. Models too large for
your system memory are greyed out. The download runs in the background — you can carry on
with the wizard.

<!-- claim:start-2-ai-3 -->
The memory that counts depends on the machine: **unified memory** on an Apple Silicon Mac,
your **GPU's VRAM** on a Windows or Linux box with a discrete card, and system RAM if there
is no GPU. The [model catalog](../ai-features/#the-model-catalog) lists the figure for each
model.

<!-- claim:start-2-ai-4 -->
The embedding model that powers semantic search (**Nomic Embed Text v1.5**, ~80 MB) ships
inside the app on macOS, so there is nothing to download for search.

## 3. Inbox layout

<!-- claim:start-3-inbox-1 -->
Choose how the mailbox is laid out — **split** (list on the left, message on the right) or
**full width** (one pane at a time). Change it whenever you like in **Settings → Appearance**,
along with the interface language (English, Spanish, French, German).

## 4. Connect an account

<!-- claim:start-4-connect-1 -->
The last step adds your first mailbox. EmailOps supports:

- **Gmail** — sign in through your browser and grant access. Tokens go straight into your
  OS keychain. <!-- claim:start-4-connect-2 -->
- **Outlook / Microsoft 365** — same browser flow, via the Microsoft Graph API. <!-- claim:start-4-connect-3 -->
- **IMAP / SMTP** — iCloud, Yahoo, Fastmail, ProtonMail Bridge or any custom server. Enter
  the server details and credentials directly. <!-- claim:start-4-connect-4 -->

<!-- claim:start-4-connect-5 -->
Add more accounts any time with **Add account** in the sidebar. With several connected you get a
unified "All accounts" inbox on top of the per-account views.

## After the wizard

### The first sync takes a while

<!-- claim:start-after-wizard-first-sync-1 -->
EmailOps downloads your mail into a local database, and the first pass has to fetch
everything from scratch. How long that takes depends on the size of the mailbox — a few
minutes for a small account, considerably longer for one with years of history and heavy
attachments. It runs in the background, and the first messages appear within seconds — mail is
downloaded in slices as the mailbox is listed, rather than after the whole history has been
scanned — so you can read and search what has already arrived while the rest catches up.

<!-- claim:start-after-wizard-first-sync-2 -->
This is a one-time cost. Every later sync is **incremental**: it asks your provider only for
what changed since last time, so it finishes in seconds and runs quietly on a schedule. If
AI is enabled, classification and embedding also work through the backlog on first run and
then only touch new mail.

<!-- claim:start-after-wizard-first-sync-3 -->
Once the first sync finishes:

1. **Classification** starts tagging new mail by priority, intent and topic — see
   [AI features](../ai-features/#classification). <!-- claim:start-after-wizard-first-sync-4 -->
2. **Embeddings** are generated in the background so semantic search has something to search.
   You can watch progress and rebuild the index in **Settings → AI Search**. <!-- claim:start-after-wizard-first-sync-5 -->
3. Consider setting a **main password** in **Settings → Privacy & Security** if you want the
   app locked on startup — see [Privacy & security](../privacy-security/). <!-- claim:start-after-wizard-first-sync-6 -->

<!-- claim:start-after-wizard-first-sync-7 -->
Both classification and embedding respect **Limit AI processing**
(**Settings → AI Backend & Models**),
so a decade-old archive does not get processed unless you ask for it.
