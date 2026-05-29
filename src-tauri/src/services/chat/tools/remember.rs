use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::memory;

pub struct RememberTool;

#[async_trait]
impl Tool for RememberTool {
    fn name(&self) -> &'static str {
        "remember"
    }

    fn description(&self) -> &'static str {
        "Persist a new memory fact when the user explicitly tells you something worth remembering long-term ('remember that I prefer morning meetings', 'recuerda que mi IVA es 21%'). Saved as 'candidate' — promotion is handled by the background consolidation job."
    }

    fn prompt_summary(&self) -> &'static str {
        "persist a new memory fact on explicit user request (\"remember that…\"/\"recuerda que…\")."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "fact": { "type": "string", "description": "One declarative sentence capturing the fact." },
                "subject_kind": { "type": "string", "description": "user | contact | domain | project" },
                "subject_key": { "type": "string", "description": "'self' for user facts, otherwise the email / domain / slug." }
            },
            "required": ["fact", "subject_kind", "subject_key"]
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        db.is_memory_enabled().unwrap_or(false)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let fact = args.get("fact").and_then(|v| v.as_str()).unwrap_or("").trim();
        let kind = args.get("subject_kind").and_then(|v| v.as_str()).unwrap_or("").trim();
        let key = args.get("subject_key").and_then(|v| v.as_str()).unwrap_or("").trim();
        if fact.is_empty() || kind.is_empty() || key.is_empty() {
            return Ok(ToolOutput::text(
                "Error: remember requires fact, subject_kind, subject_key.",
            ));
        }
        if !matches!(
            kind.to_ascii_lowercase().as_str(),
            "user" | "contact" | "domain" | "project"
        ) {
            return Ok(ToolOutput::text(format!(
                "Error: subject_kind must be user|contact|domain|project (got '{kind}')."
            )));
        }
        Ok(ToolOutput::text(
            match memory::remember_fact(ctx.db, ctx.account_id, fact, kind, key) {
                Ok(row) => format!("Saved memory fact {} (status=candidate).", row.id),
                Err(e) => format!("Remember failed: {}", e),
            },
        ))
    }
}
