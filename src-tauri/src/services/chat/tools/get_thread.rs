use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};

pub struct GetThreadTool;

#[async_trait]
impl Tool for GetThreadTool {
    fn name(&self) -> &'static str {
        "get_thread"
    }

    fn description(&self) -> &'static str {
        "Fetch all emails in a conversation thread by thread ID. Use this to see the full back-and-forth of a discussion."
    }

    fn prompt_summary(&self) -> &'static str {
        "fetch a full conversation."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": {
                    "type": "string",
                    "description": "The thread ID to fetch"
                }
            },
            "required": ["thread_id"]
        })
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let thread_id = args.get("thread_id").and_then(|v| v.as_str()).unwrap_or("");
        if thread_id.is_empty() {
            return Ok(ToolOutput::text("Error: missing thread_id"));
        }
        // The shared thread reader: each reply's new content only, one
        // budget for the whole thread, every message tagged with its id so
        // the model can link it (`email://ID`).
        use crate::services::thread_reader::{
            load_thread, read_thread, render_thread, ReadOptions, CHAT_THREAD_BUDGET,
        };
        match load_thread(ctx.db, ctx.account_id, thread_id) {
            Ok(messages) if messages.is_empty() => Ok(ToolOutput::text("No emails found in this thread.")),
            Ok(messages) => {
                let read = read_thread(&messages, &ReadOptions::budget(CHAT_THREAD_BUDGET));
                let refs = read.messages.iter().map(|m| m.id.clone()).collect();
                Ok(ToolOutput::text_with_email_refs(render_thread(&read), refs))
            }
            Err(e) => Ok(ToolOutput::text(format!("Error: {}", e))),
        }
    }
}
