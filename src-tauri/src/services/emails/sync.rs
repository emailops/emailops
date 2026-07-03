use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tauri::AppHandle;
use tokio::time::sleep;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, Email};
use crate::services::accounts;
use crate::sync::provider::{EmailProvider, ExtraMailbox};

use super::events::{emit_account_log, emit_progress};
use super::provider::build_provider_for_account;

/// Public entry point called by Tauri commands and the sync scheduler.
/// Builds the OAuth provider (refreshing tokens if needed), acquires the
/// per-account sync lock, then delegates to [`sync_account_with_provider`].
pub async fn sync_account(
    db: &Arc<Database>,
    account_id: &str,
    app_data_dir: &Path,
    app: Option<AppHandle>,
    ai_background: crate::services::task_queue::TaskQueue,
    sync_abort_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    sync_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
) -> Result<()> {
    // Acquire per-account lock atomically. If another sync is already running
    // for this account (scheduler tick racing with UI trigger, or double-click),
    // bail out immediately rather than running two downloads in parallel.
    let account_lock = {
        // Recover from a poisoned mutex instead of panicking — the inner map
        // is just a per-account lock registry; previous panics don't corrupt it.
        let mut locks = sync_locks.lock().unwrap_or_else(PoisonError::into_inner);
        locks
            .entry(account_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _sync_guard = match account_lock.try_lock() {
        Ok(g) => g,
        Err(_) => {
            crate::services::logger::log(
                "debug",
                "sync",
                format!("sync already in progress for {account_id}, skipping"),
            );
            return Ok(());
        }
    };

    db.upsert_sync_status(account_id, "syncing", None, None)?;

    // Scope guard: if this function exits via `?`-error propagation, panic, or
    // future cancellation before we mark `completed = true`, reset sync_status
    // to "error" so the poll loop can retry on the next tick.
    struct SyncStatusGuard {
        db: Arc<Database>,
        account_id: String,
        completed: bool,
    }
    impl Drop for SyncStatusGuard {
        fn drop(&mut self) {
            if !self.completed {
                if let Err(e) = self.db.upsert_sync_status(
                    &self.account_id,
                    "error",
                    None,
                    Some("Sync interrupted (network error or crash)"),
                ) {
                    crate::services::logger::log(
                        "error",
                        "sync",
                        format!("failed to reset stuck sync_status for {}: {}", self.account_id, e),
                    );
                }
            }
        }
    }
    let mut status_guard = SyncStatusGuard {
        db: db.clone(),
        account_id: account_id.to_string(),
        completed: false,
    };

    // Get account
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?;

    if !account.enabled {
        db.upsert_sync_status(account_id, "idle", None, None)?;
        emit_progress(account_id, "complete", 0, 0, "Account disabled — sync skipped");
        status_guard.completed = true;
        return Ok(());
    }

    emit_progress(account_id, "starting", 0, 0, "Connecting to email provider...");

    // Notify if a token refresh is about to happen so the UI shows "Refreshing credentials..."
    if account.provider == "gmail" || account.provider == "outlook" {
        let needs_refresh = accounts::get_tokens(account_id)
            .ok()
            .and_then(|t| t.expires_at)
            .map(|exp| exp < chrono::Utc::now().timestamp() + 300)
            .unwrap_or(true);
        if needs_refresh {
            emit_progress(account_id, "refreshing", 0, 0, "Refreshing credentials...");
        }
    }

    // Build provider — refreshes OAuth tokens if needed.
    let email_provider = build_provider_for_account(&account, app.clone()).await?;

    let result = sync_account_with_provider(
        db,
        &account,
        app_data_dir,
        app,
        ai_background,
        sync_abort_flags,
        email_provider,
    )
    .await;

    if result.is_ok() {
        status_guard.completed = true;
    }
    result
}

/// Core sync logic with an already-built `EmailProvider`. Accepts
/// `app: Option<AppHandle>` so integration tests can pass `None` and use a
/// `FakeEmailProvider` without needing a live Tauri runtime.
///
/// When `app` is `None`:
/// - `sync-progress` Tauri events are silently dropped (logged via global seam).
/// - AI follow-up tasks (classification, memory, embeddings) are skipped.
/// - Attachment auto-download and rule processing are skipped.
///
/// Callers (commands, scheduler) always pass `Some(app)`. Tests pass `None`.
pub async fn sync_account_with_provider(
    db: &Arc<Database>,
    account: &Account,
    app_data_dir: &Path,
    app: Option<AppHandle>,
    ai_background: crate::services::task_queue::TaskQueue,
    sync_abort_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    email_provider: Box<dyn EmailProvider>,
) -> Result<()> {
    let account_id = &account.id;

    // Load account settings: Gmail category filter + attachment auto-download categories
    let (label_filter_for_list, skip_promotions, auto_download_attachment_categories) = {
        let key = format!("account_settings:{}", account_id);
        let settings = db
            .get_preference(&key)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str::<crate::models::AccountSettings>(&json).ok())
            .unwrap_or_default();

        if account.provider == "gmail" {
            let filter = if settings.gmail_categories.is_empty() {
                Some("in:sent".to_string())
            } else {
                let clause = settings
                    .gmail_categories
                    .iter()
                    .map(|c| format!("category:{}", c))
                    .collect::<Vec<_>>()
                    .join(" OR ");
                Some(format!("({} OR in:sent)", clause))
            };
            let skip_promo = !settings.gmail_categories.contains(&"promotions".to_string());
            (filter, skip_promo, settings.auto_download_attachment_categories)
        } else {
            (None, false, settings.auto_download_attachment_categories)
        }
    };

    // Load previously failed emails so we can retry them at the end of this sync
    let failed_emails_to_retry = db.get_failed_emails(account_id).unwrap_or_default();

    // Inbox watermark MUST be scoped to inbox-only rows. Using the global
    // MAX(timestamp) across all mailboxes lets a locally stored sent email
    // (e.g. a reply the user just composed) push the incremental cursor
    // past unsynced received emails that arrived between syncs at an
    // earlier timestamp — Gmail's `after:T_sent` then filters them out.
    let latest_timestamp = db.get_latest_email_timestamp_for_mailbox(account_id, "inbox")?;
    let oldest_timestamp = db.get_oldest_email_timestamp(account_id)?;
    let effective_sync_from = account.sync_from_timestamp.or(match account.provider.as_str() {
        "imap" | "outlook" | "gmail" => Some(0),
        _ => None,
    });
    let plan = plan_sync_passes(effective_sync_from, latest_timestamp, oldest_timestamp);
    let backfill_after_timestamp = plan.backfill_after_timestamp;
    let backfill_before_timestamp = plan.backfill_before_timestamp;
    // Incremental's anchor only matters when the planner says to run it.
    // We still fall back to sync_from / 0 so a future planner change that
    // re-enables incremental on a fresh DB has a sensible starting point.
    let incremental_after_timestamp = if plan.run_incremental {
        latest_timestamp
            .or(account.sync_from_timestamp)
            .or(match account.provider.as_str() {
                "imap" | "outlook" | "gmail" => Some(0),
                _ => None,
            })
    } else {
        None
    };

    emit_progress(
        account_id,
        "fetching",
        0,
        0,
        &match (
            backfill_after_timestamp,
            backfill_before_timestamp,
            incremental_after_timestamp,
        ) {
            (Some(from), Some(to), _) => format!(
                "Backfilling emails from {} to {}...",
                chrono::DateTime::from_timestamp(from, 0)
                    .map(|dt| dt.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "selected date".to_string()),
                chrono::DateTime::from_timestamp(to, 0)
                    .map(|dt| dt.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "current oldest email".to_string())
            ),
            (_, _, Some(timestamp)) => format!(
                "Checking for emails since {}...",
                chrono::DateTime::from_timestamp(timestamp, 0)
                    .map(|dt| dt.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "configured sync date".to_string())
            ),
            _ => "Checking for new emails...".to_string(),
        },
    );

    const MAX_INCREMENTAL_EMAILS_PER_SYNC: u32 = 500;
    const PAGE_SIZE: u32 = 100;

    let mut all_message_refs = Vec::new();
    let mut backfill_ref_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // ── Backfill pass ─────────────────────────────────────────────────────────
    if backfill_after_timestamp.is_some() {
        let mut next_page_token: Option<String> = None;
        loop {
            let (message_refs, next_page) = email_provider
                .list_messages(
                    PAGE_SIZE,
                    next_page_token.as_deref(),
                    backfill_after_timestamp,
                    backfill_before_timestamp,
                    label_filter_for_list.as_deref(),
                )
                .await?;
            for r in &message_refs {
                backfill_ref_ids.insert(r.id.clone());
            }
            all_message_refs.extend(message_refs);
            if next_page.is_none() {
                break;
            }
            emit_account_log(
                "debug",
                "sync",
                &account.email,
                &format!(
                    "Found {} backfill message IDs so far, fetching more...",
                    backfill_ref_ids.len()
                ),
            );
            next_page_token = next_page;
        }
    }

    // ── Incremental pass ──────────────────────────────────────────────────────
    if incremental_after_timestamp.is_some() {
        let mut next_page_token: Option<String> = None;
        loop {
            let remaining = MAX_INCREMENTAL_EMAILS_PER_SYNC
                .saturating_sub(all_message_refs.len().saturating_sub(backfill_ref_ids.len()) as u32);
            if remaining == 0 {
                break;
            }
            let page_size = PAGE_SIZE.min(remaining);
            let (message_refs, next_page) = email_provider
                .list_messages(
                    page_size,
                    next_page_token.as_deref(),
                    incremental_after_timestamp,
                    None,
                    label_filter_for_list.as_deref(),
                )
                .await?;
            all_message_refs.extend(message_refs);
            let has_more = next_page.is_some()
                && all_message_refs.len().saturating_sub(backfill_ref_ids.len())
                    < MAX_INCREMENTAL_EMAILS_PER_SYNC as usize;
            if has_more {
                emit_account_log(
                    "debug",
                    "sync",
                    &account.email,
                    &format!("Found {} message IDs so far, fetching more...", all_message_refs.len()),
                );
            }
            if !has_more {
                break;
            }
            next_page_token = next_page;
        }
    }

    // Still filter by ID in case of overlap at the timestamp boundary.
    let all_ids: Vec<String> = all_message_refs.iter().map(|r| r.id.clone()).collect();
    let existing_ids = db.emails_exist_batch(&all_ids)?;
    let new_message_refs: Vec<_> = all_message_refs
        .into_iter()
        .filter(|msg_ref| !existing_ids.contains(&msg_ref.id))
        .collect();

    if backfill_after_timestamp.is_some() && backfill_before_timestamp.is_some() {
        let has_new_backfill = new_message_refs.iter().any(|r| backfill_ref_ids.contains(&r.id));
        if !has_new_backfill {
            if let Err(e) = db.update_account_sync_from(account_id, backfill_before_timestamp) {
                emit_account_log(
                    "warn",
                    "sync",
                    &account.email,
                    &format!("Failed to advance backfill watermark: {}", e),
                );
            }
        }
    }

    let new_count = new_message_refs.len() as u32;

    if new_count == 0 {
        // Inbox has no new emails — but Sent / Spam / Trash still need
        // their dedicated pass so a stale inbox doesn't gate sent-mail
        // recovery. This was the original 2024 → 2025 Sent gap bug:
        // a near-idle account never reached the extra-mailbox sync.
        if let Err(e) = sync_extra_mailboxes(db, account, account_id, email_provider.as_ref()).await {
            emit_account_log(
                "warn",
                "sync",
                &account.email,
                &format!("Extra mailbox sync failed (non-fatal): {}", e),
            );
        }
        pull_drafts_if_supported(db, account, account_id, email_provider.as_ref()).await;

        db.upsert_sync_status(account_id, "idle", Some(chrono::Utc::now().timestamp()), None)?;
        // Terminal progress event clears the UI spinner. No output-panel log
        // line: an idle sync (nothing new) should stay quiet. `current/total`
        // are 0 so the frontend skips logging this completion.
        emit_progress(account_id, "complete", 0, 0, "Inbox up to date");

        if let Some(ref a) = app {
            enqueue_ai_followups(db, a, account_id, &account.email, &ai_background, "no_new").await;
        }

        return Ok(());
    }

    emit_progress(
        account_id,
        "syncing",
        0,
        new_count,
        &format!(
            "Downloading {} new email{}...",
            new_count,
            if new_count == 1 { "" } else { "s" }
        ),
    );
    emit_account_log(
        "info",
        "sync",
        &account.email,
        &format!(
            "Downloading {} new email{}...",
            new_count,
            if new_count == 1 { "" } else { "s" }
        ),
    );

    let mut synced_count: u32 = 0;
    let mut all_new_ids: Vec<String> = Vec::new();
    let mut ai_followups_kicked = false;

    let log_every = if new_count < 50 {
        new_count
    } else {
        (new_count / 10).clamp(1, 50)
    };

    // Load attachment rules for this account (empty vec if none defined)
    let attachment_rules = db.get_attachment_rules(account_id)?;

    const BATCH_SIZE: usize = 20;
    const INTER_BATCH_DELAY_MS: u64 = 2_000;

    let mut global_done: u32 = 0;
    let mut first_batch = true;
    let mut window_oldest_ts: Option<i64> = None;
    let mut window_newest_ts: Option<i64> = None;
    let format_window_date = |ts: i64| -> String {
        chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| ts.to_string())
    };
    for chunk in new_message_refs.chunks(BATCH_SIZE) {
        // Exit early if this account was deleted while sync was in progress.
        if sync_abort_flags
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(account_id)
            .map(|f| f.load(Ordering::Relaxed))
            .unwrap_or(false)
        {
            sync_abort_flags
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(account_id);
            return Ok(());
        }

        if !first_batch {
            sleep(Duration::from_millis(INTER_BATCH_DELAY_MS)).await;
        }
        first_batch = false;

        let ids: Vec<&str> = chunk.iter().map(|r| r.id.as_str()).collect();

        let batch_results = email_provider.batch_get_messages(&ids).await?;

        let mut chunk_emails: Vec<(Email, Vec<crate::sync::provider::AttachmentInfo>)> = Vec::new();

        for (msg_ref, result) in chunk.iter().zip(batch_results) {
            global_done += 1;

            if let Ok(ref r) = result {
                let ts = r.0.timestamp;
                window_oldest_ts = Some(window_oldest_ts.map_or(ts, |o| o.min(ts)));
                window_newest_ts = Some(window_newest_ts.map_or(ts, |n| n.max(ts)));
            }

            emit_progress(
                account_id,
                "syncing",
                global_done,
                new_count,
                &format!("Downloading {} of {} new emails", global_done, new_count),
            );
            if global_done.is_multiple_of(log_every) || global_done == new_count {
                let range = match (window_oldest_ts.take(), window_newest_ts.take()) {
                    (Some(oldest), Some(newest)) => {
                        format!(
                            " (received {} → {})",
                            format_window_date(oldest),
                            format_window_date(newest),
                        )
                    }
                    _ => String::new(),
                };
                emit_account_log(
                    "debug",
                    "sync",
                    &account.email,
                    &format!("Downloaded {} / {} emails{}", global_done, new_count, range),
                );
            }

            let (mut email, category, attachment_infos) = match result {
                Ok(r) => r,
                Err(e) => {
                    let err_str = e.to_string();
                    emit_account_log(
                        "error",
                        "sync",
                        &account.email,
                        &format!("Failed to download email {}: {}", msg_ref.id, err_str),
                    );
                    if let Err(db_err) = db.add_failed_email(account_id, &msg_ref.id, &err_str) {
                        emit_account_log(
                            "error",
                            "sync",
                            &account.email,
                            &format!("Could not record email failure: {}", db_err),
                        );
                    }
                    continue;
                }
            };

            // Skip promotions unless the account is configured to sync them
            if skip_promotions && category.is_promotions() {
                continue;
            }

            email.account_id = account_id.to_string();
            chunk_emails.push((email, attachment_infos));
        }

        // Phase 2: batch write — one write-lock acquisition for the whole chunk.
        if !chunk_emails.is_empty() {
            let emails_only: Vec<Email> = chunk_emails.iter().map(|(e, _)| e.clone()).collect();
            db.insert_emails_batch(&emails_only)?;

            let metas: Vec<_> = chunk_emails
                .iter()
                .flat_map(|(email, infos)| {
                    infos.iter().map(move |info| {
                        (
                            email.id.clone(),
                            account_id.to_string(),
                            info.attachment_id.clone(),
                            info.filename.clone(),
                            info.mime_type.clone(),
                            info.size,
                            info.inline_data.clone(),
                        )
                    })
                })
                .collect();
            if !metas.is_empty() {
                let _ = db.insert_email_attachment_metas_batch(&metas);
            }

            let ids_to_remove: Vec<String> = chunk_emails.iter().map(|(e, _)| e.id.clone()).collect();
            let _ = db.remove_failed_emails_batch(account_id, &ids_to_remove);

            all_new_ids.extend(ids_to_remove);
            synced_count += chunk_emails.len() as u32;

            emit_progress(account_id, "batch", synced_count, new_count, "");

            if !ai_followups_kicked {
                if let Some(ref a) = app {
                    ai_followups_kicked = true;
                    enqueue_ai_followups(db, a, account_id, &account.email, &ai_background, "early").await;
                }
            }
        }

        // Phase 3: per-email async attachment processing (needs emails already in DB).
        // Only runs when we have an AppHandle (skipped in test context where
        // FakeEmailProvider returns no attachments anyway).
        if let Some(ref a) = app {
            for (email, attachment_infos) in &chunk_emails {
                let should_auto_download =
                    !attachment_infos.is_empty() && auto_download_attachment_categories.contains(&email.category);

                if should_auto_download {
                    if let Err(e) = crate::services::attachments::auto_download_attachments(
                        db,
                        email_provider.as_ref(),
                        email,
                        attachment_infos,
                        app_data_dir,
                        a,
                    )
                    .await
                    {
                        emit_account_log(
                            "error",
                            "attachments",
                            &account.email,
                            &format!("Auto-download attachment error: {}", e),
                        );
                    }
                }

                if !attachment_infos.is_empty() && !attachment_rules.is_empty() {
                    if let Err(e) = crate::services::attachments::process_attachments_for_email(
                        db,
                        email_provider.as_ref(),
                        email,
                        attachment_infos,
                        &attachment_rules,
                        app_data_dir,
                        Some(a),
                    )
                    .await
                    {
                        emit_account_log(
                            "error",
                            "attachments",
                            &account.email,
                            &format!("Attachment rule processing error: {}", e),
                        );
                    }
                }
            }
        }
    }

    // ── Retry previously failed emails ───────────────────────────────────────────
    const MAX_RETRY_COUNT: i32 = 3;

    let (retryable, exhausted): (Vec<_>, Vec<_>) = failed_emails_to_retry
        .into_iter()
        .partition(|(_, retry_count)| *retry_count < MAX_RETRY_COUNT);

    for (id, count) in &exhausted {
        emit_account_log(
            "warn",
            "sync",
            &account.email,
            &format!("Permanently skipping email {} — failed {} times", id, count),
        );
        let _ = db.remove_failed_email(account_id, id);
    }

    if !retryable.is_empty() {
        emit_account_log(
            "info",
            "sync",
            &account.email,
            &format!("Retrying {} previously failed email download(s)...", retryable.len()),
        );

        for chunk in retryable.chunks(BATCH_SIZE) {
            if sync_abort_flags
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(account_id)
                .map(|f| f.load(Ordering::Relaxed))
                .unwrap_or(false)
            {
                sync_abort_flags
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(account_id);
                return Ok(());
            }
            let chunk_ids: Vec<&str> = chunk.iter().map(|(id, _)| id.as_str()).collect();
            match email_provider.batch_get_messages(&chunk_ids).await {
                Ok(retry_results) => {
                    for ((email_id, _), result) in chunk.iter().zip(retry_results) {
                        match result {
                            Ok((mut email, category, attachment_infos)) => {
                                if skip_promotions && category.is_promotions() {
                                    let _ = db.remove_failed_email(account_id, email_id);
                                    continue;
                                }
                                email.account_id = account_id.to_string();
                                match db.insert_email(&email) {
                                    Ok(_) => {
                                        let _ = db.remove_failed_email(account_id, email_id);
                                        synced_count += 1;
                                        all_new_ids.push(email.id.clone());
                                        if let Some(ref a) = app {
                                            if !attachment_infos.is_empty() && !attachment_rules.is_empty() {
                                                if let Err(e) =
                                                    crate::services::attachments::process_attachments_for_email(
                                                        db,
                                                        email_provider.as_ref(),
                                                        &email,
                                                        &attachment_infos,
                                                        &attachment_rules,
                                                        app_data_dir,
                                                        Some(a),
                                                    )
                                                    .await
                                                {
                                                    emit_account_log(
                                                        "error",
                                                        "attachments",
                                                        &account.email,
                                                        &format!("Attachment error on retry: {}", e),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let err_str = e.to_string();
                                        let _ = db.increment_failed_email_retry(account_id, email_id, &err_str);
                                        emit_account_log(
                                            "warn",
                                            "sync",
                                            &account.email,
                                            &format!("Retry insert failed for email {}: {}", email_id, err_str),
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                let err_str = e.to_string();
                                let _ = db.increment_failed_email_retry(account_id, email_id, &err_str);
                                emit_account_log(
                                    "warn",
                                    "sync",
                                    &account.email,
                                    &format!("Retry download failed for email {}: {}", email_id, err_str),
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    emit_account_log(
                        "warn",
                        "sync",
                        &account.email,
                        &format!(
                            "Failed to retry {} emails (will try again next sync): {}",
                            chunk.len(),
                            e
                        ),
                    );
                }
            }
        }
    }

    emit_progress(
        account_id,
        "complete",
        new_count,
        new_count,
        &format!("Synced {} new emails", synced_count),
    );

    db.upsert_sync_status(account_id, "idle", Some(chrono::Utc::now().timestamp()), None)?;
    emit_account_log(
        "success",
        "sync",
        &account.email,
        &format!("Synced {} new emails", synced_count),
    );

    // Checkpoint the WAL after a large write batch so the WAL file doesn't grow
    // unboundedly across syncs.
    {
        let conn = db.connection();
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)");
    }

    if synced_count > 0 {
        match crate::services::email_company::backfill_account(db, app.as_ref(), account_id) {
            Ok(n) if n > 0 => emit_account_log(
                "debug",
                "company",
                &account.email,
                &format!("Tagged {n} new emails with company"),
            ),
            Ok(_) => {}
            Err(e) => emit_account_log(
                "warn",
                "company",
                &account.email,
                &format!("Company tagging failed (non-fatal): {e}"),
            ),
        }

        match crate::services::tag_priority::backfill_account(db, account_id) {
            Ok(n) if n > 0 => emit_account_log(
                "debug",
                "priority",
                &account.email,
                &format!("Backfilled priorities for {n} tags"),
            ),
            Ok(_) => {}
            Err(e) => emit_account_log(
                "warn",
                "priority",
                &account.email,
                &format!("Priority backfill failed (non-fatal): {e}"),
            ),
        }

        match crate::services::tag_priority::update_from_new_emails(db, account_id, &account.email, &all_new_ids) {
            Ok(n) => emit_account_log(
                "debug",
                "priority",
                &account.email,
                &format!("Updated priorities for {n} tags"),
            ),
            Err(e) => emit_account_log(
                "warn",
                "priority",
                &account.email,
                &format!("Priority update failed (non-fatal): {e}"),
            ),
        }
    }

    // ── Secondary mailboxes: Sent / Spam / Trash ──────────────────────────────
    if let Err(e) = sync_extra_mailboxes(db, account, account_id, email_provider.as_ref()).await {
        emit_account_log(
            "warn",
            "sync",
            &account.email,
            &format!("Extra mailbox sync failed (non-fatal): {}", e),
        );
    }
    pull_drafts_if_supported(db, account, account_id, email_provider.as_ref()).await;

    // Classify, extract memory, and generate embeddings on a final pass.
    if let Some(ref a) = app {
        enqueue_ai_followups(db, a, account_id, &account.email, &ai_background, "final").await;
        // Lens incremental hook — extract any newly synced email that matches
        // an enabled Lens's scope. Runs in the background AI queue so it never
        // blocks sync, and only when new emails were actually inserted.
        enqueue_lens_incremental(db, a, account_id, &account.email, &ai_background, &all_new_ids).await;
    }

    if synced_count > 0 {
        if let Err(e) = db.checkpoint_wal_truncate() {
            emit_account_log(
                "debug",
                "sync",
                &account.email,
                &format!("WAL checkpoint after sync failed: {e}"),
            );
        }
    }

    Ok(())
}

/// Enqueue the AI follow-up tasks (classification, memory extraction +
/// embedding + consolidation, search embeddings) onto `ai_background`.
async fn enqueue_ai_followups(
    db: &Arc<Database>,
    app: &AppHandle,
    account_id: &str,
    account_email: &str,
    ai_background: &crate::services::task_queue::TaskQueue,
    label_suffix: &str,
) {
    // Classification.
    {
        let db_classify = Arc::clone(db);
        let aid_classify = account_id.to_string();
        let email_classify = account_email.to_string();
        let task_label = format!("classify:new_emails:{}:{}", aid_classify, label_suffix);
        ai_background
            .submit_named(&task_label, async move {
                if let Err(e) = crate::services::classification::classify_new_emails(&db_classify, &aid_classify).await
                {
                    emit_account_log(
                        "error",
                        "classification",
                        &email_classify,
                        &format!("Classification failed: {}", e),
                    );
                }
            })
            .await;
    }

    // Memory facts: extraction + embedding + consolidation.
    {
        let memory_cfg = crate::services::memory::config::get_config(db).ok();
        let mem_runs = memory_cfg
            .as_ref()
            .map(|c| c.enabled && c.extract_on_sync)
            .unwrap_or(false);
        let consolidation_enabled = memory_cfg.as_ref().map(|c| c.enabled).unwrap_or(false);

        if mem_runs || consolidation_enabled {
            let db_mem = Arc::clone(db);
            let app_mem = app.clone();
            let aid_mem = account_id.to_string();
            let email_mem = account_email.to_string();
            let task_kind = match (mem_runs, consolidation_enabled) {
                (true, true) => "memory:extract+consolidate",
                (true, false) => "memory:extract",
                (false, true) => "memory:consolidate",
                (false, false) => unreachable!("guarded above"),
            };
            let task_label = format!("{}:{}:{}", task_kind, aid_mem, label_suffix);
            ai_background
                .submit_named(&task_label, async move {
                    let memory_cfg_now = crate::services::memory::config::get_config(&db_mem).ok();
                    let mem_runs_now = memory_cfg_now
                        .as_ref()
                        .map(|c| c.enabled && c.extract_on_sync)
                        .unwrap_or(false);
                    let consolidation_enabled_now = memory_cfg_now.as_ref().map(|c| c.enabled).unwrap_or(false);

                    if !mem_runs_now && !consolidation_enabled_now {
                        return;
                    }

                    if mem_runs_now {
                        if let Err(e) =
                            crate::services::memory::extractor::extract_new_emails(&db_mem, &app_mem, &aid_mem).await
                        {
                            emit_account_log(
                                "error",
                                "memory",
                                &email_mem,
                                &format!("Memory extraction failed: {}", e),
                            );
                        }
                        if let Err(e) =
                            crate::services::memory::embeddings::embed_pending_facts(&db_mem, &app_mem, &aid_mem).await
                        {
                            emit_account_log("error", "memory", &email_mem, &format!("Fact embedding failed: {}", e));
                        }
                    }
                    if consolidation_enabled_now {
                        if let Err(e) =
                            crate::services::memory::consolidation::run_consolidation(&db_mem, Some(&app_mem), &aid_mem)
                        {
                            emit_account_log("error", "memory", &email_mem, &format!("Consolidation failed: {}", e));
                        }
                    }
                })
                .await;
        }
    }

    // Tasks: extract pending actions independently from memory facts.
    {
        let task_cfg = crate::services::tasks::config::get_config(db).ok();
        let task_runs = task_cfg
            .as_ref()
            .map(|c| c.enabled && c.extract_on_sync)
            .unwrap_or(false);

        if task_runs {
            let db_tasks = Arc::clone(db);
            let app_tasks = app.clone();
            let aid_tasks = account_id.to_string();
            let email_tasks = account_email.to_string();
            let task_label = format!("tasks:extract:{}:{}", aid_tasks, label_suffix);
            ai_background
                .submit_named(&task_label, async move {
                    let task_runs_now = crate::services::tasks::config::get_config(&db_tasks)
                        .ok()
                        .as_ref()
                        .map(|c| c.enabled && c.extract_on_sync)
                        .unwrap_or(false);
                    if !task_runs_now {
                        return;
                    }
                    if let Err(e) =
                        crate::services::tasks::extractor::extract_new_emails(&db_tasks, &app_tasks, &aid_tasks).await
                    {
                        emit_account_log(
                            "error",
                            "tasks",
                            &email_tasks,
                            &format!("Task extraction failed: {}", e),
                        );
                    }
                })
                .await;
        }
    }

    // Search embeddings — drain the entire backlog in 50-row batches.
    {
        let db_clone = Arc::clone(db);
        let account_id_clone = account_id.to_string();
        let app_handle = app.clone();
        let email_embed = account_email.to_string();
        let task_label = format!("embeddings:after_sync:{}:{}", account_id_clone, label_suffix);
        ai_background
            .submit_named(&task_label, async move {
                const BATCH_SIZE: i32 = 50;
                const MAX_BATCHES: u32 = 10_000;
                let mut total: u32 = 0;
                for _ in 0..MAX_BATCHES {
                    match crate::services::embeddings::generate_embeddings(
                        &db_clone,
                        Some(&account_id_clone),
                        Some(app_handle.clone()),
                        BATCH_SIZE,
                        Some(email_embed.as_str()),
                    )
                    .await
                    {
                        Ok(0) => break,
                        Ok(n) => {
                            total += n;
                        }
                        Err(e) => {
                            emit_account_log(
                                "error",
                                "embeddings",
                                &email_embed,
                                &format!("Embedding generation failed after {}: {}", total, e),
                            );
                            return;
                        }
                    }
                }
                // Only report when embeddings were actually generated — a
                // "Generated 0 embeddings" line is noise on an idle sync.
                if total > 0 {
                    emit_account_log(
                        "success",
                        "embeddings",
                        &email_embed,
                        &format!("Generated {} embeddings", total),
                    );
                }
            })
            .await;
    }
}

/// Run Lens incremental extraction for the email IDs that were just synced.
/// Submitted as a single background task so a Lens with N enabled rows can't
/// occupy the AI queue indefinitely if one extraction hangs — it runs after
/// classification / memory / embeddings have already been queued. Builds the
/// AI provider from user preferences (same path as the user-triggered
/// "Run backfill" button) so errors here mirror what the user would see.
async fn enqueue_lens_incremental(
    db: &Arc<Database>,
    app: &AppHandle,
    account_id: &str,
    account_email: &str,
    ai_background: &crate::services::task_queue::TaskQueue,
    new_email_ids: &[String],
) {
    if new_email_ids.is_empty() {
        return;
    }
    let db_clone = Arc::clone(db);
    let app_clone = app.clone();
    let email_for_log = account_email.to_string();
    let ids = new_email_ids.to_vec();
    let task_label = format!("lens:incremental:{}:{}", account_id, ids.len());
    ai_background
        .submit_named(&task_label, async move {
            // Skip cheaply when there are no enabled lenses — avoids spinning
            // up the AI provider just to no-op.
            match db_clone.list_lenses() {
                Ok(ls) if ls.iter().any(|l| l.is_enabled) => {}
                Ok(_) => return,
                Err(e) => {
                    emit_account_log("warn", "lens", &email_for_log, &format!("List lenses failed: {e}"));
                    return;
                }
            }
            let provider = match crate::services::ai::AiService::load_provider(&db_clone) {
                Ok(p) => p,
                Err(e) => {
                    emit_account_log(
                        "warn",
                        "lens",
                        &email_for_log,
                        &format!("AI provider unavailable, skipping lens incremental: {e}"),
                    );
                    return;
                }
            };
            match crate::services::lenses::runner::on_emails_synced(db_clone, provider, &ids, Some(&app_clone)).await {
                Ok(0) => {}
                Ok(n) => emit_account_log(
                    "success",
                    "lens",
                    &email_for_log,
                    &format!("Lens incremental: extracted {n} row(s) from {} new email(s)", ids.len()),
                ),
                Err(e) => emit_account_log(
                    "error",
                    "lens",
                    &email_for_log,
                    &format!("Lens incremental failed: {e}"),
                ),
            }
        })
        .await;
}

// ── Constants shared by extra-mailbox sync paths ─────────────────────────────

const MAX_EXTRA_MAILBOX_EMAILS: u32 = 500;
/// Upper bound on backfill pages walked in a single periodic sync. The
/// pagination is bounded so a stale mailbox with 10k+ history doesn't make
/// one sync take an hour — subsequent syncs resume where this one left off
/// (the per-mailbox backfill watermark moves backward each pass).
const MAX_BACKFILL_PAGES_PER_SYNC: u32 = 10;
const EXTRA_MAILBOX_BATCH_SIZE: usize = 20;
const EXTRA_MAILBOX_INTER_BATCH_DELAY_MS: u64 = 2_000;

fn extra_mailbox_forward_key(account_id: &str, mailbox: ExtraMailbox) -> String {
    format!("extra_mailbox_sync:{}:{}", account_id, mailbox.as_str())
}

fn extra_mailbox_backfill_key(account_id: &str, mailbox: ExtraMailbox) -> String {
    format!("extra_mailbox_backfill:{}:{}", account_id, mailbox.as_str())
}

fn extra_mailbox_backfill_cursor_key(account_id: &str, mailbox: ExtraMailbox) -> String {
    format!("extra_mailbox_backfill_cursor:{}:{}", account_id, mailbox.as_str())
}

/// Outcome of ingesting a batch of message refs for one mailbox pass.
#[derive(Debug, Default)]
struct IngestOutcome {
    /// Newly inserted emails (after dedup against `emails_exist_batch`).
    inserted: u32,
    /// Highest timestamp among inserted emails (used to advance the forward
    /// watermark).
    max_timestamp: i64,
    /// Lowest timestamp among inserted emails (used to advance the backfill
    /// watermark backward).
    min_timestamp: Option<i64>,
}

/// Fetch full messages for the given refs, force their mailbox/account, and
/// insert them in batches. Shared between forward incremental, backfill, and
/// the manual `resync_mailbox_full` recovery path so all three behave
/// identically with respect to mailbox tagging, attachment metadata, and
/// batching.
async fn ingest_mailbox_refs(
    db: &Arc<Database>,
    account_email: &str,
    account_id: &str,
    mailbox_name: &str,
    email_provider: &dyn EmailProvider,
    refs: Vec<crate::sync::provider::MessageRef>,
) -> IngestOutcome {
    if refs.is_empty() {
        return IngestOutcome::default();
    }

    let all_ids: Vec<String> = refs.iter().map(|r| r.id.clone()).collect();
    let existing_ids = match db.emails_exist_batch(&all_ids) {
        Ok(set) => set,
        Err(e) => {
            emit_account_log(
                "warn",
                "sync",
                account_email,
                &format!("Failed to check existing {} messages: {}", mailbox_name, e),
            );
            return IngestOutcome::default();
        }
    };
    let new_refs: Vec<_> = refs.into_iter().filter(|r| !existing_ids.contains(&r.id)).collect();

    if new_refs.is_empty() {
        return IngestOutcome::default();
    }

    emit_account_log(
        "info",
        "sync",
        account_email,
        &format!("Downloading {} new {} email(s)...", new_refs.len(), mailbox_name),
    );

    let mut max_timestamp: i64 = 0;
    let mut min_timestamp: Option<i64> = None;
    let mut inserted: u32 = 0;
    let mut first_batch = true;

    for chunk in new_refs.chunks(EXTRA_MAILBOX_BATCH_SIZE) {
        if !first_batch {
            sleep(Duration::from_millis(EXTRA_MAILBOX_INTER_BATCH_DELAY_MS)).await;
        }
        first_batch = false;

        let ids: Vec<&str> = chunk.iter().map(|r| r.id.as_str()).collect();
        let batch_results = match email_provider.batch_get_messages(&ids).await {
            Ok(r) => r,
            Err(e) => {
                emit_account_log(
                    "warn",
                    "sync",
                    account_email,
                    &format!("Batch fetch failed for {}: {}", mailbox_name, e),
                );
                continue;
            }
        };

        let mut chunk_emails: Vec<(Email, Vec<crate::sync::provider::AttachmentInfo>)> = Vec::new();
        for (msg_ref, result) in chunk.iter().zip(batch_results) {
            match result {
                Ok((mut email, _category, attachment_infos)) => {
                    email.account_id = account_id.to_string();
                    email.mailbox = mailbox_name.to_string();
                    if email.timestamp > max_timestamp {
                        max_timestamp = email.timestamp;
                    }
                    min_timestamp = Some(min_timestamp.map_or(email.timestamp, |m| m.min(email.timestamp)));
                    chunk_emails.push((email, attachment_infos));
                }
                Err(e) => {
                    emit_account_log(
                        "debug",
                        "sync",
                        account_email,
                        &format!("Failed to download {} message {}: {}", mailbox_name, msg_ref.id, e),
                    );
                }
            }
        }

        if chunk_emails.is_empty() {
            continue;
        }

        let emails_only: Vec<Email> = chunk_emails.iter().map(|(e, _)| e.clone()).collect();
        if let Err(e) = db.insert_emails_batch(&emails_only) {
            emit_account_log(
                "warn",
                "sync",
                account_email,
                &format!("Failed to insert {} batch: {}", mailbox_name, e),
            );
            continue;
        }

        let metas: Vec<_> = chunk_emails
            .iter()
            .flat_map(|(email, infos)| {
                infos.iter().map(move |info| {
                    (
                        email.id.clone(),
                        account_id.to_string(),
                        info.attachment_id.clone(),
                        info.filename.clone(),
                        info.mime_type.clone(),
                        info.size,
                        info.inline_data.clone(),
                    )
                })
            })
            .collect();
        if !metas.is_empty() {
            if let Err(e) = db.insert_email_attachment_metas_batch(&metas) {
                emit_account_log(
                    "warn",
                    "sync",
                    account_email,
                    &format!("Failed to insert {} attachment metas: {}", mailbox_name, e),
                );
            }
        }

        inserted += chunk_emails.len() as u32;
    }

    IngestOutcome {
        inserted,
        max_timestamp,
        min_timestamp,
    }
}

/// Sync Sent / Spam / Trash mailboxes for an account.
///
/// Runs two passes per mailbox:
///
/// 1. **Forward incremental** — fetches messages with timestamp > the
///    per-mailbox forward watermark and advances the watermark when new mail
///    is ingested. This is the steady-state pass.
/// 2. **Backward backfill** — guarded by a per-mailbox done flag, walks the
///    mailbox history backward in `MAX_EXTRA_MAILBOX_EMAILS`-sized windows
///    using `before_timestamp`. Bounded by [`MAX_BACKFILL_PAGES_PER_SYNC`] so
///    one sync run never blocks indefinitely on a deep history; subsequent
///    syncs resume from where this one left off.
///
/// The two passes together cover the gap that the inbox sync's single 500-cap
/// query missed (see ExtraMailbox::all doc-comment for the original bug).
async fn sync_extra_mailboxes(
    db: &Arc<Database>,
    account: &Account,
    account_id: &str,
    email_provider: &dyn EmailProvider,
) -> Result<()> {
    for mailbox in ExtraMailbox::all().iter().copied() {
        // ── Forward incremental ──────────────────────────────────────────────
        sync_extra_mailbox_incremental(db, account, account_id, mailbox, email_provider).await;

        // ── Backward backfill (bounded per sync run) ─────────────────────────
        sync_extra_mailbox_backfill(
            db,
            account,
            account_id,
            mailbox,
            email_provider,
            MAX_BACKFILL_PAGES_PER_SYNC,
        )
        .await;
    }

    Ok(())
}

/// Pull the provider's Drafts folder into the local `drafts` table, for
/// providers that support server-side drafts (Gmail/Outlook). Non-fatal: logs
/// and returns on error so a drafts hiccup never fails the overall sync.
async fn pull_drafts_if_supported(
    db: &Arc<Database>,
    account: &Account,
    account_id: &str,
    email_provider: &dyn EmailProvider,
) {
    if !crate::sync::provider::provider_supports_drafts(&account.provider) {
        return;
    }
    match super::compose::pull_provider_drafts(db, account_id, email_provider).await {
        Ok(count) if count > 0 => emit_account_log(
            "debug",
            "sync",
            &account.email,
            &format!(
                "Pulled {count} draft{} from provider",
                if count == 1 { "" } else { "s" }
            ),
        ),
        Ok(_) => {}
        Err(e) => emit_account_log(
            "warn",
            "sync",
            &account.email,
            &format!("Draft pull failed (non-fatal): {e}"),
        ),
    }
}

/// Forward incremental pass for one extra mailbox. Fetches at most
/// [`MAX_EXTRA_MAILBOX_EMAILS`] messages newer than the persisted watermark
/// and advances the watermark to the newest ingested timestamp.
async fn sync_extra_mailbox_incremental(
    db: &Arc<Database>,
    account: &Account,
    account_id: &str,
    mailbox: ExtraMailbox,
    email_provider: &dyn EmailProvider,
) {
    let mailbox_name = mailbox.as_str();
    let pref_key = extra_mailbox_forward_key(account_id, mailbox);
    let after_timestamp = db
        .get_preference(&pref_key)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i64>().ok());

    let refs = match email_provider
        .list_mailbox_messages(mailbox, MAX_EXTRA_MAILBOX_EMAILS, after_timestamp, None)
        .await
    {
        Ok(refs) => refs,
        Err(e) => {
            emit_account_log(
                "warn",
                "sync",
                &account.email,
                &format!("Failed to list {} mailbox: {}", mailbox_name, e),
            );
            return;
        }
    };

    if refs.is_empty() {
        return;
    }

    let outcome = ingest_mailbox_refs(db, &account.email, account_id, mailbox_name, email_provider, refs).await;

    if outcome.inserted > 0 {
        emit_account_log(
            "success",
            "sync",
            &account.email,
            &format!("Synced {} new {} email(s)", outcome.inserted, mailbox_name),
        );

        // Advance the forward watermark to the newest ingested timestamp.
        let new_watermark = match after_timestamp {
            Some(prev) => prev.max(outcome.max_timestamp),
            None => outcome.max_timestamp,
        };
        if new_watermark > 0 {
            if let Err(e) = db.set_preference(&pref_key, &new_watermark.to_string()) {
                emit_account_log(
                    "warn",
                    "sync",
                    &account.email,
                    &format!("Failed to persist {} watermark: {}", mailbox_name, e),
                );
            }
        }
    }
}

/// Backward backfill pass for one extra mailbox.
///
/// Walks the mailbox history in `MAX_EXTRA_MAILBOX_EMAILS`-sized windows using
/// `before_timestamp = cursor`. The cursor starts at `now()` on first run
/// (or the last persisted position) and moves strictly backward each
/// iteration. This is intentionally NOT anchored at the oldest DB row —
/// real-user gaps live INSIDE the stored history (e.g. the 2024-10 → 2025-12
/// Sent gap where 2018+2026 sent emails are present but 2025 is missing).
/// Anchoring at the oldest row would skip the entire interior gap.
///
/// Each iteration:
/// - Fetch up to `MAX_EXTRA_MAILBOX_EMAILS` refs with `before_timestamp = cursor`.
/// - Ingest (`emails_exist_batch` dedupes against existing DB rows so already-
///   known messages are simply re-confirmed, not re-downloaded).
/// - Advance cursor to the minimum timestamp known for any returned ref
///   (inserted in this pass, or already in DB) so each iteration moves
///   strictly back.
/// - Persist cursor every iteration so partial progress survives crashes.
///
/// Done conditions (sets `extra_mailbox_backfill` to "1"):
/// - Provider returns an empty page — reached the start of mailbox history.
/// - Cursor would fall to/below `sync_from_floor` — user does not want older.
/// - Cannot advance the cursor (defensive — should not happen with the
///   `get_min_timestamp_for_ids` fallback below).
///
/// `max_pages` caps how many iterations run in a single call so that periodic
/// sync never stalls on a deep history. The manual `resync_mailbox_full`
/// recovery path passes `u32::MAX` to walk to exhaustion.
async fn sync_extra_mailbox_backfill(
    db: &Arc<Database>,
    account: &Account,
    account_id: &str,
    mailbox: ExtraMailbox,
    email_provider: &dyn EmailProvider,
    max_pages: u32,
) {
    let mailbox_name = mailbox.as_str();
    let done_key = extra_mailbox_backfill_key(account_id, mailbox);
    let cursor_key = extra_mailbox_backfill_cursor_key(account_id, mailbox);

    if matches!(db.get_preference(&done_key).ok().flatten().as_deref(), Some("1")) {
        return;
    }

    // Cursor starts at the persisted position, or `now()` for a fresh run.
    // We deliberately do NOT anchor at the oldest DB timestamp — interior
    // gaps would be skipped.
    let mut before_timestamp = db
        .get_preference(&cursor_key)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or_else(|| chrono::Utc::now().timestamp());

    let sync_from_floor = account.sync_from_timestamp.unwrap_or(0);
    let mut total_inserted: u32 = 0;

    for _ in 0..max_pages {
        if before_timestamp <= sync_from_floor {
            // Past the configured account sync floor — nothing older to fetch.
            let _ = db.set_preference(&done_key, "1");
            break;
        }

        let refs = match email_provider
            .list_mailbox_messages(mailbox, MAX_EXTRA_MAILBOX_EMAILS, None, Some(before_timestamp))
            .await
        {
            Ok(refs) => refs,
            Err(e) => {
                emit_account_log(
                    "warn",
                    "sync",
                    &account.email,
                    &format!("Backfill of {} failed: {}", mailbox_name, e),
                );
                return;
            }
        };

        if refs.is_empty() {
            // Reached the start of the mailbox history.
            let _ = db.set_preference(&done_key, "1");
            break;
        }

        let ref_ids: Vec<String> = refs.iter().map(|r| r.id.clone()).collect();
        let outcome = ingest_mailbox_refs(db, &account.email, account_id, mailbox_name, email_provider, refs).await;
        total_inserted += outcome.inserted;

        // Determine how far the window actually reached so we can advance
        // strictly backward. Prefer the freshly-inserted min timestamp.
        // If the page was all duplicates, look up min(timestamp) for the
        // returned IDs in the DB — that's how far back the provider's
        // window stretched, regardless of whether anything new came of it.
        let candidate_min = match outcome.min_timestamp {
            Some(min_ts) => Some(min_ts),
            None => db.get_min_timestamp_for_ids(&ref_ids).ok().flatten(),
        };

        match candidate_min {
            Some(min_ts) if min_ts < before_timestamp => {
                before_timestamp = min_ts;
                if let Err(e) = db.set_preference(&cursor_key, &before_timestamp.to_string()) {
                    emit_account_log(
                        "debug",
                        "sync",
                        &account.email,
                        &format!("Failed to persist {} backfill cursor: {}", mailbox_name, e),
                    );
                }
            }
            _ => {
                // Cannot advance with the same cursor — provider returned
                // a non-empty page where every ref is either unknown
                // (download failed) or at/above the cursor. Mark done so
                // we don't infinite-loop. This is defensive; normal
                // operation should always have a candidate_min < cursor.
                let _ = db.set_preference(&done_key, "1");
                break;
            }
        }
    }

    if total_inserted > 0 {
        emit_account_log(
            "success",
            "sync",
            &account.email,
            &format!("Backfilled {} older {} email(s)", total_inserted, mailbox_name),
        );
    }
}

/// Manually triggered full re-scan of one mailbox.
///
/// Clears the per-mailbox backfill done flag and walks the entire history to
/// exhaustion, deduplicating against existing rows. Use this to recover from
/// gaps left by older sync versions that never had a dedicated Sent pass
/// (e.g. the 2024-10 → 2025-12 gap on real user data). Returns the number of
/// newly inserted emails.
pub async fn resync_mailbox_full(
    db: &Arc<Database>,
    account: &Account,
    mailbox: ExtraMailbox,
    email_provider: &dyn EmailProvider,
) -> Result<u32> {
    let account_id = &account.id;
    let mailbox_name = mailbox.as_str();
    let done_key = extra_mailbox_backfill_key(account_id, mailbox);
    let cursor_key = extra_mailbox_backfill_cursor_key(account_id, mailbox);

    // Reset the done flag and the cursor so the backfill walks the full
    // history from "now" downward, regardless of any prior state — that's
    // the whole point of the recovery path.
    if let Err(e) = db.set_preference(&done_key, "0") {
        emit_account_log(
            "warn",
            "sync",
            &account.email,
            &format!("Failed to reset {} backfill flag: {}", mailbox_name, e),
        );
    }
    if let Err(e) = db.set_preference(&cursor_key, &chrono::Utc::now().timestamp().to_string()) {
        emit_account_log(
            "warn",
            "sync",
            &account.email,
            &format!("Failed to reset {} backfill cursor: {}", mailbox_name, e),
        );
    }

    // Snapshot DB size before BOTH passes so the reported delta reflects
    // every email recovered by the manual rescan (forward + backfill).
    let before_count = db.count_emails_in_mailbox(account_id, mailbox_name).unwrap_or(0);

    // Forward pass first to catch anything since the last successful sync.
    sync_extra_mailbox_incremental(db, account, account_id, mailbox, email_provider).await;

    sync_extra_mailbox_backfill(db, account, account_id, mailbox, email_provider, u32::MAX).await;

    let after_count = db
        .count_emails_in_mailbox(account_id, mailbox_name)
        .unwrap_or(before_count);
    Ok(after_count.saturating_sub(before_count) as u32)
}

// ── Sync-pass planner ────────────────────────────────────────────────────────

/// Which list-messages passes the main inbox sync should run, derived from
/// the account's `sync_from_timestamp` and what's already in the local DB.
/// See [`plan_sync_passes`] for the decision logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SyncPlan {
    /// Lower bound (inclusive) for the uncapped backfill pass. `None` means
    /// the backfill loop is skipped entirely.
    pub backfill_after_timestamp: Option<i64>,
    /// Upper bound for the backfill pass — the oldest email already in the
    /// local DB. `None` on the very first sync, which makes the backfill
    /// fetch everything from `backfill_after_timestamp` up to "now".
    pub backfill_before_timestamp: Option<i64>,
    /// Whether to run the page-capped incremental pass (the
    /// `MAX_INCREMENTAL_EMAILS_PER_SYNC` loop).
    pub run_incremental: bool,
}

/// Pure decision: given the user's chosen `sync_from_timestamp` and the
/// timestamps already present locally, decide which passes to run.
///
/// The key invariant we enforce here is: **on the very first sync of an
/// account, the per-sync 500-email cap on the incremental loop must NOT
/// truncate the user's chosen history range**. The old logic ran ONLY the
/// capped incremental loop on first sync (because the backfill condition
/// required an existing `oldest_timestamp` that doesn't exist yet), so a
/// user who picked "sync since 2004" got back the most recent 500 emails
/// and nothing else. The fix promotes first sync to the uncapped backfill
/// loop whenever the user gave us a `sync_from` to honor, and skips
/// incremental on that first pass so we don't double-fetch the same window.
///
/// `effective_sync_from` is the caller's already-resolved
/// `account.sync_from_timestamp.or(provider_default_zero)`. We treat both
/// the explicit user choice and the provider default the same way: any
/// value asks us to make sure that range is covered in full on first sync.
pub(super) fn plan_sync_passes(
    effective_sync_from: Option<i64>,
    latest_timestamp: Option<i64>,
    oldest_timestamp: Option<i64>,
) -> SyncPlan {
    let backfill_after_timestamp = match (effective_sync_from, oldest_timestamp) {
        // History gap: user wants emails older than what's locally stored.
        // The backfill loop pulls them between sync_from (inclusive) and the
        // current oldest (exclusive).
        (Some(sync_from), Some(oldest)) if sync_from < oldest => Some(sync_from),
        // First sync ever, with a sync_from set: pull everything from
        // sync_from to "now" (no upper bound). This is the case that was
        // silently capped at 500 by the old code.
        (Some(sync_from), None) => Some(sync_from),
        _ => None,
    };
    let backfill_before_timestamp = oldest_timestamp;
    // Skip incremental on the very first sync — the backfill above already
    // covers the full range without the per-sync cap. Once at least one
    // email has landed (latest_timestamp.is_some()), incremental resumes
    // its normal job of pulling "what's new since last sync".
    let run_incremental = latest_timestamp.is_some();
    SyncPlan {
        backfill_after_timestamp,
        backfill_before_timestamp,
        run_incremental,
    }
}

#[cfg(test)]
mod plan_sync_passes_tests {
    use super::*;

    #[test]
    fn first_sync_with_explicit_sync_from_uses_uncapped_backfill_and_skips_incremental() {
        // Reproduces the production bug: user picked "Sync since 2004" on a
        // brand-new Outlook account. Old code ran the 500-capped incremental
        // pass and stopped, leaving thousands of older messages unsynced.
        let plan = plan_sync_passes(Some(1_088_640_000), None, None);
        assert_eq!(
            plan,
            SyncPlan {
                backfill_after_timestamp: Some(1_088_640_000),
                backfill_before_timestamp: None,
                run_incremental: false,
            }
        );
    }

    #[test]
    fn first_sync_without_any_sync_from_runs_neither_pass() {
        // No user date AND no provider default → nothing to anchor on; do
        // nothing rather than backfill from epoch. Currently no provider in
        // the catalog hits this branch (gmail/outlook/imap all pass Some(0)),
        // but the case is reachable for future providers.
        let plan = plan_sync_passes(None, None, None);
        assert_eq!(plan.backfill_after_timestamp, None);
        assert!(!plan.run_incremental);
    }

    #[test]
    fn subsequent_sync_with_no_history_gap_runs_incremental_only() {
        // Account has been synced before AND sync_from is newer than the
        // oldest local email — i.e. nothing older to backfill.
        let plan = plan_sync_passes(Some(3_000), Some(5_000), Some(2_500));
        assert_eq!(plan.backfill_after_timestamp, None);
        assert_eq!(plan.backfill_before_timestamp, Some(2_500));
        assert!(plan.run_incremental);
    }

    #[test]
    fn subsequent_sync_with_history_gap_runs_backfill_and_incremental() {
        // Classic "user lowered the sync_from date after the initial sync"
        // scenario: local oldest is 2024, but they now want emails since
        // 2004. Backfill fills the gap; incremental keeps up with what's new.
        let plan = plan_sync_passes(Some(1_088_640_000), Some(1_716_000_000), Some(1_700_000_000));
        assert_eq!(plan.backfill_after_timestamp, Some(1_088_640_000));
        assert_eq!(plan.backfill_before_timestamp, Some(1_700_000_000));
        assert!(plan.run_incremental);
    }

    #[test]
    fn subsequent_sync_with_sync_from_equal_to_oldest_skips_backfill() {
        // Boundary: sync_from == oldest_timestamp. Nothing older exists by
        // definition, so skip the backfill loop.
        let plan = plan_sync_passes(Some(1_000), Some(2_000), Some(1_000));
        assert_eq!(plan.backfill_after_timestamp, None);
        assert!(plan.run_incremental);
    }

    #[test]
    fn subsequent_sync_without_sync_from_runs_incremental_only() {
        // sync_from never set (None) and account has history → just keep
        // pulling new mail; no backfill intent to honor.
        let plan = plan_sync_passes(None, Some(2_000), Some(1_000));
        assert_eq!(plan.backfill_after_timestamp, None);
        assert!(plan.run_incremental);
    }

    #[test]
    fn first_sync_skips_incremental_so_it_cannot_truncate_the_backfill_range() {
        // The behaviour the production bug really hinged on: even with the
        // 500-cap loop wired in elsewhere, this planner must not authorize
        // it on first sync. If this test fails, the cap will silently
        // truncate long-history backfills again.
        let plan = plan_sync_passes(Some(1_088_640_000), None, None);
        assert!(
            !plan.run_incremental,
            "first sync must skip incremental; the 500-cap would truncate the backfill"
        );
    }
}
