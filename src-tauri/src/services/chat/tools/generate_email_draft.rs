use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolEffect, ToolError, ToolOutput};
use crate::db::Database;
use crate::models::SaveDraftRequest;
use crate::services::emails;

pub struct GenerateEmailDraftTool;

#[async_trait]
impl Tool for GenerateEmailDraftTool {
    fn name(&self) -> &'static str {
        "generate_email_draft"
    }

    fn description(&self) -> &'static str {
        "Generate and save an email draft. You MUST provide ONE of these two arg sets — never neither:\n\
         (A) REPLY mode → `email_id` (id of the inbound email you are replying to). The most common case: when the user said \"draft a reply to X\" or \"draft for X's last email\", first call search_emails to find that email's id, then pass it here.\n\
         (B) NEW mode → BOTH `to` (array of recipients) AND `subject`. Use only when the user is starting a brand-new conversation.\n\
         `instructions` is OPTIONAL extra guidance (e.g. \"mention the March invoice\", \"keep it short\") — passing only `instructions` is INVALID and will be rejected. The draft is saved locally and the composer opens automatically."
    }

    fn prompt_summary(&self) -> &'static str {
        "draft a reply OR new email and save it; composer opens automatically. REQUIRED ARGS: pass `email_id` for a reply, OR pass both `to` and `subject` for a new email. Passing only `instructions` will fail — always include one of those two arg sets. Use this whenever the user says draft / write / reply / compose."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "email_id": {
                    "type": "string",
                    "description": "ID of the email being replied to. REQUIRED when drafting a reply (this is the usual case). Get it from search_emails."
                },
                "to": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Recipient email addresses. REQUIRED (together with `subject`) only when composing a brand-new email — NOT used for replies."
                },
                "subject": {
                    "type": "string",
                    "description": "Subject line. REQUIRED (together with `to`) only when composing a brand-new email — NOT used for replies (the reply subject is derived from the inbound email)."
                },
                "instructions": {
                    "type": "string",
                    "description": "Optional extra guidance (tone, key points, length). NEVER sufficient on its own — always include `email_id` OR (`to` + `subject`)."
                }
            },
            "required": []
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        db.is_ai_drafts_enabled().unwrap_or(true)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        let instructions = args.get("instructions").and_then(|v| v.as_str());
        let email_id = args
            .get("email_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());

        // ── REPLY MODE ─────────────────────────────────────────────────────
        if let Some(eid) = email_id {
            return Ok(generate_reply_draft(ctx, eid, instructions).await);
        }

        // ── NEW MODE ───────────────────────────────────────────────────────
        let to: Vec<String> = match args.get("to") {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            // Tolerate a single-string `to` since some models forget the array.
            Some(Value::String(s)) => s
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            _ => Vec::new(),
        };
        let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("").trim();

        if to.is_empty() || subject.is_empty() {
            return Ok(ToolOutput::text(
                "Error: provide either `email_id` (reply) OR (`to` + `subject`) for a new email.",
            ));
        }

        Ok(generate_new_draft(ctx, &to, subject, instructions).await)
    }
}

/// Reply path: load the inbound email, generate the body via the existing
/// service, persist a draft row pointing at that email, and emit
/// `OpenComposer` so the frontend pops the composer prefilled.
async fn generate_reply_draft(ctx: &ToolCtx<'_>, email_id: &str, instructions: Option<&str>) -> ToolOutput {
    let inbound = match ctx.db.get_email_by_id(email_id) {
        Ok(Some(e)) => e,
        Ok(None) => return ToolOutput::text(format!("Error: email {email_id} not found.")),
        Err(e) => return ToolOutput::text(format!("Error loading email: {e}")),
    };
    let result = match emails::generate_draft(ctx.db, email_id, instructions).await {
        Ok(r) => r,
        Err(e) => return ToolOutput::text(format!("Draft generation failed: {e}")),
    };

    let subject = if inbound.subject.to_lowercase().starts_with("re:") {
        inbound.subject.clone()
    } else {
        format!("Re: {}", inbound.subject)
    };

    let req = SaveDraftRequest {
        id: None,
        email_id: Some(email_id.to_string()),
        account_id: inbound.account_id.clone(),
        to_addresses: vec![inbound.sender_email.clone()],
        subject,
        body: result.body.clone(),
    };
    match emails::save_draft(ctx.db, &req) {
        Ok(draft) => {
            // Capture the id before `draft` is partially moved into the
            // OpenComposer effect — we need it in two places: the effect
            // (so the UI can open the composer) and the draft_refs slot
            // (so the frontend can validate the `draft://DRAFT_ID` link the
            // LLM will likely emit when describing what it just saved).
            let draft_id = draft.id.clone();
            // Mention the saved draft id inline in the LLM-visible text so
            // small models can grab it and wrap their natural-language
            // confirmation as `[label](draft://DRAFT_ID)`. Without this the
            // model only sees the subject and the chars count.
            let text = format!(
                "Reply draft saved: id={} subject=\"{}\" ({} chars). Composer is opening.",
                draft_id,
                draft.subject,
                draft.body.len()
            );
            ToolOutput {
                text,
                effects: vec![ToolEffect::OpenComposer {
                    draft_id: draft.id,
                    account_id: draft.account_id,
                    // Reply mode: carry the inbound email id so the frontend
                    // can open the reply inline inside the matching thread,
                    // same as clicking Reply on the thread itself.
                    email_id: Some(email_id.to_string()),
                    to_addresses: draft.to_addresses,
                    subject: draft.subject,
                    body: draft.body,
                }],
                // The reply path showed the inbound email to the LLM (via
                // get_email_by_id) before drafting — expose its id so a later
                // `email://EMAIL_ID` link the LLM may emit ("reply to <link>…")
                // passes the allowlist check.
                email_refs: vec![email_id.to_string()],
                // Allowlist the saved draft so the LLM's `draft://` chip
                // (re-open-the-draft) renders without being dropped by the
                // validator.
                draft_refs: vec![draft_id],
            }
        }
        Err(e) => ToolOutput::text(format!(
            "Draft was generated ({} chars) but saving failed: {}. \
             Body:\n\n{}",
            result.body.len(),
            e,
            result.body,
        )),
    }
}

