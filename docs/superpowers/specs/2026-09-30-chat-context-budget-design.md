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
JSON 14 739, of which `search_emails` alone is 7 621). `make bench-oneshot-kv` measured a
full chat prompt at 7 466 tokens against the 7 168 an 8 192 window leaves, so at that tier
the system prompt does not fit even with an empty conversation. On the demo mailbox, one
retrieval turn at 8 192 sent 11 961 prompt tokens and the runtime dropped 4 793 from the
front — more than half of the system prompt.

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
- **Estimate:** 3.5 chars per token until a call has been measured (a 20.6k-char chat
  prompt measured 5 224 tokens, 3.95 each). After that, the
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
it through the same function (`turn::system_prompt_inputs`). Target: a system message of
at most 16 000 chars, about 4 096 tokens; a unit test pins it.

- **Template:** `chat.system_compact` (`CHAT_SYSTEM_COMPACT`), a second entry in the
  prompt registry: the tool rules of `chat.system` in one line each, with the email-link
  contract and three of its four examples kept whole (a first version with one-line link
  rules lost the links on tool-result answers at 8k). About 6 300 chars against 11 400. A
  `chat.system` the user customised is kept whatever the window — it is the prompt they
  asked for.
- **Tool catalogue:** `CatalogDetail::Compact`. Each tool is described by its
  `prompt_summary()` and the first sentence of every parameter description, and stated
  once: as its schema in the `<tools>` block on the embedded runtime, or as its summary
  line on a provider that is sent the schemas through its API (those schemas are compact
  too).
- **Measured** on the demo mailbox at 8 192 tokens: a retrieval turn sends 6 112 prompt
  tokens with 0 dropped, against 11 961 with 4 793 dropped before.

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
- Eval: `--n-ctx` pins the window of a run and `n_ctx:` of a case (a process-wide pin in
  `services::ai`; the stored preference is not touched), `previous_questions:` runs
  earlier turns, and the unconditional `prompt_fits_window` check fails a case when any
  call reports `dropped_front_tokens > 0` or the budget could not fit the prompt. New
  public cases in `evals/chat/cases/context_budget.yaml`, on three ~12k-char digests added
  to the demo generator: a fact deep in a long email at 8k, a full-body read at 8k (the
  tool result is cut), and a follow-up on the fourth retrieval turn at 16k (earlier
  questions lose their emails).
- Gate: the smoke tier at the default window is unchanged; the smoke tier at 8k is
  compared before and after.

Results on 2026-09-30, embedded runtime, `qwen3.5-4b-q4_k_m`, demo mailbox, smoke tier
(41 cases), `emailops-cli eval --tier smoke [--n-ctx 8192]`:

| Window | Build | Passed | Cases with a truncated prompt | Tokens dropped |
|---|---|---|---|---|
| default (32 768) | before | 38 | 0 | 0 |
| default (32 768) | after | 38 | 0 | 0 |
| 8 192 | before | 36 | 39 | 107 876 |
| 8 192 | after | 36 | 0 | 0 |

The default-window runs fail the same three cases before and after
(`app_help_add_account_es`, `sf_hetzner_may_servers`, `at_flyio_march_invoice`). At 8 192
the new build also fails `app_help_mailbox_question_stays_mailbox` (the answer names the
email without its link) and `partial_list_states_the_total_es`. The 8k result is not
stable at this model size: three runs of the compact prompt with small wording changes
passed 34, 37 and 36, with different cases failing each time. What is stable is that no
prompt was truncated in any of them.

## Known limits

- Before the embedded model has loaded, its live window is unknown and the configured
  tier stands in; the KV-fit clamp can make the real window smaller.
- The first turn of a conversation estimates at 3.5 chars per token; these prompts
  measure about 3.95, so it cuts somewhat early.
