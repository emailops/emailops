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
// Robustness:
//   - Malformed JSON inside a block → skip that block, keep scanning.
//   - Prose surrounding the blocks → ignored.
//   - Nested braces inside `arguments` → parsed correctly via serde_json.
//   - Mismatched tags (open without close) → ignored.

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
        let Some(rel_close) = text[after_open..].find(CLOSE_TAG) else {
            // Unterminated open tag — best to bail rather than treat the
            // rest of the prompt as one giant tool-call body.
            break;
        };
        let close = after_open + rel_close;
        let inner = text[after_open..close].trim();
        cursor = close + CLOSE_TAG.len();

        // serde_json is the right shape detector here: it ignores leading/
        // trailing whitespace and handles nested objects natively. A failed
        // parse means a malformed block — silently skip and keep scanning.
        let Ok(value) = serde_json::from_str::<serde_json::Value>(inner) else {
            continue;
        };
        let Some(obj) = value.as_object() else { continue };
        // `arguments` is conventionally an object, but some emitters use the
        // empty object `{}` or omit it entirely. Treat both as no-args.
        let mut arguments = obj
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
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
}
