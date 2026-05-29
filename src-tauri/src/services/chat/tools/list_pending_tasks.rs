use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::chat::{format_date, parse_iso_date_to_ts};
use crate::services::tasks;

pub struct ListPendingTasksTool;

#[async_trait]
impl Tool for ListPendingTasksTool {
    fn name(&self) -> &'static str {
        "list_pending_tasks"
    }

    fn description(&self) -> &'static str {
        "Return the user's open tasks (extracted from emails or manually created). Use for 'what are my pending tasks', 'what did I promise this week', 'mis pendientes'."
    }

    fn prompt_summary(&self) -> &'static str {
        "list the user's open tasks (extracted from emails or manually created)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "description": "Filter by status: open (default), done, snoozed, dismissed." },
                "due_before": { "type": "string", "description": "ISO date — only return tasks with due_at on or before this date." },
                "limit": { "type": "integer", "description": "Max tasks (default 10, max 50)." }
            },
            "required": []
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        db.is_tasks_enabled().unwrap_or(false)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let status = args.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());
        let due_before = args
            .get("due_before")
            .and_then(|v| v.as_str())
            .and_then(parse_iso_date_to_ts);
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10).clamp(1, 50) as i32;
        Ok(ToolOutput::text(
            match tasks::list_pending(ctx.db, ctx.account_id, status.as_deref(), due_before, limit) {
                Ok(tasks) if tasks.is_empty() => "No matching tasks.".to_string(),
                Ok(tasks) => {
                    let mut out = String::new();
                    for t in &tasks {
                        let due = t.due_at.map(format_date).unwrap_or_else(|| "-".into());
                        out.push_str(&format!(
                            "- id={} title=\"{}\" priority={} due={} status={}\n",
                            t.id, t.title, t.priority, due, t.status
                        ));
                    }
                    out
                }
                Err(e) => format!("Task list error: {}", e),
            },
        ))
    }
}
