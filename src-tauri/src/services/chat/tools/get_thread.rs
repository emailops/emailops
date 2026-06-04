use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::services::chat::format_date;
use crate::services::emails;
use crate::services::thread_clean;

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
        match emails::get_thread(ctx.db, ctx.account_id, thread_id) {
            Ok(thread) if thread.is_empty() => Ok(ToolOutput::text("No emails found in this thread.")),
            Ok(thread) => {
                let mut out = String::new();
                let mut refs: Vec<String> = Vec::with_capacity(thread.len());
                // Share one context budget across the whole thread: short
                // threads keep each message nearly whole, long threads divide
                // the budget but never drop below the per-email floor. Same
                // cleaning pipeline as "chat about this thread".
                let cap = thread_clean::chars_per_email(thread.len());
                for email in &thread {
                    let body = emails::get_email_body(ctx.db, &email.id).unwrap_or_default();
                    let body_text = thread_clean::clean_email_body(&body, cap);
                    // Tag each block with the email id so the LLM can address
                    // individual messages in its prose ("the kickoff was
                    // <email://abc-123>last Tuesday</email://abc-123>…").
                    out.push_str(&format!(
                        "--- id={} | {} | from: {} <{}> | date: {} ---\n{}\n\n",
                        email.id,
                        email.subject,
                        email.sender,
                        email.sender_email,
                        format_date(email.timestamp),
                        body_text,
                    ));
                    refs.push(email.id.clone());
                }
                Ok(ToolOutput::text_with_email_refs(out, refs))
            }
            Err(e) => Ok(ToolOutput::text(format!("Error: {}", e))),
        }
    }
}
