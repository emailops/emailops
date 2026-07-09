// Parser for Qwen 3's native tool-call output format.
//
// Qwen 3 (and most modern instruction-tuned models that follow the Qwen
// schema) emits tool calls as XML-bracketed JSON blocks inline with the
// model's prose:
//
//     I'll search the inbox.
//     <tool_call>{"name":"search_emails","arguments":{"limit":25}}</tool_call>
//
// Up until llama-cpp-2 0.1.146 we relied on `parse_response_oaicompat` (a
// thin Rust wrapper over llama.cpp's `common/`-library OAI helpers) to find
// these blocks and decode them into structured `tool_calls`. 0.1.147 removed
// the OAI compat surface upstream, so we own this parsing now.
//
// Scope:
//   - Handles `<tool_call>{json}</tool_call>` (Qwen native, our primary
//     model family).
//   - Other model families (Gemma's `<|tool_call>call:NAME{args}<tool_call|>`,
//     Llama 3's `<|python_tag|>{json}`) are not in scope here. The
//     fallback chain in `runtime.rs` runs the existing salvage parsers
//     (`parse_xml_tool_calls`, `parse_python_call_tool_calls`) afterwards
//     for those.
//
// Robustness (all observed Qwen 3.6 35B emissions):
//   - Malformed JSON inside a block → skip that block, keep scanning.
//   - Prose surrounding the blocks → ignored.
//   - Nested braces inside `arguments` → parsed correctly via serde_json.
//   - Trailing garbage after the object (`…,"limit":25}}`) → first complete
//     JSON value wins.
//   - Flattened args (`{"name":"x","from":"y"}`, no `arguments` wrapper) →
//     top-level keys beside `name` become the arguments.
//   - Missing `</tool_call>` (newline-batched calls) → the next open tag or
//     end of text is an implicit close; incomplete JSON still yields nothing.

use crate::ai::provider::{AiToolCall, AiToolCallFunction};

const OPEN_TAG: &str = "<tool_call>";
const CLOSE_TAG: &str = "</tool_call>";

