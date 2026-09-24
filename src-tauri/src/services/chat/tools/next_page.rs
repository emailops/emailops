//! Continue the conversation's last `search_emails` result.
//!
//! The chat history the model replays carries only user and assistant text —
//! never the arguments of a past tool call. So "show me the next ones" cannot
//! be answered by asking the model to repeat the previous filters: it would
//! have to guess them, and a guessed page 2 does not continue page 1. This
//! tool takes no arguments; the page it continues comes from
//! [`ToolCtx::page`](super::ToolCtx::page), which the chat turn seeds from the
//! previous assistant message's persisted trace.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::search_emails::SearchEmailsTool;
use super::{SearchPage, Tool, ToolCtx, ToolError, ToolOutput};
use crate::models::ChatMessage;

pub struct NextPageTool;

#[async_trait]
impl Tool for NextPageTool {
    fn name(&self) -> &'static str {
        "next_page"
    }

    fn description(&self) -> &'static str {
        "Return the next page of the last search_emails result in this conversation — same filters, the matches that came after the ones already shown. Takes no arguments. Call it when the user asks for more of a list you already showed ('the next ones', 'show me more', 'y los demás'). Only useful after a result that said another page exists; without one it says so."
    }

    fn prompt_summary(&self) -> &'static str {
        "continue the last search_emails result with the next page of matches; no arguments."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, _args: Value) -> Result<ToolOutput, ToolError> {
        let Some(page) = ctx.page.and_then(|p| p.pending()) else {
            return Ok(ToolOutput::text(
                "No further results to show — run search_emails first, or its last page was already the final one.",
            ));
        };
        let mut args = page.args;
        if let Some(obj) = args.as_object_mut() {
            obj.insert("offset".to_string(), json!(page.next_offset));
        }
        SearchEmailsTool.execute(ctx, args).await
    }
}

/// The page a new turn can continue, recovered from the conversation history.
///
/// Pure: walks the persisted traces newest-first and returns the first page
/// that still has matches left. The filters come from the last
/// `search_emails` call of that turn, the position from the pagination note
/// the tool wrote into the result — the same line the model saw.
pub fn pending_page_from_history(history: &[ChatMessage]) -> Option<SearchPage> {
    history.iter().rev().find_map(|message| {
        let trace = message.trace.as_ref()?;
        let mut args: Option<Value> = None;
        let mut position: Option<(i32, i32)> = None;
        for call in &trace.tool_calls {
            if call.name == SearchEmailsTool.name() {
                args = Some(call.arguments.clone());
            }
            if let Some(found) = parse_page_note(&call.result_preview) {
                position = Some(found);
            }
        }
        let (next_offset, total) = position?;
        if next_offset >= total {
            return None;
        }
        Some(SearchPage {
            args: args?,
            next_offset,
            total,
        })
    })
}

/// Read "(showing 26-50 of 156 matching threads …)" back into
/// `(next_offset, total)`. A capped probe ("500+") reports the floor, which
/// is enough to know another page exists.
fn parse_page_note(result: &str) -> Option<(i32, i32)> {
    let rest = result.strip_prefix("(showing ")?;
    let (range, rest) = rest.split_once(" of ")?;
    let (_, last) = range.split_once('-')?;
    let (total, _) = rest.split_once(" matching threads")?;
    let last: i32 = last.trim().parse().ok()?;
    let total: i32 = total.trim().trim_end_matches('+').parse().ok()?;
    Some((last, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChatTrace, RouteDecision, RouteMode, ToolCallTrace};

    fn tool_call(name: &str, arguments: Value, result_preview: &str) -> ToolCallTrace {
        ToolCallTrace {
            name: name.to_string(),
            round: 0,
            arguments,
            result_preview: result_preview.to_string(),
            result_chars: result_preview.len() as i32,
            elapsed_ms: 1,
        }
    }

    fn assistant_with(calls: Vec<ToolCallTrace>) -> ChatMessage {
        let mut message = ChatMessage {
            id: "m".to_string(),
            conversation_id: "c".to_string(),
            role: "assistant".to_string(),
            content: "answer".to_string(),
            model: None,
            token_count: None,
            latency_ms: None,
            created_at: 0,
            sources: Vec::new(),
            trace: None,
            referenced_email_ids: Vec::new(),
            referenced_draft_ids: Vec::new(),
            prompt_content: None,
        };
        message.trace = Some(ChatTrace {
            route: RouteDecision {
                mode: RouteMode::ToolsFirst,
                reason: String::new(),
                matched_keywords: Vec::new(),
                classifier: String::new(),
            },
            retrieval: None,
            tool_calls: calls,
            model: "m".to_string(),
            total_elapsed_ms: 1,
            tool_loop_ms: 1,
            llm_streaming_ms: None,
            llm_calls: Vec::new(),
            help: None,
            research: None,
            steps: Vec::new(),
        });
        message
    }

    #[test]
    fn continues_the_search_that_still_has_matches_left() {
        let history = vec![assistant_with(vec![tool_call(
            "search_emails",
            json!({"from": "news@example.com", "limit": 25}),
            "(showing 1-25 of 54 matching threads — call next_page for the next ones)\n- id=a",
        )])];

        let page = pending_page_from_history(&history).expect("a page to continue");

        assert_eq!(page.next_offset, 25);
        assert_eq!(page.total, 54);
        assert_eq!(page.args["from"], json!("news@example.com"));
    }

    #[test]
    fn a_later_page_advances_from_the_note_not_from_the_original_args() {
        // The second page came from next_page, whose own args carry no filters.
        let history = vec![assistant_with(vec![
            tool_call(
                "search_emails",
                json!({"from": "news@example.com", "limit": 25}),
                "(showing 1-25 of 54 matching threads — call next_page for the next ones)\n- id=a",
            ),
            tool_call(
                "next_page",
                json!({}),
                "(showing 26-50 of 54 matching threads — call next_page for the next ones)\n- id=z",
            ),
        ])];

        let page = pending_page_from_history(&history).expect("a page to continue");

        assert_eq!(page.next_offset, 50);
        assert_eq!(page.args["from"], json!("news@example.com"));
    }

    #[test]
    fn the_last_page_leaves_nothing_to_continue() {
        let history = vec![assistant_with(vec![tool_call(
            "search_emails",
            json!({"from": "news@example.com", "limit": 25}),
            "(showing 51-54 of 54 matching threads — this is the last page)\n- id=a",
        )])];

        assert!(pending_page_from_history(&history).is_none());
    }

    #[test]
    fn a_search_that_fit_on_one_page_leaves_nothing_to_continue() {
        let history = vec![assistant_with(vec![tool_call(
            "search_emails",
            json!({"from": "news@example.com"}),
            "## Primary (3)\n- id=a",
        )])];

        assert!(pending_page_from_history(&history).is_none());
    }

    #[test]
    fn the_newest_search_wins_over_an_older_paged_one() {
        let history = vec![
            assistant_with(vec![tool_call(
                "search_emails",
                json!({"from": "old@example.com"}),
                "(showing 1-25 of 54 matching threads — call next_page for the next ones)\n- id=a",
            )]),
            assistant_with(vec![tool_call(
                "search_emails",
                json!({"from": "new@example.com"}),
                "(showing 1-25 of 99 matching threads — call next_page for the next ones)\n- id=b",
            )]),
        ];

        let page = pending_page_from_history(&history).expect("a page to continue");

        assert_eq!(page.args["from"], json!("new@example.com"));
        assert_eq!(page.total, 99);
    }

    #[test]
    fn no_trace_no_page() {
        assert!(pending_page_from_history(&[]).is_none());
    }
}
