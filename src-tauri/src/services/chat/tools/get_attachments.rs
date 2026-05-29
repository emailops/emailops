use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::services::attachments;

pub struct GetAttachmentsTool;

#[async_trait]
impl Tool for GetAttachmentsTool {
    fn name(&self) -> &'static str {
        "get_attachments"
    }

    fn description(&self) -> &'static str {
        "List attachments on a specific email (by email_id). MUST BE CALLED after `search_emails` whenever the user's question mentions invoices / facturas / recibos / PDFs / files / documents / 'adjuntos' — the search result's snippet never contains the real filenames, so naming the attachment in your final answer requires this call. The result includes filename, size, and an attachment:// link that you MUST render verbatim in your answer as a Markdown link so the user can click to download."
    }

    fn prompt_summary(&self) -> &'static str {
        "list attachments. Render each link verbatim as a Markdown link to `attachment://meta/<id>`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "email_id": {
                    "type": "string",
                    "description": "The email ID whose attachments should be listed"
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

        // The service helper returns both the canonical meta list and the
        // rule-matched table — the chat tool needs both so the user can see
        // every attachment, with the right namespace prefix for the open URL.
        let (metas, rule_matched) = attachments::list_for_email(ctx.db, email_id).unwrap_or_default();

        if metas.is_empty() && rule_matched.is_empty() {
            return Ok(ToolOutput::text("No attachments on this email."));
        }

        // Two ID namespaces tell the UI which open command to call:
        // `meta/<id>` for email_attachment_meta (canonical) and
        // `attach/<id>` for the rule-matched `attachments` table.
        let mut out = String::new();
        for m in &metas {
            out.push_str(&format!(
                "- filename=\"{}\" mime={} size={} link=[{}](attachment://meta/{})\n",
                m.filename, m.mime_type, m.file_size, m.filename, m.id,
            ));
        }
        for a in &rule_matched {
            if metas.iter().any(|m| m.filename == a.filename) {
                continue;
            }
            out.push_str(&format!(
                "- filename=\"{}\" mime={} size={} tags={} link=[{}](attachment://attach/{})\n",
                a.filename,
                a.mime_type,
                a.file_size,
                a.tags.join(","),
                a.filename,
                a.id,
            ));
        }
        // The LLM is told to mention the parent email when explaining the
        // attachments — whitelist it so the corresponding `email://EMAIL_ID`
        // link survives validation. The other attachment IDs use the
        // `attachment://` scheme so they go through a different validator.
        Ok(ToolOutput::text_with_email_refs(out, vec![email_id.to_string()]))
    }
}
