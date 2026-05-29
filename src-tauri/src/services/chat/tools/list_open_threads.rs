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
        "Return threads with open state — who's waiting on whom, summary, deadline. Useful for 'what am I waiting on', 'what did I leave hanging'."
    }

    fn prompt_summary(&self) -> &'static str {
        "list threads with open state (who's waiting on whom, deadlines)."
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
                    let mut out = String::new();
                    for t in &threads {
                        let summary = t.summary.clone().unwrap_or_default();
                        let deadline = t.deadline_at.map(format_date).unwrap_or_else(|| "-".into());
                        out.push_str(&format!(
                            "- thread_id={} awaiting={} deadline={} summary=\"{}\"\n",
                            t.thread_id, t.awaiting, deadline, summary
                        ));
                    }
                    out
                }
                Err(e) => format!("Thread list error: {}", e),
            },
        ))
    }
}
