use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::chat::format_date;
use crate::services::emails;

pub struct ListDraftsTool;

#[async_trait]
impl Tool for ListDraftsTool {
    fn name(&self) -> &'static str {
        "list_drafts"
    }

    fn description(&self) -> &'static str {
        "List the user's saved email drafts for the active account, newest first. Returns each draft's id, subject, recipients, and last-edit time so the user (and the model) can pick which one to open."
    }

    fn prompt_summary(&self) -> &'static str {
        "list saved email drafts for the active account."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "Max drafts (default 10, max 25)." }
            },
            "required": []
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        db.is_ai_drafts_enabled().unwrap_or(true)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10).clamp(1, 25) as usize;
        match emails::list_drafts(ctx.db, ctx.account_id) {
            Ok(drafts) if drafts.is_empty() => Ok(ToolOutput::text("No drafts saved.")),
            Ok(drafts) => {
                let mut out = String::new();
                let mut refs: Vec<String> = Vec::with_capacity(drafts.len().min(limit));
                for d in drafts.iter().take(limit) {
                    let to = if d.to_addresses.is_empty() {
                        "-".to_string()
                    } else {
                        d.to_addresses.join(",")
                    };
                    out.push_str(&format!(
                        "- id={} subject=\"{}\" to={} updated={}\n",
                        d.id,
                        d.subject,
                        to,
                        format_date(d.updated_at),
                    ));
                    refs.push(d.id.clone());
                }
                Ok(ToolOutput::text_with_draft_refs(out, refs))
            }
            Err(e) => Ok(ToolOutput::text(format!("Draft list error: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::Database;
    use crate::models::SaveDraftRequest;

    #[tokio::test]
    async fn empty_when_no_drafts() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        db.seed_test_account("acc");
        let ctx = ToolCtx {
            db: &db,
            account_id: "acc",
            categories: &[],
        };
        let out = ListDraftsTool.execute(&ctx, serde_json::json!({})).await.unwrap();
        assert_eq!(out.text, "No drafts saved.");
    }

    #[tokio::test]
    async fn lists_saved_drafts_newest_first() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        db.seed_test_account("acc");
        db.save_draft(&SaveDraftRequest {
            id: None,
            email_id: None,
            account_id: "acc".to_string(),
            to_addresses: vec!["a@x.com".to_string()],
            subject: "First".to_string(),
            body: "body".to_string(),
        })
        .unwrap();
        let ctx = ToolCtx {
            db: &db,
            account_id: "acc",
            categories: &[],
        };
        let out = ListDraftsTool.execute(&ctx, serde_json::json!({})).await.unwrap();
        assert!(out.text.contains("First"), "got: {}", out.text);
        assert!(out.text.contains("a@x.com"), "got: {}", out.text);
    }

    #[tokio::test]
    async fn respects_limit_argument() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        db.seed_test_account("acc");
        for i in 0..5 {
            db.save_draft(&SaveDraftRequest {
                id: None,
                email_id: None,
                account_id: "acc".to_string(),
                to_addresses: vec![format!("a{i}@x.com")],
                subject: format!("Draft {i}"),
                body: "body".to_string(),
            })
            .unwrap();
        }
        let ctx = ToolCtx {
            db: &db,
            account_id: "acc",
            categories: &[],
        };
        let out = ListDraftsTool
            .execute(&ctx, serde_json::json!({ "limit": 2 }))
            .await
            .unwrap();
        // Counting newlines is a robust enough proxy: one row per line.
        let lines = out.text.lines().filter(|l| l.starts_with("- id=")).count();
        assert_eq!(lines, 2, "expected 2 rows; got:\n{}", out.text);
    }
}
