# Migrate llama-cpp-2 0.1.146 → 0.1.147 — fast path, single PR, no intermediate evals

## Goal

Bump `llama-cpp-2` to 0.1.147 and replace every removed OpenAI-compat surface
(`apply_chat_template_oaicompat`, `parse_response_oaicompat`,
`OpenAIChatTemplateParams`, `ChatTemplateResult`) with equivalents we control,
in a single commit-ready branch.

## Scope: deliberately tight

- **Primary model**: Qwen 3.5 (4B / 9B). All design choices optimise for it.
- **Secondary model**: Gemma 4 (E2B / 12B). Keep it working with the
  GGUF-embedded template; the hand-rolled `GEMMA4_CHAT_TEMPLATE` fallback for
  mradermacher's stripped builds is **dropped** in this migration — failure
  there will be an explicit error, not silent quality loss. The fallback can
  be re-added in a follow-up if anyone runs into it.
- **Out of scope** for this PR: speculative decoding, kv-cache cell-sharing
  bugfixes wiring, ggml 0.10.0-specific changes, Metal perf knob retuning.
  All of those land for free with the bump but we don't actively use them.

## What changes, by file

### 1. `src-tauri/Cargo.toml` (1 line)

```toml
llama-cpp-2 = { version = "0.1.147", optional = true, default-features = false }
```

Plus the matching `Cargo.lock` regen via `cargo update -p llama-cpp-2`.

### 2. `src-tauri/src/ai/llama_cpp/runtime.rs` (the bulk of the work)

Three things go away, three things replace them.

**Gone:**
- `apply_chat_template_oaicompat` call
- `parse_response_oaicompat` call
- `OpenAIChatTemplateParams` / `ChatTemplateResult` types
- The `GEMMA4_CHAT_TEMPLATE` const (deleted for now — see Scope)

**Replaced by:**

(a) **A new `render_template` that uses the plain `apply_chat_template`**:

```rust
async fn render_template(
    model: Arc<LlamaModel>,
    messages: &[AiMessage],
    add_generation_prompt: bool,
) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        let tmpl = model.chat_template(None)
            .map_err(|e| AppError::AiError(format!("model has no chat template: {e}")))?;
        let chat: Vec<LlamaChatMessage> = messages.iter().map(ai_to_llama_msg).collect();
        model.apply_chat_template(&tmpl, &chat, add_generation_prompt)
            .map_err(|e| AppError::AiError(format!("apply_chat_template: {e}")))
    }).await.map_err(|e| AppError::AiError(format!("template render task panicked: {e}")))?
}
```

(b) **Tools no longer passed to the template.** Instead they ride inside the
system message via the existing `{{tools_section}}` template variable in
`chat.system` (already populated by `registry.render_system_prompt_section`).
The model already sees the tool *information* there. The Jinja `tools=`
rendering that the OAI path added on top was just structural/syntactic — we
replace it with explicit format instructions in the system prompt (item 4).

(c) **A new `parse_qwen_tool_calls(text) -> Vec<AiToolCall>` in a new
`src-tauri/src/ai/llama_cpp/tool_parser.rs`**:

```rust
// Matches Qwen 3's emitted format:
//   <tool_call>{"name":"search_emails","arguments":{"limit":25}}</tool_call>
// JSON inside an XML envelope. Walk for each <tool_call>...</tool_call>
// block; serde_json::from_str the inner content; produce AiToolCall.
pub(super) fn parse_qwen_tool_calls(text: &str) -> Vec<AiToolCall>
```

For Gemma's `<|tool_call>call:name{args}<tool_call|>` format we fall back to
the existing `parse_xml_tool_calls` + `parse_python_call_tool_calls` salvage
parsers in `turn.rs` (they are NOT a perfect fit but cover the common cases
the salvage path already handled). If Gemma users hit a parse miss the chat
still completes — it just lands as a plain-text reply.

(d) **Token-count probes (`stable_prompt_bytes`, `system_prefix_bytes`)** now
call the new `render_template(messages, add_ass)` with the same probe
construction logic they already use. Same byte-level LCP analysis as today.

(e) **`chat_with_tools` / `chat_stream_with_tools` rewrite the parse path**:
take the model's final accumulated text, try `parse_qwen_tool_calls` first,
fall back to `parse_xml_tool_calls` then `parse_python_call_tool_calls`. The
same fallback chain that today runs as "salvage after oaicompat failed" now
becomes the primary path. Returns `AiMessage` with `tool_calls: Some(...)`
when any of them produced a non-empty list.

### 3. `src-tauri/src/ai/llama_cpp/tool_parser.rs` (NEW, ~80 LOC)

