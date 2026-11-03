use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::services::emails;
use crate::services::thread_clean;

const NO_BODY: &str = "Email body is empty or not yet downloaded.";

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
        let email = match ctx.readable_email(email_id) {
            Ok(Some(email)) => email,
            Ok(None) => return Ok(ToolOutput::text(NO_BODY)),
            Err(e) => return Ok(ToolOutput::text(format!("Error: {}", e))),
        };
        match emails::get_email_body(ctx.db, email_id) {
            Ok(body) if body.is_empty() => Ok(ToolOutput::text(NO_BODY)),
            Ok(body) => {
                // Read it in its thread, like "chat about this email": what the
                // thread already has (quoted history, a repeated signature)
                // goes, a forward or a quote of unsynced mail stays. Capped at
                // the single-email ceiling rather than a slice cut mid-sentence.
                let new = crate::services::thread_reader::message_new_content(ctx.db, &email, &body);
                let text = thread_clean::clean_email_body(&new, thread_clean::MAX_CHARS_PER_EMAIL);
                // Whitelist the email the LLM just read so any
                // `email://EMAIL_ID` link it emits ("here's the relevant
                // excerpt from <email://X>...") passes validation.
                Ok(ToolOutput::text_with_email_refs(text, vec![email_id.to_string()]))
            }
            Err(e) => Ok(ToolOutput::text(format!("Error: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::Database;
    use crate::services::chat::tools::PageState;
    use crate::services::thread_reader::fixtures;

    #[tokio::test]
    async fn a_reply_is_read_without_the_quote_its_thread_already_has() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        fixtures::seed_quoting_thread(&db);
        let ctx = ToolCtx {
            db: &db,
            account_id: "acct",
            categories: &[],
            page: None,
        };
        let out = GetEmailBodyTool.execute(&ctx, json!({"email_id": "e2"})).await.unwrap();
        assert_eq!(out.text, fixtures::REPLY_NEW);
    }

    /// In unified mode the chat runs on one account while the open thread can
    /// belong to any of them: its emails stay readable, and nothing else of
    /// that account does.
    #[tokio::test]
    async fn the_thread_the_user_has_open_is_readable_from_another_account() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        fixtures::seed_quoting_thread(&db);
        let page = PageState::seeded(None).with_open_thread(vec!["e2".to_string()]);
        let ctx = ToolCtx {
            db: &db,
            account_id: "chat-account",
            categories: &[],
            page: Some(&page),
        };

        let open = GetEmailBodyTool.execute(&ctx, json!({"email_id": "e2"})).await.unwrap();
        assert_eq!(open.text, fixtures::REPLY_NEW);

        let elsewhere = GetEmailBodyTool.execute(&ctx, json!({"email_id": "e1"})).await.unwrap();
        assert_eq!(elsewhere.text, NO_BODY);
    }
}
