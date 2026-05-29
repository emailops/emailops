use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::memory;

pub struct MemorySearchTool;

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &'static str {
        "memory_search"
    }

    fn description(&self) -> &'static str {
        "Full-text search over the user's durable memory facts (profile, contacts, domains, projects). Use when the user asks about things THEY told you earlier, or about recurring patterns that aren't in a specific email. Returns fact text, subject, and status (candidate|promoted)."
    }

    fn prompt_summary(&self) -> &'static str {
        "full-text search the user's durable memory facts (profile, contacts, domains, projects)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keywords to search for." },
                "limit": { "type": "integer", "description": "Max facts (default 5, max 15)." }
            },
            "required": ["query"]
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        db.is_memory_enabled().unwrap_or(false)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").trim();
        if query.is_empty() {
            return Ok(ToolOutput::text("Error: memory_search requires a non-empty query."));
        }
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5).clamp(1, 15) as i32;
        // Service handles the FTS + best-effort recency bump.
        Ok(ToolOutput::text(
            match memory::search_facts(ctx.db, ctx.account_id, query, limit) {
                Ok(hits) if hits.is_empty() => format!("No memory facts matched \"{}\".", query),
                Ok(hits) => {
                    let mut out = String::new();
                    for (f, _) in &hits {
                        out.push_str(&format!(
                            "- [{}:{}] {} (status={}, confidence={:.2})\n",
                            f.subject_kind, f.subject_key, f.fact, f.status, f.confidence
                        ));
                    }
                    out
                }
                Err(e) => format!("Memory search error: {}", e),
            },
        ))
    }
}
