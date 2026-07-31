//! Run orchestration for Lenses: backfill, incremental, re-extract, single-row.
//!
//! Heavy work is intended to run on `AppState::ai_background` (concurrency 1)
//! so an in-flight backfill never starves the interactive `ai_queue`. The
//! Tauri command layer is responsible for submitting these futures to the queue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::services::app_handle::AppHandle;

use crate::ai::provider::AIProvider;
use crate::db::Database;
use crate::models::error::Result;
use crate::models::lens::{Lens, LensRunKind};

use super::emit_log;
use super::extractor::{self, ExtractionStatus};
use super::scope;

fn is_cancelled(flag: Option<&Arc<AtomicBool>>) -> bool {
    flag.is_some_and(|f| f.load(Ordering::Relaxed))
}

/// Run a backfill: extract every scope-matching email that isn't already
/// covered by an existing `lens_rows` entry (any status) or `lens_exclusions`.
///
/// Returns the run id (also written to `lens_runs`).
pub async fn backfill_lens(
    db: Arc<Database>,
    provider: Arc<dyn AIProvider>,
    lens_id: String,
    app: Option<AppHandle>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<String> {
    let lens = db.get_lens(&lens_id)?;

    // Evaluate scope first so we know `total` before inserting the run row —
    // that way the UI can render a real progress bar from the very first poll
    // instead of showing "running" with no denominator.
    let candidates = scope::evaluate(&db, &lens.scope)?;
    let total = candidates.len() as i64;
    let run_id = db.insert_lens_run(&lens_id, LensRunKind::Backfill, total)?;

    emit_log(
        app.as_ref(),
        "info",
        format!("Lens '{}': backfill started ({} candidates)", lens.name, total),
    );

    // Best-effort warm-up — first call on llamacpp loads the GGUF.
    let _ = provider.warmup().await;

    let mut processed = 0i64;
    let mut succeeded = 0i64;
    let mut failed = 0i64;

    let mut cancelled = false;
    for email_id in candidates {
        if is_cancelled(cancel.as_ref()) {
            cancelled = true;
            break;
        }
        // Skip emails that already have a non-failed row OR are excluded.
        // Failed rows are retried on subsequent backfill runs so that schema
        // or prompt fixes can recover from a previously broken run.
        if db.lens_row_completed_or_excluded(&lens_id, &email_id).unwrap_or(false) {
            continue;
        }

        match extract_one(&db, provider.clone(), &lens, &email_id, app.as_ref()).await {
            Ok(ExtractionStatus::Ok) => succeeded += 1,
            Ok(ExtractionStatus::Failed) => failed += 1,
            Err(e) => {
                // Hard pipeline error (DB write failed, etc.) — abort the run.
                let msg = format!("backfill aborted: {e}");
                db.finish_lens_run(&run_id, "failed", Some(&msg))?;
                emit_log(app.as_ref(), "error", format!("Lens '{}': {msg}", lens.name));
                return Err(e);
            }
        }
        processed += 1;

        // Cheap progress update every 5 rows to bound write traffic.
        // Also emit an app-log event so the frontend's status listener wakes
        // and refreshes the on-screen counter — otherwise long runs look stuck.
        if processed % 5 == 0 {
            db.update_lens_run_progress(&run_id, processed, succeeded, failed)?;
            emit_log(
                app.as_ref(),
                "debug",
                format!(
                    "Lens '{}': backfill {}/{} processed ({} ok, {} failed)",
                    lens.name, processed, total, succeeded, failed
                ),
            );
        }
    }

    db.update_lens_run_progress(&run_id, processed, succeeded, failed)?;
    if cancelled {
        db.finish_lens_run(&run_id, "cancelled", Some("cancelled by user"))?;
        emit_log(
            app.as_ref(),
            "info",
            format!(
                "Lens '{}': backfill cancelled ({} ok, {} failed)",
                lens.name, succeeded, failed
            ),
        );
    } else {
        db.finish_lens_run(&run_id, "complete", None)?;
        emit_log(
            app.as_ref(),
            "success",
            format!(
                "Lens '{}': backfill complete ({} ok, {} failed, {} skipped)",
                lens.name,
                succeeded,
                failed,
                total - processed,
            ),
        );
    }
    Ok(run_id)
}

/// Re-extract every row whose `prompt_version < lens.prompt_version`.
/// Overrides are preserved by the `upsert_lens_row` UPSERT shape.
pub async fn reextract_lens(
    db: Arc<Database>,
    provider: Arc<dyn AIProvider>,
    lens_id: String,
    app: Option<AppHandle>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<String> {
    let lens = db.get_lens(&lens_id)?;
    let stale = db.list_stale_row_email_ids(&lens_id)?;
    let total = stale.len() as i64;
    let run_id = db.insert_lens_run(&lens_id, LensRunKind::Reextract, total)?;

    emit_log(
        app.as_ref(),
        "info",
        format!("Lens '{}': re-extracting {} rows", lens.name, total),
    );

    let _ = provider.warmup().await;

    let mut processed = 0i64;
    let mut succeeded = 0i64;
    let mut failed = 0i64;

    let mut cancelled = false;
    for email_id in stale {
        if is_cancelled(cancel.as_ref()) {
            cancelled = true;
            break;
        }
        match extract_one(&db, provider.clone(), &lens, &email_id, app.as_ref()).await {
            Ok(ExtractionStatus::Ok) => succeeded += 1,
            Ok(ExtractionStatus::Failed) => failed += 1,
            Err(e) => {
                let msg = format!("re-extract aborted: {e}");
                db.finish_lens_run(&run_id, "failed", Some(&msg))?;
                return Err(e);
            }
        }
        processed += 1;
        if processed % 5 == 0 {
            db.update_lens_run_progress(&run_id, processed, succeeded, failed)?;
            emit_log(
                app.as_ref(),
                "debug",
                format!(
                    "Lens '{}': re-extract {}/{} processed ({} ok, {} failed)",
                    lens.name, processed, total, succeeded, failed
                ),
            );
        }
    }

    db.update_lens_run_progress(&run_id, processed, succeeded, failed)?;
    if cancelled {
        db.finish_lens_run(&run_id, "cancelled", Some("cancelled by user"))?;
        emit_log(
            app.as_ref(),
            "info",
            format!(
                "Lens '{}': re-extract cancelled ({} ok, {} failed)",
                lens.name, succeeded, failed
            ),
        );
    } else {
        db.finish_lens_run(&run_id, "complete", None)?;
        emit_log(
            app.as_ref(),
            "success",
            format!(
                "Lens '{}': re-extract complete ({} ok, {} failed)",
                lens.name, succeeded, failed
            ),
        );
    }
    Ok(run_id)
}

/// Re-extract a single row (user-triggered retry from the row context menu).
pub async fn reextract_row(
    db: Arc<Database>,
    provider: Arc<dyn AIProvider>,
    lens_id: String,
    email_id: String,
    app: Option<AppHandle>,
) -> Result<()> {
    let lens = db.get_lens(&lens_id)?;
    let run_id = db.insert_lens_run(&lens_id, LensRunKind::Single, 1)?;

    let result = extract_one(&db, provider, &lens, &email_id, app.as_ref()).await;
    let (succeeded, failed) = match &result {
        Ok(ExtractionStatus::Ok) => (1, 0),
        Ok(ExtractionStatus::Failed) => (0, 1),
        Err(_) => (0, 0),
    };
    db.update_lens_run_progress(&run_id, 1, succeeded, failed)?;
    match &result {
        Ok(_) => db.finish_lens_run(&run_id, "complete", None)?,
        Err(e) => {
            let msg = format!("single re-extract failed: {e}");
            db.finish_lens_run(&run_id, "failed", Some(&msg))?;
        }
    }
    result.map(|_| ())
}

/// Sync-hook entry point: filter `email_ids` against every enabled Lens and
/// extract the matching ones. Cheap on the hot path because we only re-evaluate
/// scope against the supplied IDs, not the whole mailbox.
///
/// Returns the total number of (lens, email) extractions performed.
pub async fn on_emails_synced(
    db: Arc<Database>,
    provider: Arc<dyn AIProvider>,
    email_ids: &[String],
    app: Option<&AppHandle>,
) -> Result<usize> {
    if email_ids.is_empty() {
        return Ok(0);
    }
    let lenses = db.list_lenses()?;
    let mut total = 0usize;

    for summary in lenses.iter().filter(|l| l.is_enabled) {
        let lens = match db.get_lens(&summary.id) {
            Ok(l) => l,
            Err(e) => {
                emit_log(app, "error", format!("Lens '{}': load failed: {e}", summary.name));
                continue;
            }
        };

        for email_id in email_ids {
            if email_matches_scope(&db, &lens, email_id)? {
                if db.lens_row_exists(&lens.id, email_id).unwrap_or(false) {
                    continue;
                }
                if let Err(e) = extract_one(&db, provider.clone(), &lens, email_id, app).await {
                    emit_log(
                        app,
                        "error",
                        format!("Lens '{}': incremental extract failed: {e}", lens.name),
                    );
                    continue;
                }
                total += 1;
            }
        }
    }
    Ok(total)
}

/// Cheap per-email scope match — runs the same evaluator but limited to a
/// single email. Implementation detail: we use a tiny in-memory check by
/// inspecting the email's columns directly so we don't pay an FTS query
/// per new email.
fn email_matches_scope(db: &Database, lens: &Lens, email_id: &str) -> Result<bool> {
    let email = match db.get_email_by_id(email_id)? {
        Some(e) => e,
        None => return Ok(false),
    };

    let scope = &lens.scope;
    if let Some(account_ids) = scope.account_ids.as_ref() {
        if !account_ids.contains(&email.account_id) {
            return Ok(false);
        }
    }
    if let Some(mailboxes) = scope.mailboxes.as_ref() {
        if !mailboxes.contains(&email.mailbox) {
            return Ok(false);
        }
    }
    if let Some(categories) = scope.categories.as_ref() {
        // Case-insensitive: the UI sends "Primary"/"Updates" (capitalized) but
        // sync writes email.category lowercase ("primary"/"updates"). Without
        // this normalization, on_emails_synced silently skips all inbox emails.
        let cat_lower = email.category.to_lowercase();
        if !categories.iter().any(|c| c.to_lowercase() == cat_lower) {
            return Ok(false);
        }
    }
    if let Some(senders) = scope.sender_emails.as_ref() {
        let lower = email.sender_email.to_lowercase();
        if !senders.iter().any(|s| s.to_lowercase() == lower) {
            return Ok(false);
        }
    }
    if let Some(domains) = scope.sender_domains.as_ref() {
        // sender_domain is denormalised lowercase already.
        let dom = email.sender_email.split('@').nth(1).unwrap_or("").to_lowercase();
        if !domains.iter().any(|d| d.to_lowercase() == dom) {
            return Ok(false);
        }
    }
    if let Some(direction) = scope.direction {
        use crate::models::lens::Direction;
        match direction {
            Direction::Inbound if email.mailbox == "sent" => return Ok(false),
            Direction::Outbound if email.mailbox != "sent" => return Ok(false),
            _ => {}
        }
    }
    if let Some(range) = scope.date_range.as_ref() {
        if let Some(days) = range.last_days {
            let cutoff = now_secs().saturating_sub(days.saturating_mul(86_400));
            if email.timestamp < cutoff {
                return Ok(false);
            }
        }
        if let Some(from) = range.from {
            if email.timestamp < from {
                return Ok(false);
            }
        }
        if let Some(to) = range.to {
            if email.timestamp > to {
                return Ok(false);
            }
        }
    }
    // FTS-keyword and tag filters are not cheap to check inline; if the Lens
    // uses them, fall back to the full scope evaluator scoped to this one id.
    if scope.query.is_some() || scope.tags.is_some() {
        let ids = scope::evaluate(db, scope)?;
        return Ok(ids.iter().any(|id| id == email_id));
    }
    Ok(true)
}

/// Run extraction for one email and persist the row. Returns the resolved
/// `ExtractionStatus` so the caller can keep per-run counters.
async fn extract_one(
    db: &Database,
    provider: Arc<dyn AIProvider>,
    lens: &Lens,
    email_id: &str,
    app: Option<&AppHandle>,
) -> Result<ExtractionStatus> {
    let email = match db.get_email_by_id(email_id)? {
        Some(e) => e,
        None => {
            emit_log(
                app,
                "warn",
                format!("Lens '{}': email {email_id} not found — skipping", lens.name),
            );
            return Ok(ExtractionStatus::Failed);
        }
    };

    let result = extractor::extract_email(db, provider, lens, email_id, app).await?;

    if result.status == ExtractionStatus::Failed {
        let reason = result.error_message.as_deref().unwrap_or("unknown reason");
        emit_log(
            app,
            "warn",
            format!(
                "Lens '{}': extraction failed for '{}' — {reason}",
                lens.name, email.subject
            ),
        );
    }

    let extracted_json = serde_json::to_string(&result.data)?;
    db.upsert_lens_row(
        &lens.id,
        email_id,
        &email.account_id,
        &extracted_json,
        lens.prompt_version,
        email.timestamp,
        result.status.as_str(),
        result.error_message.as_deref(),
    )?;
    Ok(result.status)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    //! End-to-end integration tests for the Lens runner.
    //!
    //! These tests substitute a `MockProvider` for the real AI backend so the
    //! whole pipeline — scope eval → prompt build → tool-call response →
    //! schema validation → `lens_rows` upsert — exercises every layer that
    //! ships in production. They guard against the "extractor runs but no
    //! row appears" failure mode the user hit on a real invoice.
    use super::*;
    use crate::ai::provider::{
        AiMessage, AiToolCall, AiToolCallFunction, BackendCapabilities, ChatStreamResult, CompletionOptions,
        CompletionResult, EmbeddingResult, ModelInfo, ProviderType,
    };
    use crate::db::Database;
    use crate::models::error::Result as AppResult;
    use crate::models::lens::{
        CreateLensInput, DateRange, Direction, LensColumn, LensColumnType, LensSchema, LensScope,
    };
    use async_trait::async_trait;

    /// Canned-response AIProvider. The `chat_with_tools` impl just returns
    /// whatever JSON value the test installed under `tool_args`, packaged as a
    /// tool_call against the `submit_extraction` function — exactly what
    /// llama.cpp / Ollama emit when they succeed.
    struct MockProvider {
        tool_args: serde_json::Value,
    }

    #[async_trait]
    impl crate::ai::provider::AIProvider for MockProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::LlamaCpp
        }
        fn model_name(&self) -> &str {
            "mock"
        }
        fn embedding_model_name(&self) -> &str {
            "mock-embed"
        }
        async fn is_available(&self) -> bool {
            true
        }
        async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
            Ok(vec![])
        }
        async fn list_embedding_models(&self) -> AppResult<Vec<ModelInfo>> {
            Ok(vec![])
        }
        async fn complete(&self, _prompt: &str, _opts: CompletionOptions) -> AppResult<CompletionResult> {
            unimplemented!("not used in lens tests")
        }
        async fn embed(&self, _text: &str) -> AppResult<EmbeddingResult> {
            unimplemented!("not used in lens tests")
        }
        async fn embed_batch(&self, _texts: &[String]) -> AppResult<Vec<EmbeddingResult>> {
            unimplemented!("not used in lens tests")
        }
        async fn chat_with_tools(&self, _messages: &[AiMessage], _tools: &[serde_json::Value]) -> AppResult<AiMessage> {
            Ok(AiMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: Some(vec![AiToolCall {
                    function: AiToolCallFunction {
                        name: "submit_extraction".into(),
                        arguments: self.tool_args.clone(),
                    },
                }]),
            })
        }
        async fn chat_stream(
            &self,
            _messages: Vec<AiMessage>,
            _on_token: Box<dyn FnMut(String) -> bool + Send>,
        ) -> AppResult<ChatStreamResult> {
            unimplemented!("not used in lens tests")
        }
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities {
                tools: true,
                streaming: false,
                embeddings: false,
            }
        }
    }

    fn insert_account(db: &Database, id: &str, email: &str) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?2, 'Test', 0)",
                rusqlite::params![id, email],
            )
            .expect("insert account");
    }

    /// Insert an email with full FTS-searchable fields, mirroring what the
    /// Gmail sync writes. Both `emails` and `emails_fts` are populated so the
    /// scope evaluator's FTS5 query branch works.
    #[allow(clippy::too_many_arguments)]
    fn insert_invoice_email(
        db: &Database,
        id: &str,
        account_id: &str,
        sender_name: &str,
        sender_email: &str,
        subject: &str,
        body: &str,
        mailbox: &str,
        category: &str,
        ts: i64,
    ) {
        let sender_domain = sender_email
            .rsplit_once('@')
            .map(|(_, d)| d.to_lowercase())
            .unwrap_or_default();
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
            rusqlite::params![account_id],
        )
        .expect("seed account");
        conn.execute(
            "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '[]', '[]', ?8, ?9, 0, ?10, ?11, ?9)",
            rusqlite::params![
                id,
                account_id,
                format!("t-{id}"),
                subject,
                sender_name,
                sender_email,
                sender_domain,
                body.chars().take(100).collect::<String>(),
                ts,
                category,
                mailbox,
            ],
        )
        .expect("insert email");
        conn.execute(
            "INSERT INTO email_bodies (email_id, body) VALUES (?1, ?2)",
            rusqlite::params![id, body],
        )
        .expect("insert email body");
        // Mirror the sync's FTS insert. emails_fts is the FTS5 virtual table.
        conn.execute(
            "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, subject, sender_name, body],
        )
        .expect("insert fts");
    }

    fn invoices_lens_input(account_id: &str) -> CreateLensInput {
        // The scope mirrors what a corrected version of the user's UI config
        // would send: account = gmail, inbox only, Primary+Updates, inbound,
        // last 30 days, and the FTS query (with straight quotes only).
        let scope = LensScope {
            account_ids: Some(vec![account_id.into()]),
            mailboxes: Some(vec!["inbox".into()]),
            categories: Some(vec!["Primary".into(), "Updates".into()]),
            direction: Some(Direction::Inbound),
            date_range: Some(DateRange {
                last_days: Some(30),
                from: None,
                to: None,
            }),
            query: Some("invoice OR receipt OR \"amount due\" OR factura".into()),
            ..Default::default()
        };
        let schema = LensSchema {
            columns: vec![
                LensColumn {
                    key: "vendor".into(),
                    label: "Vendor".into(),
                    column_type: LensColumnType::String,
                    description: "Vendor name".into(),
                    enum_values: None,
                    required: true,
                    is_unique_key: false,
                },
                LensColumn {
                    key: "amount".into(),
                    label: "Amount".into(),
                    column_type: LensColumnType::Currency,
                    description: "Total due".into(),
                    enum_values: None,
                    required: true,
                    is_unique_key: false,
                },
                LensColumn {
                    key: "invoice_number".into(),
                    label: "Invoice #".into(),
                    column_type: LensColumnType::String,
                    description: "Invoice number".into(),
                    enum_values: None,
                    required: false,
                    is_unique_key: true,
                },
                LensColumn {
                    key: "status".into(),
                    label: "Status".into(),
                    column_type: LensColumnType::Enum,
                    description: "Status".into(),
                    enum_values: Some(vec!["unpaid".into(), "paid".into(), "overdue".into(), "unknown".into()]),
                    required: true,
                    is_unique_key: false,
                },
            ],
        };
        CreateLensInput {
            name: "Invoices received".into(),
            icon: Some("🧾".into()),
            template_key: Some("invoices_received".into()),
            account_id: None,
            scope,
            schema,
            prompt_text: "Extract invoice fields.".into(),
            model_provider: None,
            model_name: None,
        }
    }

    /// Full happy path: scope matches the invoice email, mock provider returns
    /// schema-valid tool args, `extract_one` persists a `lens_rows` row, and
    /// `get_lens_rows` returns it with the fields exposed under `data`.
    #[tokio::test]
    async fn extracts_and_persists_invoice_row() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let acct = "acct1";
        insert_account(&db, acct, "alex@northwindlabs.io");

        // Real invoice-like email — Spanish "factura" so the FTS query hits.
        let now = now_secs();
        let email_id = "19e3f60ef87746cc";
        insert_invoice_email(
            &db,
            email_id,
            acct,
            "Impact Hub Madrid Barceló",
            "madrid.barcelo@impacthub.net",
            "Tu factura de marzo",
            "Hola, adjuntamos tu factura número INV-2026-0142 por un importe de 145,00 EUR. \
             El pago vence el 2026-04-15.",
            "inbox",
            "Primary",
            now - 86_400, // yesterday — well within 30 day window
        );

        // Sanity: scope alone should pick the email up. If this fails, the
        // user's filter has nothing to extract from — diagnose before going
        // further.
        let lens = db.create_lens(&invoices_lens_input(acct)).expect("create lens");
        let picks = scope::evaluate(&db, &lens.scope).expect("scope eval");
        assert!(
            picks.iter().any(|p| p == email_id),
            "scope did not match email_id={email_id}; got {picks:?}"
        );

        // Mock the AI: return a fully-populated, schema-valid tool_call.
        let provider = Arc::new(MockProvider {
            tool_args: serde_json::json!({
                "vendor": "Impact Hub Madrid",
                "amount": { "amount": 145.0, "currency": "EUR" },
                "invoice_number": "INV-2026-0142",
                "status": "unpaid",
            }),
        });

        // Run the full backfill: this also exercises the run-lifecycle
        // bookkeeping (lens_runs insert, processed/succeeded counters,
        // finish_lens_run).
        backfill_lens(db.clone(), provider.clone(), lens.id.clone(), None, None)
            .await
            .expect("backfill");

        // Row must exist with status=ok and the mocked data.
        let page = db.get_lens_rows(&lens.id, None, 50, 0).expect("get lens rows");
        assert_eq!(page.rows.len(), 1, "expected exactly one row");
        let row = &page.rows[0];
        assert_eq!(row.email_id, email_id);
        assert_eq!(row.status, "ok");
        assert_eq!(row.data["vendor"], "Impact Hub Madrid");
        assert_eq!(row.data["amount"]["amount"], 145.0);
        assert_eq!(row.data["amount"]["currency"], "EUR");
        assert_eq!(row.data["invoice_number"], "INV-2026-0142");
        assert_eq!(row.data["status"], "unpaid");

        // Run history reflects 1 processed / 1 succeeded / 0 failed.
        let runs = db.list_lens_runs(&lens.id, 5).expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "complete");
        assert_eq!(runs[0].succeeded, 1);
        assert_eq!(runs[0].failed, 0);
    }

    /// Reproduces the user's mis-configuration: they put a full email address
    /// in the `senderDomains` field. The scope evaluator does exact-match on
    /// the email's denormalized `sender_domain` (here `impacthub.net`), so
    /// the filter rejects everything and zero rows are extracted. This test
    /// pins that behavior so the UI / docs guidance stays honest.
    #[tokio::test]
    async fn sender_domain_field_with_full_email_yields_no_matches() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let acct = "acct1";
        insert_account(&db, acct, "alex@northwindlabs.io");

        let now = now_secs();
        let email_id = "19e3f60ef87746cc";
        insert_invoice_email(
            &db,
            email_id,
            acct,
            "Impact Hub",
            "madrid.barcelo@impacthub.net",
            "Tu factura de marzo",
            "Factura INV-2026-0142 importe 145,00 EUR.",
            "inbox",
            "Primary",
            now - 86_400,
        );

        let mut input = invoices_lens_input(acct);
        // The user's exact mistake: a full email in senderDomains.
        input.scope.sender_domains = Some(vec!["madrid.barcelo@impacthub.net".into()]);

        let lens = db.create_lens(&input).expect("create lens");
        let picks = scope::evaluate(&db, &lens.scope).expect("scope eval");
        assert!(
            picks.is_empty(),
            "domain field with a full email should NOT match (sender_domain is 'impacthub.net'); got {picks:?}"
        );

        // And the correct configuration — same value moved to senderEmails —
        // should match.
        let mut fixed = invoices_lens_input(acct);
        fixed.name = "Invoices received fixed".into();
        fixed.scope.sender_emails = Some(vec!["madrid.barcelo@impacthub.net".into()]);
        let lens2 = db.create_lens(&fixed).expect("create lens (fixed)");
        let picks2 = scope::evaluate(&db, &lens2.scope).expect("scope eval (fixed)");
        assert!(
            picks2.iter().any(|p| p == email_id),
            "senderEmails with the full address should match the email"
        );
    }
}
