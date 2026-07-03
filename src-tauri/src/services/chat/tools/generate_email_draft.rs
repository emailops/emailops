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
         `instructions` is OPTIONAL extra guidance (e.g. \"mention the March invoice\", \"keep it short\") — passing only `instructions` is INVALID and will be rejected. The draft is saved locally and the composer opens automatically.\n\
         STALE-THREAD GUARD: in REPLY mode, if the requested `email_id` is not the most recent message in its thread, the tool refuses to draft and returns the full thread so you can ask the user which message to reply to. Once the user confirms, re-call with the chosen `email_id` and `acknowledge_not_latest=true`."
    }

    fn prompt_summary(&self) -> &'static str {
        "draft a reply OR new email and save it; composer opens automatically. REQUIRED ARGS: pass `email_id` for a reply, OR pass both `to` and `subject` for a new email. Passing only `instructions` will fail — always include one of those two arg sets. Use this whenever the user says draft / write / reply / compose. If the email being replied to is not the latest in its thread, the tool refuses and returns the thread — ask the user which message to reply to, then re-call with `acknowledge_not_latest=true`."
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
                },
                "acknowledge_not_latest": {
                    "type": "boolean",
                    "description": "Set true to bypass the stale-thread guard when replying to a message that is not the latest in its thread. Use only AFTER the user has confirmed which message to reply to. Without this flag, the tool refuses to draft for non-latest messages and returns the thread so you can ask."
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
        let acknowledge_not_latest = args
            .get("acknowledge_not_latest")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // ── REPLY MODE ─────────────────────────────────────────────────────
        if let Some(eid) = email_id {
            return Ok(generate_reply_draft(ctx, eid, instructions, acknowledge_not_latest).await);
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

/// If the requested email isn't the latest message in its thread, return
/// a `ToolOutput` that:
///   - explains the situation in plain text so the LLM can relay it,
///   - lists every message in the thread (id, sender, ISO date, snippet)
///     so the LLM can render `email://` chips for each,
///   - allowlists every thread id in `email_refs` so those chips pass the
///     frontend's validator,
///   - emits no effects and allowlists no draft id.
///
/// Returns `None` when the requested email is the latest (or the only
/// message), so the caller proceeds to generate the draft normally.
///
/// DB lookup failures are treated as "not stale" — the worst-case
/// fallback (drafting anyway) matches the pre-guard behavior, and a
/// transient read error shouldn't block a draft.
fn check_thread_freshness(ctx: &ToolCtx<'_>, inbound: &crate::models::Email) -> Option<ToolOutput> {
    let thread = match ctx.db.get_thread(&inbound.account_id, &inbound.thread_id) {
        Ok(t) if !t.is_empty() => t,
        _ => return None,
    };
    let latest = thread.iter().max_by_key(|e| e.timestamp)?;
    if latest.id == inbound.id {
        return None;
    }

    // Render the thread as a numbered list the LLM can lift into chips.
    // ISO date keeps the format model-independent.
    let mut lines = String::new();
    for e in &thread {
        let date = chrono::DateTime::from_timestamp(e.timestamp, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| e.timestamp.to_string());
        let marker = if e.id == inbound.id {
            " (the one you asked about)"
        } else if e.id == latest.id {
            " (latest — most likely the one to reply to)"
        } else {
            ""
        };
        let snippet = e.snippet.chars().take(120).collect::<String>();
        lines.push_str(&format!(
            "- id={} from \"{}\" <{}> on {}{}: \"{}\"\n",
            e.id, e.sender, e.sender_email, date, marker, snippet
        ));
    }

    let text = format!(
        "Refusing to draft: the requested email (id={}) is NOT the latest message in its thread. \
         {} other message(s) followed it. Show the user the thread below — wrap each id as an \
         email://ID chip — and ask which message they want to reply to. After they confirm, \
         re-call generate_email_draft with the chosen email_id AND `acknowledge_not_latest=true` \
         to bypass this guard.\n\n\
         Thread (oldest → newest):\n{}",
        inbound.id,
        thread.len().saturating_sub(1),
        lines,
    );

    Some(ToolOutput {
        text,
        effects: Vec::new(),
        email_refs: thread.iter().map(|e| e.id.clone()).collect(),
        draft_refs: Vec::new(),
    })
}

/// Reply path: load the inbound email, generate the body via the existing
/// service, persist a draft row pointing at that email, and emit
/// `OpenComposer` so the frontend pops the composer prefilled.
async fn generate_reply_draft(
    ctx: &ToolCtx<'_>,
    email_id: &str,
    instructions: Option<&str>,
    acknowledge_not_latest: bool,
) -> ToolOutput {
    let inbound = match ctx.db.get_email_by_id(email_id) {
        Ok(Some(e)) => e,
        Ok(None) => return ToolOutput::text(format!("Error: email {email_id} not found.")),
        Err(e) => return ToolOutput::text(format!("Error loading email: {e}")),
    };

    // Stale-thread guard: if the requested email isn't the most recent
    // message in its thread, refuse to draft and hand the LLM the full
    // thread so it can ask the user which message to reply to. Replying
    // to a mid-thread message after others have continued the conversation
    // is almost always a mistake — the user usually means "the latest"
    // and just named the sender they remember. The acknowledge flag lets
    // them override after explicit confirmation.
    if !acknowledge_not_latest {
        if let Some(out) = check_thread_freshness(ctx, &inbound) {
            return out;
        }
    }

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
        cc_addresses: Vec::new(),
        subject,
        body: result.body.clone(),
        body_html: None,
        provider_draft_id: None,
        attachments: None,
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
        cc_addresses: Vec::new(),
        subject: subject.to_string(),
        body: result.body.clone(),
        body_html: None,
        provider_draft_id: None,
        attachments: None,
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

    use rusqlite::params;

    use super::*;
    use crate::db::Database;

    fn ctx_with_db<'a>(db: &'a Arc<Database>, account_id: &'static str) -> ToolCtx<'a> {
        ToolCtx {
            db,
            account_id,
            categories: &[],
        }
    }

    /// Seed an `accounts` row and an `emails` row so `get_email_by_id` /
    /// `get_thread` return them. Keeps the FK on `emails.account_id`
    /// satisfied. No body / FTS rows — the stale-thread guard only reads
    /// `emails`, never the body table.
    fn seed_email(
        db: &Database,
        id: &str,
        account: &str,
        thread_id: &str,
        sender: &str,
        sender_email: &str,
        subject: &str,
        timestamp: i64,
    ) {
        let sender_domain = sender_email
            .rsplit_once('@')
            .map(|(_, d)| d.to_lowercase())
            .unwrap_or_default();
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails
             (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
              recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'[]','[]','snip',?8,0,'primary',0)",
            params![
                id,
                account,
                thread_id,
                subject,
                sender,
                sender_email,
                sender_domain,
                timestamp,
            ],
        )
        .unwrap();
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

    // ── Stale-thread guard ──────────────────────────────────────────────
    //
    // Scenario: the user says "draft a reply to the last email from Ana"
    // but Ana's last email sits mid-thread — others (Oscar, the user
    // themself) have replied since. Replying to Ana's stale message would
    // be anachronistic. The tool must abort, hand the LLM the full thread
    // so it can surface a chip list, and wait for the user to confirm via
    // `acknowledge_not_latest=true`.

    #[tokio::test]
    async fn reply_to_non_latest_in_thread_aborts_without_drafting() {
        let db = Arc::new(Database::new_for_testing().expect("db"));
        // Three messages in one thread, requesting reply to the OLDEST.
        seed_email(&db, "msg-old", "acc", "thread-1", "Ana", "ana@x.com", "Re: Topic", 100);
        seed_email(
            &db,
            "msg-mid",
            "acc",
            "thread-1",
            "Oscar",
            "oscar@x.com",
            "Re: Topic",
            200,
        );
        seed_email(
            &db,
            "msg-new",
            "acc",
            "thread-1",
            "Gero",
            "gero@x.com",
            "Re: Topic",
            300,
        );

        let tool = GenerateEmailDraftTool;
        let args = serde_json::json!({ "email_id": "msg-old" });
        let out = tool.execute(&ctx_with_db(&db, "acc"), args).await.unwrap();

        // No draft saved, no composer opened.
        assert!(out.effects.is_empty(), "should not open composer: {:?}", out.effects);
        assert!(out.draft_refs.is_empty(), "no draft id should be allowlisted");
        // Warning text must name the situation so the LLM relays it.
        assert!(
            out.text.to_lowercase().contains("not the latest") || out.text.to_lowercase().contains("newer"),
            "expected stale-thread warning, got: {}",
            out.text
        );
        // Every thread message id must be in email_refs so the LLM can
        // wrap them as `email://` chips when relaying to the user.
        assert!(
            out.email_refs.contains(&"msg-old".to_string()),
            "refs: {:?}",
            out.email_refs
        );
        assert!(
            out.email_refs.contains(&"msg-mid".to_string()),
            "refs: {:?}",
            out.email_refs
        );
        assert!(
            out.email_refs.contains(&"msg-new".to_string()),
            "refs: {:?}",
            out.email_refs
        );
        // Each message must appear in the text by id so the LLM has a
        // concrete handle for the user-facing chips.
        assert!(out.text.contains("msg-old"), "text: {}", out.text);
        assert!(out.text.contains("msg-mid"), "text: {}", out.text);
        assert!(out.text.contains("msg-new"), "text: {}", out.text);
        // Mention `acknowledge_not_latest` so the LLM knows how to retry
        // once the user confirms.
        assert!(
            out.text.contains("acknowledge_not_latest"),
            "text must explain the retry path: {}",
            out.text
        );
    }

    #[tokio::test]
    async fn reply_to_latest_in_thread_does_not_trip_the_guard() {
        // The latest message in the thread is the one being replied to —
        // the guard must NOT fire. (Draft generation will still fail
        // because there's no AI provider configured, but the failure must
        // NOT be the stale-thread warning.)
        let db = Arc::new(Database::new_for_testing().expect("db"));
        seed_email(&db, "msg-old", "acc", "thread-1", "Ana", "ana@x.com", "Re: Topic", 100);
        seed_email(
            &db,
            "msg-new",
            "acc",
            "thread-1",
            "Oscar",
            "oscar@x.com",
            "Re: Topic",
            200,
        );

        let tool = GenerateEmailDraftTool;
        let args = serde_json::json!({ "email_id": "msg-new" });
        let out = tool.execute(&ctx_with_db(&db, "acc"), args).await.unwrap();

        assert!(
            !out.text.to_lowercase().contains("not the latest") && !out.text.to_lowercase().contains("newer message"),
            "guard fired on the latest message: {}",
            out.text
        );
    }

    #[tokio::test]
    async fn acknowledge_not_latest_bypasses_the_guard() {
        // Same stale-thread setup as the first test, but with the
        // acknowledge flag set — the guard must yield and let the normal
        // reply path run (which will fail at AI generation, not at the
        // guard).
        let db = Arc::new(Database::new_for_testing().expect("db"));
        seed_email(&db, "msg-old", "acc", "thread-1", "Ana", "ana@x.com", "Re: Topic", 100);
        seed_email(
            &db,
            "msg-new",
            "acc",
            "thread-1",
            "Oscar",
            "oscar@x.com",
            "Re: Topic",
            200,
        );

        let tool = GenerateEmailDraftTool;
        let args = serde_json::json!({
            "email_id": "msg-old",
            "acknowledge_not_latest": true,
        });
        let out = tool.execute(&ctx_with_db(&db, "acc"), args).await.unwrap();

        assert!(
            !out.text.to_lowercase().contains("not the latest") && !out.text.to_lowercase().contains("newer message"),
            "guard fired despite acknowledge_not_latest=true: {}",
            out.text
        );
    }

    #[test]
    fn schema_advertises_acknowledge_not_latest_parameter() {
        // The LLM picks this flag up from the schema. If it's missing,
        // the model can't retry — and the guard becomes a dead end.
        let tool = GenerateEmailDraftTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").expect("properties");
        assert!(
            props.get("acknowledge_not_latest").is_some(),
            "schema must declare acknowledge_not_latest: {}",
            schema
        );
    }
}
