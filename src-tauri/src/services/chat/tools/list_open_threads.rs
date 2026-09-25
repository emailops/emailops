use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::services::chat::format_date;
use crate::services::memory;

pub struct ListOpenThreadsTool;

#[async_trait]
impl Tool for ListOpenThreadsTool {
    fn name(&self) -> &'static str {
        "list_open_threads"
    }

    fn description(&self) -> &'static str {
        "Return threads with open conversational state — who owes the next reply, summary, deadline. Useful for 'what am I waiting on', 'what did I leave hanging'. This is reply state, not read state: for mail the user has not read, use search_emails with unread=true."
    }

    fn prompt_summary(&self) -> &'static str {
        "list threads awaiting a reply (who owes it, deadlines) — not unread mail."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "awaiting": { "type": "string", "description": "Filter by who owes the next reply: user, them, resolved. Default: any non-resolved." },
                "limit": { "type": "integer", "description": "Max threads (default 10)." }
            },
            "required": []
        })
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let awaiting = args.get("awaiting").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10).clamp(1, 50) as i32;
        Ok(ToolOutput::text(
            match memory::list_open_threads(ctx.db, ctx.account_id, awaiting, limit) {
                Ok(threads) if threads.is_empty() => "No open threads.".to_string(),
                Ok(threads) => {
                    // Who wrote last and which email to link: without them the
                    // model named the user as the sender and invented ids.
                    let thread_ids: Vec<String> = threads.iter().map(|t| t.thread_id.clone()).collect();
                    let inbound = match ctx.db.latest_inbound_by_thread(ctx.account_id, &thread_ids) {
                        Ok(inbound) => inbound,
                        Err(e) => return Ok(ToolOutput::text(format!("Thread list error: {}", e))),
                    };
                    let mut out = String::new();
                    for t in &threads {
                        let summary = t.summary.clone().unwrap_or_default();
                        let deadline = t.deadline_at.map(format_date).unwrap_or_else(|| "-".into());
                        let last_inbound = inbound
                            .get(&t.thread_id)
                            .map(|e| {
                                let from = if e.sender.trim().is_empty() {
                                    e.sender_email.clone()
                                } else {
                                    format!("{} <{}>", e.sender, e.sender_email)
                                };
                                format!(
                                    " from=\"{}\" last_inbound={} email_id={}",
                                    from,
                                    format_date(e.timestamp),
                                    e.email_id
                                )
                            })
                            .unwrap_or_default();
                        out.push_str(&format!(
                            "- thread_id={} awaiting={}{} deadline={} summary=\"{}\"\n",
                            t.thread_id, t.awaiting, last_inbound, deadline, summary
                        ));
                    }
                    out
                }
                Err(e) => format!("Thread list error: {}", e),
            },
        ))
    }
}