/// Extract every `<tool_call>{json}</tool_call>` block from `text`, decoded
/// into structured `AiToolCall` values. Empty `Vec` when nothing parses —
/// callers should fall back to other parsers (or treat the whole text as
/// the model's plain answer).
pub(super) fn parse_qwen_tool_calls(text: &str) -> Vec<AiToolCall> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_open) = text[cursor..].find(OPEN_TAG) {
        let after_open = cursor + rel_open + OPEN_TAG.len();
        // The model sometimes batches calls as newline-separated
        // `<tool_call>{json}` lines with NO closing tags. The next open tag —
        // or the end of the text — acts as an implicit close; the lenient
        // JSON parse below rejects incomplete bodies, so a genuinely
        // truncated stream still yields nothing rather than a bogus call.
        let rest = &text[after_open..];
        let (close, next_cursor) = match (rest.find(CLOSE_TAG), rest.find(OPEN_TAG)) {
            (Some(c), Some(o)) if c < o => (after_open + c, after_open + c + CLOSE_TAG.len()),
            (Some(c), None) => (after_open + c, after_open + c + CLOSE_TAG.len()),
            (_, Some(o)) => (after_open + o, after_open + o),
            (None, None) => (text.len(), text.len()),
        };
        let inner = text[after_open..close].trim();
        cursor = next_cursor;

        // Lenient first-value parse: take the first complete JSON value and
        // ignore trailing garbage — Qwen 3.6 occasionally emits an extra
        // closing brace after the object (`…,"limit":25}}`), which a strict
        // `from_str` would reject, dropping an otherwise-good call. A block
        // with no leading parseable value is malformed — skip and keep
        // scanning.
        let Some(value) = serde_json::Deserializer::from_str(inner)
            .into_iter::<serde_json::Value>()
            .next()
            .and_then(|r| r.ok())
        else {
            continue;
        };
        let Some(obj) = value.as_object() else { continue };
        // `arguments` is conventionally an object, but some emitters use the
        // empty object `{}`, omit it entirely, or FLATTEN the args to the top
        // level next to `name` (`{"name":"search_emails","from":"x"}`). When
        // the wrapper is absent, treat every top-level key except `name` as
        // the arguments — an empty remainder degrades to the no-args object.
        let mut arguments = obj.get("arguments").cloned().unwrap_or_else(|| {
            let flat: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .filter(|(k, _)| k.as_str() != "name")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            serde_json::Value::Object(flat)
        });
        // Small models (Qwen 3.5 4B was the observed offender) sometimes
        // flatten the call into `{"arguments":{...,"name":"<tool>"}}` with no
        // top-level `name`. Hoist the nested `name` so the block dispatches
        // instead of being silently dropped (which surfaced to the user as an
        // empty reply). When BOTH a top-level and a nested `name` exist, trust
        // the top-level one — the canonical shape — and leave the nested
        // value untouched in case a future tool legitimately takes a `name`
        // parameter.
        let name = match obj.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => {
                let Some(args_obj) = arguments.as_object_mut() else {
                    continue;
                };
                let Some(serde_json::Value::String(n)) = args_obj.remove("name") else {
                    continue;
                };
                n
            }
        };

        out.push(AiToolCall {
            function: AiToolCallFunction { name, arguments },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_input_returns_empty() {
        assert!(parse_qwen_tool_calls("").is_empty());
        assert!(parse_qwen_tool_calls("just prose, no tool calls").is_empty());
    }

    #[test]
    fn single_well_formed_call() {
        let text = r#"<tool_call>{"name":"search_emails","arguments":{"limit":25,"since":"2026-06-15"}}</tool_call>"#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(calls[0].function.arguments, json!({"limit":25,"since":"2026-06-15"}));
    }

    #[test]
    fn multiple_calls_in_one_turn() {
        // Qwen sometimes emits a sequence of independent calls when asked
        // to fetch multiple things at once. Each block parses to its own
        // entry; order is preserved.
        let text = r#"
            <tool_call>{"name":"a","arguments":{"x":1}}</tool_call>
            and then
            <tool_call>{"name":"b","arguments":{"y":2}}</tool_call>
        "#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "a");
        assert_eq!(calls[1].function.name, "b");
        assert_eq!(calls[0].function.arguments, json!({"x":1}));
        assert_eq!(calls[1].function.arguments, json!({"y":2}));
    }

    #[test]
    fn ignores_surrounding_prose() {
        let text = r#"I'll search the inbox first.
        <tool_call>{"name":"search_emails","arguments":{}}</tool_call>
        Then I'll summarise the results."#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(calls[0].function.arguments, json!({}));
    }

    #[test]
    fn nested_objects_in_arguments() {
        // Lens-extraction prompts can carry deeply nested filter objects.
        // serde_json handles the brace counting natively; our linear scanner
        // only looks for the literal close tag, so the JSON's internal braces
        // never confuse it.
        let text = r#"<tool_call>{"name":"filter","arguments":{"filter":{"k":"v","nested":{"a":1}}}}</tool_call>"#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].function.arguments,
            json!({"filter":{"k":"v","nested":{"a":1}}})
        );
    }

    #[test]
    fn malformed_json_skips_the_block_but_keeps_scanning() {
        // First block is malformed (trailing comma, no quotes on key).
        // Second is well-formed. We want the second one through, not
        // both lost, and not a panic.
        let text = r#"
            <tool_call>{name: search_emails,}</tool_call>
            <tool_call>{"name":"recover","arguments":{"k":1}}</tool_call>
        "#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "recover");
    }

    #[test]
    fn unterminated_open_tag_does_not_hang_or_misparse() {
        // Model could emit a partial tool call mid-stream (then crash, or
        // hit max_tokens). We should bail cleanly, not treat the rest of
        // the prompt as a giant tool-call body.
        let text = r#"<tool_call>{"name":"truncated_at"#;
        assert!(parse_qwen_tool_calls(text).is_empty());
    }

    #[test]
    fn missing_arguments_field_defaults_to_empty_object() {
        let text = r#"<tool_call>{"name":"no_args"}</tool_call>"#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "no_args");
        assert_eq!(calls[0].function.arguments, json!({}));
    }

    #[test]
    fn missing_name_field_skips_the_block() {
        // Without a name there is no tool to dispatch — silently drop.
        let text = r#"<tool_call>{"arguments":{"x":1}}</tool_call>"#;
        assert!(parse_qwen_tool_calls(text).is_empty());
    }

    #[test]
    fn nested_name_inside_arguments_is_hoisted() {
        // Small models (Qwen 3.5 4B was the observed offender) sometimes
        // flatten the call into `{"arguments":{...,"name":"<tool>"}}` —
        // `name` ends up as a sibling of the real args instead of a
        // top-level key. Without leniency the block is silently dropped
        // and the user sees an empty reply. We salvage by hoisting `name`
        // out of `arguments` and stripping it from the args we dispatch
        // (the tool's schema doesn't take a `name` parameter).
        let text = r#"<tool_call>{"arguments":{"email_id":"abc123","instructions":"Brief polite reply.","name":"generate_email_draft"}}</tool_call>"#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "generate_email_draft");
        assert_eq!(
            calls[0].function.arguments,
            json!({"email_id":"abc123","instructions":"Brief polite reply."})
        );
    }

    #[test]
    fn nested_name_with_no_other_args_dispatches_with_empty_args() {
        // Same leniency, edge case: `arguments` carried ONLY `name`. After
        // hoisting we should dispatch with an empty args object, not with
        // an args bag that still contains a `name` key.
        let text = r#"<tool_call>{"arguments":{"name":"list_drafts"}}</tool_call>"#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "list_drafts");
        assert_eq!(calls[0].function.arguments, json!({}));
    }

    #[test]
    fn top_level_name_wins_over_nested_name() {
        // If the model emits BOTH a top-level `name` and a nested one,
        // trust the top level (the canonical shape) and leave the nested
        // value untouched as a real argument. This guards against the
        // pathological case where a tool legitimately takes a `name`
        // parameter (none today, but future tools might).
        let text = r#"<tool_call>{"name":"outer","arguments":{"name":"inner","x":1}}</tool_call>"#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "outer");
        assert_eq!(calls[0].function.arguments, json!({"name":"inner","x":1}));
    }

    #[test]
    fn non_object_json_skips_the_block() {
        // A plain array or string inside the block can't represent a call.
        let text = r#"<tool_call>["not","an","object"]</tool_call>"#;
        assert!(parse_qwen_tool_calls(text).is_empty());
    }

    #[test]
    fn flattened_args_with_trailing_brace_are_salvaged() {
        // Verbatim production emission (Qwen 3.6 35B): the `arguments` wrapper
        // is dropped (args flattened next to `name`) AND an extra closing
        // brace trails the object, so a strict serde parse rejects the whole
        // block and no tool ever runs. Both defects must be tolerated.
        let text = "<tool_call>{\"name\":\"search_emails\",\"from\":\"sharique\",\"limit\":25}}\n</tool_call>";
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(calls[0].function.arguments, json!({"from":"sharique","limit":25}));
    }

    #[test]
    fn flattened_args_without_wrapper_become_arguments() {
        // Same flattened shape, valid JSON: top-level keys besides `name`
        // are the arguments.
        let text = r#"<tool_call>{"name":"get_email_body","email_id":"e42"}</tool_call>"#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_email_body");
        assert_eq!(calls[0].function.arguments, json!({"email_id":"e42"}));
    }

    #[test]
    fn explicit_arguments_wrapper_still_wins_over_flat_siblings() {
        // When BOTH shapes appear, the canonical wrapper is authoritative —
        // stray top-level keys are ignored.
        let text = r#"<tool_call>{"name":"a","arguments":{"x":1},"stray":2}</tool_call>"#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.arguments, json!({"x":1}));
    }

    #[test]
    fn newline_batched_calls_without_close_tags_are_all_parsed() {
        // Verbatim production emission (Qwen 3.6 35B): a batch of calls as
        // newline-separated `<tool_call>{json}` lines with NO closing tags.
        // Requiring `</tool_call>` dropped the whole batch — no tool ever ran.
        // The next open tag (or end of text) is an implicit close.
        let text = "<tool_call>{\"arguments\":{\"thread_id\":\"t1\"},\"name\":\"get_thread\"}\n\
<tool_call>{\"arguments\":{\"thread_id\":\"t2\"},\"name\":\"get_thread\"}\n\
<tool_call>{\"arguments\":{\"thread_id\":\"t3\"},\"name\":\"get_thread\"}\n";
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].function.arguments, json!({"thread_id":"t1"}));
        assert_eq!(calls[2].function.arguments, json!({"thread_id":"t3"}));
    }

    #[test]
    fn single_trailing_close_after_unterminated_batch_splits_correctly() {
        // Same batch shape but the model remembered exactly ONE closing tag at
        // the very end — each open must still bind to its own JSON line, not
        // swallow the whole batch as one block.
        let text = "<tool_call>{\"name\":\"a\",\"arguments\":{\"x\":1}}\n\
<tool_call>{\"name\":\"b\",\"arguments\":{\"y\":2}}</tool_call>";
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "a");
        assert_eq!(calls[1].function.name, "b");
    }

    #[test]
    fn unterminated_block_with_complete_json_at_eof_is_parsed() {
        // The stream ended right after a complete JSON object (no close tag).
        // The call is complete — execute it.
        let text = r#"<tool_call>{"name":"list_drafts","arguments":{}}"#;
        let calls = parse_qwen_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "list_drafts");
    }

    #[test]
    fn trailing_garbage_only_blocks_still_skip() {
        // Leniency must not turn junk into a call.
        assert!(parse_qwen_tool_calls("<tool_call>}}</tool_call>").is_empty());
        assert!(parse_qwen_tool_calls("<tool_call>not json}</tool_call>").is_empty());
    }
}
