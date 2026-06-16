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
        "Disambiguate a person hint to an email address. Use only when the name alone is ambiguous (single first name, partial email fragment, role like \"CEO of acme\"). For full names, search_emails(from=\"Full Name\") matches display-name substrings directly — no pre-lookup needed."
    }

    fn prompt_summary(&self) -> &'static str {
        "disambiguate an unclear name (\"alice\", \"smith@\") to an email address. Skip for full names — search_emails(from=…) matches display names."
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
