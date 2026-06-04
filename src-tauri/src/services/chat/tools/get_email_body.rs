use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::services::emails;
use crate::services::thread_clean;

pub struct GetEmailBodyTool;

#[async_trait]
impl Tool for GetEmailBodyTool {
    fn name(&self) -> &'static str {
        "get_email_body"
    }

    fn description(&self) -> &'static str {
        "Fetch the full body text of a specific email by its ID. Use this when the snippet from search_emails is not enough to answer the question."
    }

    fn prompt_summary(&self) -> &'static str {
        "fetch one email's body."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "email_id": {
                    "type": "string",
                    "description": "The email ID to fetch"
                }
            },
            "required": ["email_id"]
        })
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let email_id = args.get("email_id").and_then(|v| v.as_str()).unwrap_or("");
        if email_id.is_empty() {
            return Ok(ToolOutput::text("Error: missing email_id"));
        }
        match emails::get_email_body(ctx.db, email_id) {
            Ok(body) if body.is_empty() => Ok(ToolOutput::text("Email body is empty or not yet downloaded.")),
            Ok(body) => {
                // Reuse the same cleaning pipeline as "chat about this email"
                // (strip HTML, quoted replies, signatures, invisible chars) at
                // the single-email ceiling, so the model gets the body nearly
                // whole instead of a 3000-char slice cut mid-sentence.
                let text = thread_clean::clean_email_body(&body, thread_clean::MAX_CHARS_PER_EMAIL);
                // Whitelist the email the LLM just read so any
                // `email://EMAIL_ID` link it emits ("here's the relevant
                // excerpt from <email://X>...") passes validation.
                Ok(ToolOutput::text_with_email_refs(text, vec![email_id.to_string()]))
            }
            Err(e) => Ok(ToolOutput::text(format!("Error: {}", e))),
        }
    }
}
