---
title: 'Troubleshooting'
description: 'Fixes for the problems people hit most: AI unavailable, slow chat, keyword-only search, sync errors.'
weight: 60
---

## AI features are unavailable

<!-- claim:trbl-ai-features-1 -->
With the **in-app** backend, check that the recommended model finished downloading in
**Settings → AI Backend & Models**. An interrupted download leaves the model unusable —
remove it and download it again.

<!-- claim:trbl-ai-features-2 -->
If you switched to **Ollama**, make sure the daemon is running and reachable at
`http://localhost:11434`, and that you have pulled a model:

```bash
ollama pull llama3.2
ollama pull nomic-embed-text
```

<!-- claim:trbl-ai-features-3 -->
On an **Intel Mac** the in-app AI cannot run at all — it needs an Apple Silicon chip (M1 or
newer), so EmailOps keeps it switched off. Use OpenRouter instead. Ollama will install, but it
gets no GPU acceleration on Intel either, so expect it to be too slow to be pleasant.

## Chat is slow

<!-- claim:trbl-chat-slow-1 -->
Local inference takes real time — on a modest machine, a chat answer can take tens of
seconds. Things that help, in rough order of effect:

1. **Check the model actually fits.** This is the big one. On Windows or Linux, a model
   larger than your GPU's **VRAM** spills over to the CPU and slows down by several times —
   the fix is a smaller model, not more system RAM. On Apple Silicon the comparison is
   against total unified memory. See the
   [model catalog](../ai-features/#the-model-catalog) for the figure per model. <!-- claim:trbl-chat-slow-2 -->
2. **Use a smaller model.** Qwen 3.5 4B is the smallest chat model in the catalog. <!-- claim:trbl-chat-slow-3 -->
3. **Raise "keep model loaded"** in AI settings so the model is not reloaded from disk on
   every question. <!-- claim:trbl-chat-slow-4 -->
4. **Lower the context window** — a smaller window means less to process per turn, and it is
   what to reduce first when a model only just fits. <!-- claim:trbl-chat-slow-5 -->
5. **Turn off thinking mode**, which trades speed for accuracy. <!-- claim:trbl-chat-slow-6 -->

## The GPU is not being used (Windows / Linux)

<!-- claim:trbl-gpu-used-1 -->
The app log says which device a model was loaded onto. A working GPU load looks like:

```
llamacpp: chat model offload — Vulkan0 (Vulkan) has 15 GB free — offloading all layers
```

<!-- claim:trbl-gpu-used-2 -->
If you do not see a line like that, the Vulkan backend found no usable device and quietly
fell back to the CPU — the app still works, just slower. Check, in order:

1. **Your graphics driver.** This is almost always the cause. Install or update the normal
   driver for your card; no CUDA toolkit or vendor SDK is needed. <!-- claim:trbl-gpu-used-3 -->
2. **That Vulkan sees the device.** Run `vulkaninfo --summary` (from `vulkan-tools`). If it
   reports no device, the problem is below EmailOps — fix the driver stack first. <!-- claim:trbl-gpu-used-4 -->
3. **VRAM headroom.** If the log offloads only *some* layers, the model is bigger than the
   card's free VRAM. Pick a smaller model or lower the context window. <!-- claim:trbl-gpu-used-5 -->

<!-- claim:trbl-gpu-used-6 -->
Virtual machines and remote desktops frequently expose no GPU at all, which is expected.

## Search returns keyword results only

<!-- claim:trbl-search-returns-1 -->
Semantic search needs embeddings. Open **Settings → AI Search**, check that the categories
you care about are selected, and let the embedding pass finish. After changing the embedding
model, rebuild the index from the same screen.

<!-- claim:trbl-search-returns-2 -->
Also check **Limit AI processing** in AI settings — mail older than that window is skipped
deliberately.

## Classification is not tagging anything

- Confirm **auto-classify new emails** is on in **Settings → AI Classification**. <!-- claim:trbl-classification-tagging-1 -->
- Check which Gmail categories are selected; if none are, nothing gets classified. <!-- claim:trbl-classification-tagging-2 -->
- For mail that arrived before you enabled it, use **Classify Unclassified**, or
  **Reclassify All** after changing the prompt or rules. <!-- claim:trbl-classification-tagging-3 -->

## Gmail sync stalls or reports rate limits

<!-- claim:trbl-gmail-sync-1 -->
Gmail enforces per-account quotas. When it asks EmailOps to back off, sync pauses that
account until the window reopens and resumes on the next scheduled run — no action needed.
If sync stays broken, remove and re-add the account so a fresh token is issued.

## The app is locked and I forgot the main password

<!-- claim:trbl-app-locked-1 -->
The main password is a local lock with no recovery path — that is the point. Your mail is
still on the server; you can reinstall EmailOps against a fresh data directory and re-sync.

## Something else

<!-- claim:trbl-something-else-1 -->
Check the [open issues](https://github.com/emailops/emailops/issues) and, if your problem is
not there, open a new one. Include your OS and version, the EmailOps version, which AI
backend and model you use, and what you expected to happen.