Single function `parse_qwen_tool_calls(text: &str) -> Vec<AiToolCall>`, plus a
table-driven unit test covering:
- Empty input → empty Vec
- Single `<tool_call>` with valid JSON args → 1 AiToolCall
- Multiple `<tool_call>` blocks → N AiToolCalls
- Malformed JSON inside the block → skip that block (don't crash), continue
- Tool call mixed with surrounding prose → tool call extracted, prose ignored
- Nested braces in arguments (e.g. `{"filter":{"k":"v"}}`) → parsed correctly

The parser walks the text linearly looking for `<tool_call>` … `</tool_call>`
substrings and `serde_json::from_str` on the inside.

### 4. `src-tauri/src/services/prompts/` — system prompt update (~15 LOC)

The `chat.system` template (or its tools_section render) gains explicit
format instructions so Qwen knows the exact syntax to emit even without the
Jinja `tools=` block. Append to the existing TOOL-CALLING DISCIPLINE section:

```
- Emit each tool call EXACTLY as:
    <tool_call>{"name":"<tool>","arguments":{<json args>}}</tool_call>
  One JSON per <tool_call> block. Multiple blocks are allowed. Do not wrap
  in code fences, do not add prose inside the block.
```

This is the single largest behavioural risk in the migration — the model now
sees example syntax but not the structured Jinja `tools=` rendering. Hand-eye
test: the first smoke chat should produce a parseable tool call. If not, we
escalate.

### 5. `src-tauri/src/ai/llama_cpp/runtime.rs` — `parse_oai_assistant_message` removal

The function exists today to decode the JSON `parse_response_oaicompat`
returned. No JSON now → delete the function. Replace its call sites with
direct construction of `AiMessage` from the model's text + parsed tool calls.

### 6. Thinking-mode behaviour change (acknowledged, no code)

The plain `apply_chat_template` doesn't accept `enable_thinking: false`. Qwen
3 will now emit `<think>...</think>` blocks with real content (instead of
empty). Generation latency increases by ~10-30%. The existing
`ThinkingGate` + `strip_reasoning` already strip these from user-visible
streams and stored messages. So:
- User experience: same (no visible `<think>` leakage)
- Latency: slightly worse — accept this for now
- Cache behaviour: slightly *better* — the volatile gen header tail shrinks
  from `<|im_start|>assistant\n<think>\n\n</think>\n\n` (7 tokens) to just
  `<|im_start|>assistant\n` (3 tokens). Cleaner stable boundary.

No code change needed; document in the runtime.rs comment block.

## What we DON'T touch

- Actor (`actor.rs`) — KV cache logic is unaffected
- Planner (`planner.rs`) — same
- All `services/chat/*` except the prompt template edit — same
- All tests except a new unit test for the Qwen parser — same
- Frontend — completely unchanged

## Migration steps in order (no eval gates per request)

1. **Bump Cargo.toml + cargo update** — confirms 0.1.147 resolves cleanly.
2. **Add `tool_parser.rs` with the Qwen parser + its unit test** — sanity
   check the parser standalone before threading it into runtime.
3. **Rewrite `runtime.rs::render_template`** with the plain
   `apply_chat_template` signature.
4. **Update `stable_prompt_bytes` + `system_prefix_bytes`** to use the new
   `render_template`. The byte-level diff logic stays identical; only the
   underlying template render call changes.
5. **Rewrite `chat_with_tools` + `chat_stream_with_tools`** to:
   - Drop `tools_json_owned` / `tools_json` plumbing (the parameter still
     exists in the function signature for API compatibility — we just don't
     forward it to the template).
   - After generation, run the new parser chain
     (`parse_qwen_tool_calls` → existing salvage parsers).
6. **Update `chat_stream`** (no tools) — simpler: just `render_template` +
   `actor.generate`.
7. **Delete `parse_oai_assistant_message`** + the `GEMMA4_CHAT_TEMPLATE`
   const + the `chat_template_fallback()` method (it's only used to feed
   the deleted const).
8. **System prompt template edit** (Step 4 above).
9. **Run `cargo check --no-default-features` + `cargo clippy -- -D warnings`
   + `cargo fmt --check` + `cargo test --no-default-features --lib`**
   — must all pass.
10. **Single smoke test** via `make cli-kv-personal`. Per the user's request,
    we **skip the chat eval suites** (chat_eval, chat_shortcut_eval,
    email_classification_eval). The smoke run just confirms the chat
    completes, produces a sensible-looking answer, and that the trace shows
    proper `Extend`/`RestartFromAnchor` behaviour from our recent KV work.

## Risks & how we ride them

| Risk | Likelihood | Mitigation if it bites |
|---|---|---|
| Qwen stops emitting `<tool_call>` syntax without Jinja `tools=` injection | medium | the new system-prompt format instructions (item 4) should cover it; if the smoke test shows the model going to plain text, expand the prompt with an explicit example |
| `parse_qwen_tool_calls` misses some tool-call shape | low | comprehensive unit tests + fallback to existing salvage parsers (`parse_xml_tool_calls`, `parse_python_call_tool_calls`) |
| Gemma 4 stops working entirely (no embedded template available) | low if user is on bartowski's builds, high on mradermacher's | document in commit; user can fall back to Qwen for now; re-add fallback template in a follow-up |
| Model quality regresses (no eval) | medium — this is the price of the "no eval" requirement | document the migration date + commit so we can revert if user reports a regression in real use; ideally run the chat eval at our convenience after merge |
| `ggml 0.10.0` introduces subtle Metal kernel behaviour changes | low | smoke test catches gross failures; subtle drift only an eval would catch |
| Thinking-mode latency hit | high (probable) | accept it; users will see slightly slower responses but the chat still works |

## Acceptance criteria for the smoke test (Step 10)

- Run completes with exit code 0
- Trace shows at least one `tool_round` with `tool_calls_requested > 0`
  (proves tool-call parsing works)
- Final stream produces a non-empty markdown table or prose answer
  (proves end-to-end generation works)
- No `parse_qwen_tool_calls` panic / fallback alert in stderr

If all four hold, we ship. If any fail, we triage that specific issue
before shipping.

## Estimated effort

- Code changes: **2–3 hours** of focused work
- Smoke testing: **15 minutes** (one CLI run)
- Total: **half a day**, conservatively.

Skipping the eval suite per the user's instruction is the time-saver. We
absorb ~half a day to a day of "is the model output quality the same?"
uncertainty in return.
