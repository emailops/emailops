use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::chat::parse_iso_date_to_ts;

pub struct CreateTaskTool;

#[async_trait]
impl Tool for CreateTaskTool {
    fn name(&self) -> &'static str {
        "create_task"
    }

    fn description(&self) -> &'static str {
        "Create a new pending task. Only call this on explicit user request ('add a task to…', 'recuérdame que…', 'crea un pendiente…'). Returns the task id."
    }

    fn prompt_summary(&self) -> &'static str {
        "create a new pending task on explicit user request (\"add a task…\"/\"recuérdame que…\")."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Short imperative. Example: 'Send Q1 invoice to Acme'." },
                "due_at": { "type": "string", "description": "Optional ISO date or datetime." },
                "priority": { "type": "string", "description": "low | normal | high (default normal)." },
                "source_email_id": { "type": "string", "description": "Optional email id this task ties back to." }
            },
            "required": ["title"]
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        db.is_tasks_enabled().unwrap_or(false)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
        if title.is_empty() {
            return Ok(ToolOutput::text("Error: create_task requires a non-empty title."));
        }
        let due_at = args
            .get("due_at")
            .and_then(|v| v.as_str())
            .and_then(parse_iso_date_to_ts);
        let priority = args
            .get("priority")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
            .filter(|s| matches!(s.as_str(), "low" | "normal" | "high"));
        let source_email_id = args
            .get("source_email_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let req = crate::models::CreateTaskRequest {
            account_id: ctx.account_id.to_string(),
            title: title.to_string(),
            detail: None,
            priority,
            due_at,
            source_email_id,
            source_thread_id: None,
            source: Some("chat".to_string()),
            company: None,
        };
        Ok(ToolOutput::text(
            match crate::services::tasks::create_task(ctx.db, req) {
                Ok(task) => format!("Created task {} (\"{}\").", task.id, task.title),
                Err(e) => format!("Create task failed: {}", e),
            },
        ))
    }
}