/// New-email path: skip the inbound lookup and the thread context; the
/// service builds a "compose new" prompt. Save and emit OpenComposer.
async fn generate_new_draft(ctx: &ToolCtx<'_>, to: &[String], subject: &str, instructions: Option<&str>) -> ToolOutput {
    let result = match emails::generate_new_draft(ctx.db, ctx.account_id, to, subject, instructions).await {
        Ok(r) => r,
        Err(e) => return ToolOutput::text(format!("Draft generation failed: {e}")),
    };
    let req = SaveDraftRequest {
        id: None,
        email_id: None,
        account_id: ctx.account_id.to_string(),
        to_addresses: to.to_vec(),
        subject: subject.to_string(),
        body: result.body.clone(),
    };
    match emails::save_draft(ctx.db, &req) {
        Ok(draft) => {
            let draft_id = draft.id.clone();
            let text = format!(
                "New draft saved: id={} subject=\"{}\" → {} ({} chars). Composer is opening.",
                draft_id,
                draft.subject,
                draft.to_addresses.join(", "),
                draft.body.len(),
            );
            ToolOutput {
                text,
                effects: vec![ToolEffect::OpenComposer {
                    draft_id: draft.id,
                    account_id: draft.account_id,
                    // New-mail mode: no inbound thread, so the frontend opens
                    // a standalone compose tab.
                    email_id: None,
                    to_addresses: draft.to_addresses,
                    subject: draft.subject,
                    body: draft.body,
                }],
                // New-mail mode: no inbound was shown to the LLM, so there is
                // no email to whitelist for `email://EMAIL_ID` links.
                email_refs: Vec::new(),
                // The freshly-saved draft is the one the LLM should chip up.
                draft_refs: vec![draft_id],
            }
        }
        Err(e) => ToolOutput::text(format!(
            "Draft was generated ({} chars) but saving failed: {}. \
             Body:\n\n{}",
            result.body.len(),
            e,
            result.body,
        )),
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::Database;

    fn ctx_with_db<'a>(db: &'a Arc<Database>, account_id: &'static str) -> ToolCtx<'a> {
        ToolCtx {
            db,
            account_id,
            categories: &[],
        }
    }

    #[tokio::test]
    async fn missing_all_args_returns_validation_error() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let tool = GenerateEmailDraftTool;
        let out = tool
            .execute(&ctx_with_db(&db, "acc"), serde_json::json!({}))
            .await
            .unwrap();
        assert!(out.text.starts_with("Error:"));
        assert!(out.effects.is_empty(), "no effects on validation error");
    }

    #[tokio::test]
    async fn new_mode_missing_subject_returns_validation_error() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let tool = GenerateEmailDraftTool;
        let args = serde_json::json!({ "to": ["a@x.com"] });
        let out = tool.execute(&ctx_with_db(&db, "acc"), args).await.unwrap();
        assert!(out.text.starts_with("Error:"));
        assert!(out.effects.is_empty());
    }

    #[tokio::test]
    async fn reply_mode_unknown_email_id_returns_error() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        let tool = GenerateEmailDraftTool;
        let args = serde_json::json!({ "email_id": "does-not-exist" });
        let out = tool.execute(&ctx_with_db(&db, "acc"), args).await.unwrap();
        assert!(out.text.contains("not found"), "got: {}", out.text);
        assert!(out.effects.is_empty());
    }

    #[test]
    fn tool_is_gated_off_when_drafts_disabled() {
        let db = Database::new_for_testing().expect("db");
        db.set_preference("ai_drafts_enabled", "false").unwrap();
        let tool = GenerateEmailDraftTool;
        assert!(!tool.is_available(&db));
    }

    #[test]
    fn tool_is_available_when_drafts_enabled() {
        let db = Database::new_for_testing().expect("db");
        // Default is true even if pref is absent.
        let tool = GenerateEmailDraftTool;
        assert!(tool.is_available(&db));
        db.set_preference("ai_drafts_enabled", "true").unwrap();
        assert!(tool.is_available(&db));
    }
}
