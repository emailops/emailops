use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::chat::format_date;
use crate::services::lenses;

pub struct GetLensDataTool;

#[async_trait]
impl Tool for GetLensDataTool {
    fn name(&self) -> &'static str {
        "get_lens_data"
    }

    fn description(&self) -> &'static str {
        "Read the already-extracted rows of a Lens. Pass either `lens_id` (exact) or `lens_name` (case-insensitive — looked up via list_lenses). Returns each row's extracted JSON fields plus the source email's subject and sender. Does NOT trigger a new extraction run; if the lens has zero rows, returns a hint that the user can run the lens from the UI to populate it."
    }

    fn prompt_summary(&self) -> &'static str {
        "read already-extracted rows of a Lens (by lens_id or lens_name). Does NOT trigger extraction."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "lens_id": { "type": "string", "description": "Exact lens id (from list_lenses)." },
                "lens_name": { "type": "string", "description": "Case-insensitive lens name. Used when lens_id is omitted." },
                "limit": { "type": "integer", "description": "Max rows (default 20, max 50)." },
                "offset": { "type": "integer", "description": "Offset into the result set (default 0)." }
            },
            "required": []
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        db.is_lenses_enabled().unwrap_or(false)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        // ── Resolve lens id ──
        let lens_id_arg = args
            .get("lens_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let lens_name_arg = args
            .get("lens_name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());

        let (lens_id, lens_name) = match (lens_id_arg, lens_name_arg) {
            (Some(id), _) => (id.to_string(), None),
            (None, Some(name)) => match resolve_lens_by_name(ctx, name) {
                Ok(Some((id, real_name))) => (id, Some(real_name)),
                Ok(None) => {
                    return Ok(ToolOutput::text(format!(
                        "No lens matched \"{name}\". Call list_lenses to see what's available."
                    )));
                }
                Err(e) => return Ok(ToolOutput::text(format!("Lens lookup error: {e}"))),
            },
            (None, None) => {
                return Ok(ToolOutput::text("Error: provide either `lens_id` or `lens_name`."));
            }
        };

        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20).clamp(1, 50);
        let offset = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0).max(0);

        let page = match lenses::get_lens_rows(ctx.db, &lens_id, None, limit, offset) {
            Ok(p) => p,
            Err(e) => return Ok(ToolOutput::text(format!("Lens data error: {e}"))),
        };

        if page.rows.is_empty() {
            // Don't trigger extraction — the user only asked to *see* the data.
            // Provide a hint so the chat can suggest the next action.
            let label = lens_name.unwrap_or_else(|| lens_id.clone());
            return Ok(ToolOutput::text(format!(
                "Lens \"{label}\" has no extracted rows yet. Run the lens from the sidebar (Lenses → {label}) to populate it."
            )));
        }

        let mut out = String::new();
        for r in &page.rows {
            let data = serde_json::to_string(&r.data).unwrap_or_else(|_| "{}".into());
            out.push_str(&format!(
                "- email_id={} subject=\"{}\" sender=\"{}\" date={} data={}\n",
                r.email_id,
                r.email_subject,
                r.email_sender,
                format_date(r.email_timestamp),
                data,
            ));
        }
        Ok(ToolOutput::text(out))
    }
}

/// Case-insensitive name resolution against `list_lenses`. Returns the
/// (id, real_name) pair so callers can show the canonical capitalisation
/// back in error messages. Returns Ok(None) for no match.
fn resolve_lens_by_name(
    ctx: &ToolCtx<'_>,
    name: &str,
) -> Result<Option<(String, String)>, crate::models::error::AppError> {
    let needle = name.to_ascii_lowercase();
    for l in lenses::list_lenses(ctx.db)? {
        if l.name.to_ascii_lowercase() == needle {
            return Ok(Some((l.id, l.name)));
        }
    }
    // Fall back to substring match in case the user gave a partial name.
    for l in lenses::list_lenses(ctx.db)? {
        if l.name.to_ascii_lowercase().contains(&needle) {
            return Ok(Some((l.id, l.name)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::Database;

    fn enabled_db() -> Arc<Database> {
        let db = Database::new_for_testing().expect("db");
        db.set_preference("lenses_enabled", "true").unwrap();
        Arc::new(db)
    }

    #[tokio::test]
    async fn missing_both_args_returns_validation_error() {
        let db = enabled_db();
        let ctx = ToolCtx {
            db: &db,
            account_id: "acc",
            categories: &[],
            app: None,
        };
        let out = GetLensDataTool.execute(&ctx, serde_json::json!({})).await.unwrap();
        assert!(out.text.starts_with("Error:"));
    }

    #[tokio::test]
    async fn unknown_lens_name_returns_helpful_hint() {
        let db = enabled_db();
        let ctx = ToolCtx {
            db: &db,
            account_id: "acc",
            categories: &[],
            app: None,
        };
        let out = GetLensDataTool
            .execute(&ctx, serde_json::json!({ "lens_name": "Receipts" }))
            .await
            .unwrap();
        assert!(out.text.contains("No lens matched"), "got: {}", out.text);
    }

    #[test]
    fn gated_off_when_lenses_disabled() {
        let db = Database::new_for_testing().expect("db");
        assert!(!GetLensDataTool.is_available(&db));
    }
}
