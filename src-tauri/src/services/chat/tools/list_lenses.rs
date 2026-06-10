use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::lenses;

pub struct ListLensesTool;

#[async_trait]
impl Tool for ListLensesTool {
    fn name(&self) -> &'static str {
        "list_lenses"
    }

    fn description(&self) -> &'static str {
        "List the user's saved Lenses (custom AI-extracted views of the mailbox like 'Invoices', 'Job applicants'). Each row returns id, name, icon, the account it's scoped to, whether it's enabled, and the count of already-extracted rows. Call this BEFORE `get_lens_data` whenever the user names a lens informally so you can resolve the id."
    }

    fn prompt_summary(&self) -> &'static str {
        "list user-defined Lenses (custom AI views of the mailbox). Call BEFORE get_lens_data to resolve a lens name to its id."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        db.is_lenses_enabled().unwrap_or(false)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, _args: Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(match lenses::list_lenses(ctx.db) {
            Ok(items) if items.is_empty() => "No lenses defined.".to_string(),
            Ok(items) => {
                let mut out = String::new();
                for l in &items {
                    out.push_str(&format!(
                        "- id={} name=\"{}\" icon={} enabled={} rows={}\n",
                        l.id,
                        l.name,
                        l.icon.as_deref().unwrap_or("-"),
                        l.is_enabled,
                        l.row_count,
                    ));
                }
                out
            }
            Err(e) => format!("Lens list error: {}", e),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::Database;

    #[test]
    fn gated_off_when_lenses_disabled() {
        let db = Database::new_for_testing().expect("db");
        assert!(!ListLensesTool.is_available(&db));
    }

    #[test]
    fn available_when_lenses_enabled() {
        let db = Database::new_for_testing().expect("db");
        db.set_preference("lenses_enabled", "true").unwrap();
        assert!(ListLensesTool.is_available(&db));
    }

    #[tokio::test]
    async fn empty_when_no_lenses() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        db.set_preference("lenses_enabled", "true").unwrap();
        let ctx = ToolCtx {
            db: &db,
            account_id: "acc",
            categories: &[],
        };
        let out = ListLensesTool.execute(&ctx, serde_json::json!({})).await.unwrap();
        assert_eq!(out.text, "No lenses defined.");
    }
}
