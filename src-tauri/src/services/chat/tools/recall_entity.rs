use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::memory;

pub struct RecallEntityTool;

#[async_trait]
impl Tool for RecallEntityTool {
    fn name(&self) -> &'static str {
        "recall_entity"
    }

    fn description(&self) -> &'static str {
        "Return everything memory knows about a specific contact/domain/project, identified by email, domain, or slug. Combines facts with the latest thread_state. Preferred when the user names a specific person or organisation."
    }

    fn prompt_summary(&self) -> &'static str {
        "everything memory knows about a contact/domain/project (by email, domain, or slug)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "email, domain, or project slug, e.g. 'alice@acme.com', 'acme.com', 'project-atlas'." }
            },
            "required": ["key"]
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        db.is_memory_enabled().unwrap_or(false)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if key.is_empty() {
            return Ok(ToolOutput::text("Error: recall_entity requires a non-empty key."));
        }
        // Service combines contact / domain / project lookups in priority order.
        let combined = memory::recall_entity(ctx.db, ctx.account_id, &key).unwrap_or_default();
        if combined.is_empty() {
            return Ok(ToolOutput::text(format!("No memory entries for \"{}\".", key)));
        }
        let mut out = String::new();
        for f in &combined {
            out.push_str(&format!(
                "- [{}] {} (status={}, confidence={:.2})\n",
                f.subject_kind, f.fact, f.status, f.confidence
            ));
        }
        Ok(ToolOutput::text(out))
    }
}
