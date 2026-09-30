# Chat context budget — design

Date: 2026-09-30. Branch: `feature/chat-context-budget` (off `fix/review-hardening`).

## Problem

A chat turn sends the system prompt, the conversation history, the retrieved emails and
the tool results of the turn. Nothing checked that they fit the model's window.

- Embedded llama.cpp: `plan_prompt_budget` (`ai/llama_cpp/planner.rs`) drops tokens from
  the FRONT of the prompt. The system prompt — rules, date, tool catalogue — goes first.
- Ollama truncates on its side and reports nothing. OpenRouter is sent whatever there is.
- The user sees an answer that ignores the rules. The only trace is
  `dropped_front_tokens` and one log warning per actor lifetime.

Measured on 2026-09-30 with the default registry on an empty test DB: the system message
is 29 332 chars (template 11 513, identity ~1 200, tool summary lines 1 901, `<tools>`
JSON 14 739, of which `search_emails` alone is 7 621). `NEXT-SESSION.md` records the full
chat prompt at 7 466 tokens against the 7 168 an 8 192 window leaves, so at that tier the
system prompt does not fit even with an empty conversation.

## Goal

No turn loses its system prompt to truncation. When the prompt would not fit, it is cut
in a decided order, and the user is told when the cut can affect the answer.

## Decisions (developer, 2026-09-30)

1. One change: a budget planner for every window, plus a compact system prefix for
   windows under 16 384 tokens.
2. History gives way in two steps: earlier questions first lose the emails they were
   asked with, then whole exchanges go.
3. Every cut is in the reasoning trace. A note under the answer appears only when
   something of THIS turn was cut, or the prompt did not fit at all.
4. Sizes are estimated from chars and corrected with the provider's own token counts.
   No tokenizer access is added to the `AIProvider` trait.

## Planner — `services/chat/budget.rs` (pure)

- **Window:** `research::resolve_n_ctx` (already resolves all three providers and the
  remote budget).
- **Reply reserve:** `n_ctx / 8`, clamped to 1 024..4 096. A further 256 tokens cover
  chat-template tokens and estimate error.
- **Estimate:** 3 chars per token until a call has been measured. After that, the
  measured ratio (clamped to 2..5), and within a turn the provider's last count plus an
  estimate of what changed since. `LlmCallTrace.prompt_chars` carries the ratio to the
  next turn. A count that is implausible for any tokenizer (under 1.5 or over 8 chars per
  token) is ignored.
- **Order of cuts**, each applied only until the prompt fits:

  | Step | What | How |
  |---|---|---|
  | — | System prompt, question, memory/help/skill/view blocks | never |
  | 1 | Emails of earlier questions | the question replays under a one-line note, oldest first, down to 75 % of the budget |
  | 2 | Earlier exchanges | dropped whole, oldest first |
  | 3a | The open email thread | read again into what is left, floor 2 000 chars |
  | 3b | This turn's retrieved emails | shorter excerpts shared evenly (floor 600 chars), then fewer emails from the end of the list; one always stays |
  | 4 | This turn's tool results | cut to one shared cap (floor 1 500 chars), each ending with a note that says how much is missing |

- A prompt that still does not fit is sent anyway. The runtime's own truncation remains
  the safety net, and the trace says `fits: false`.
- Step 1 is stored: the row's `prompt_content` becomes the stripped form, so later turns
  extend this prompt instead of cutting the same emails again. No migration. The only
  other reader of `prompt_content` is the eval harness, which reads the current turn.
- The existing caps of 6 (12 for a thread-bound chat) replayed messages stay as upper
  bounds.

## Where it hooks in — `services/chat/turn.rs`

- **First prompt** (standard, open-email and shortcut turns): the prompt is built uncut,
  `plan_first_prompt` measures it, and only a plan with cuts triggers a second build
  through `build_prompt_fitted`. A turn that fits sends the same bytes as before.
- **Tool loop:** `TurnBudget::fit` runs before every model call in `run_tool_loop` and in
  `synthesize_with_recovery`, so synthesis, recovery and the retries are covered.
- **Thread-bound chat:** the seeded thread rides in the system message, so only steps 2
  and 4 apply.
- **HTTP providers** are also sent the tool schemas as `tools`; their chars count against
  the window. The embedded runtime reads tools from the system prompt only.
- **Out of scope:** research mode (sizes its own prompts) and the one-shot completions
  (query planner, rewrite, rerank, form fill).

## Compact system prefix (windows under 16 384)

Chosen by the window, not per turn, so the KV anchor stays byte-stable; prewarm selects
it with the same function. Target: a system message of at most 4 096 measured tokens.
What is removed is decided by measurement and by the eval — see the implementation notes
at the end of this file once that step lands.

## What the user sees

- `ChatTrace.budget` (`BudgetTrace`): window, reply reserve, largest prompt estimate, the
  cuts, and whether the prompt fit. Absent when nothing was cut. Shown in the reasoning
  trace and in `emailops-cli chat --trace`.
- A note under the answer when `BudgetTrace::affects_answer()` — steps 3–4, or a prompt
  that did not fit. It links to Settings → AI.

## Verification

- Table-driven unit tests for the planner and for `TurnBudget`.
- Turn-level tests against `FakeAiProvider`: the system message reaches the model whole,
  a prompt that fits is unchanged, stripped questions are stored.
- Eval: `--n-ctx` to pin the window of a run, earlier turns in a case, and a metric that
  fails a case when any call reports `dropped_front_tokens > 0`. New public cases: a long
  newsletter at 8k, a follow-up on the fourth retrieval turn, several full-body reads.
- Gate: the smoke tier at the default window is unchanged; the smoke tier at 8k is
  compared before and after.

## Known limits

- Before the embedded model has loaded, its live window is unknown and the configured
  tier stands in; the KV-fit clamp can make the real window smaller.
- The first turn of a conversation estimates at 3 chars per token; mail measures 3.5–4,
  so it cuts somewhat early.
