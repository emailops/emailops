//! Continue the conversation's last `search_emails` result.
//!
//! The chat history the model replays carries only user and assistant text —
//! never the arguments of a past tool call. So "show me the next ones" cannot
//! be answered by asking the model to repeat the previous filters: it would
//! have to guess them, and a guessed page 2 does not continue page 1. This
//! tool takes no arguments; the page it continues comes from
//! [`ToolCtx::page`](super::ToolCtx::page), which the chat turn seeds from the
//! page the previous turn recorded on its trace (`ChatTrace::search_page`).

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
/// Pure: the newest turn that recorded the conversation's search page decides
/// (see `ChatTrace::search_page`) — its page if matches are left, otherwise
/// nothing. A turn that records no page (a form fill) is skipped.
pub fn pending_page_from_history(history: &[ChatMessage]) -> Option<SearchPage> {
    let page = history
        .iter()
        .rev()
        .find_map(|message| message.trace.as_ref()?.search_page.clone())?;
    (page.next_offset < page.total).then_some(SearchPage {
        args: page.args,
        next_offset: page.next_offset,
        total: page.total,
    })
}

/// A turn's page, as its trace records it.
pub fn page_trace(page: &super::PageState) -> Option<crate::models::SearchPageTrace> {
    page.snapshot().map(|p| crate::models::SearchPageTrace {
        args: p.args,
        next_offset: p.next_offset,
        total: p.total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChatTrace, RouteDecision, RouteMode, SearchPageTrace, ToolCallTrace};

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
            applied_skills: Vec::new(),
            steps: Vec::new(),
            search_page: None,
            budget: None,
        });
        message
    }

    fn with_page(mut message: ChatMessage, args: Value, next_offset: i32, total: i32) -> ChatMessage {
        if let Some(trace) = message.trace.as_mut() {
            trace.search_page = Some(SearchPageTrace {
                args,
                next_offset,
                total,
            });
        }
        message
    }

    #[test]
    fn continues_the_page_the_last_turn_persisted() {
        let history = vec![with_page(
            assistant_with(vec![]),
            json!({"from": "news@example.com", "limit": 25}),
            25,
            54,
        )];

        let page = pending_page_from_history(&history).expect("a page to continue");

        assert_eq!(page.next_offset, 25);
        assert_eq!(page.total, 54);
        assert_eq!(page.args["from"], json!("news@example.com"));
    }

    #[test]
    fn a_result_whose_note_is_not_first_still_continues() {
        // Other notes (tag coverage, semantic fallback) are prepended to the
        // page note, so reading the prose missed it; the state is explicit now.
        let history = vec![with_page(
            assistant_with(vec![tool_call(
                "search_emails",
                json!({"from": "news@example.com", "intent": "request"}),
                "(3 emails carry the tags asked for; …)\n(showing 1-25 of 54 matching threads — …)\n- id=a",
            )]),
            json!({"from": "news@example.com", "intent": "request"}),
            25,
            54,
        )];

        assert_eq!(pending_page_from_history(&history).map(|p| p.next_offset), Some(25));
    }

    #[test]
    fn an_exhausted_newest_page_does_not_fall_back_to_an_older_one() {
        let history = vec![
            with_page(assistant_with(vec![]), json!({"from": "old@example.com"}), 25, 54),
            with_page(assistant_with(vec![]), json!({"from": "new@example.com"}), 4, 4),
        ];

        assert!(pending_page_from_history(&history).is_none());
    }

    #[test]
    fn a_turn_that_carries_no_page_state_is_skipped() {
        // A form-fill turn records no search state: the page before it stands.
        let history = vec![
            with_page(assistant_with(vec![]), json!({"from": "a@example.com"}), 25, 54),
            assistant_with(vec![]),
        ];

        assert_eq!(
            pending_page_from_history(&history).map(|p| p.args["from"].clone()),
            Some(json!("a@example.com"))
        );
    }

    #[test]
    fn no_trace_no_page() {
        assert!(pending_page_from_history(&[]).is_none());
    }
}
