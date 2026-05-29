use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::services::chat::format_date;
use crate::services::contacts;

pub struct SearchContactsTool;

#[async_trait]
impl Tool for SearchContactsTool {
    fn name(&self) -> &'static str {
        "search_contacts"
    }

    fn description(&self) -> &'static str {
        "Resolve a person hint (informal name, partial email, company + first name) to actual contacts in the user's mailbox. Returns each match's full email address, display name, total email count and last-seen date. Call this BEFORE search_emails whenever the user names someone informally — then feed the resulting email into search_emails as `from`."
    }

    fn prompt_summary(&self) -> &'static str {
        "resolve a person hint (\"alice from emailops\") to email addresses. Call this BEFORE search_emails when the user names someone informally, then feed the email into search_emails(from=...)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Name or email hint. Examples: 'alice emailops', 'maria dolores', 'smith@'. Spaces are treated as AND (all tokens must match name or email)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max contacts to return. Default 10, max 25."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").trim();
        if query.is_empty() {
            return Ok(ToolOutput::text("Error: search_contacts requires a non-empty query."));
        }
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10).clamp(1, 25) as i32;
        Ok(ToolOutput::text(
            match contacts::search_contacts(ctx.db, ctx.account_id, query, limit) {
                Ok(contacts) if contacts.is_empty() => format!("No contacts matched \"{}\".", query),
                Ok(contacts) => {
                    let mut out = String::new();
                    for c in &contacts {
                        let last = c.last_timestamp.map(format_date).unwrap_or_else(|| "-".into());
                        out.push_str(&format!(
                            "- name=\"{}\" email={} count={} last_seen={}\n",
                            c.name, c.email, c.email_count, last
                        ));
                    }
                    out
                }
                Err(e) => format!("Search error: {}", e),
            },
        ))
    }
}
