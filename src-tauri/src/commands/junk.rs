//! Tauri commands for junk detection. Thin wrappers over `services::junk`.

use tauri::State;

use crate::db::emails::junk::StoredJunkVerdict;
use crate::models::error::AppError;
use crate::services::junk;
use crate::AppState;

/// Verdicts for a batch of emails, keyed by email id.
///
/// Missing keys mean "not scored yet" — the UI must render nothing rather than
/// a clean badge, because absence of a verdict is not a verdict of innocence.
#[tauri::command]
pub async fn get_junk_verdicts(
    state: State<'_, AppState>,
    email_ids: Vec<String>,
) -> Result<std::collections::HashMap<String, StoredJunkVerdict>, AppError> {
    state.db.get_junk_verdicts_batch(&email_ids)
}

/// Record the user's correction.
///
/// `is_junk = false` is permanent: it survives re-scoring, model-version bumps
/// and backfills. One re-flagged legitimate message destroys trust in the whole
/// feature.
#[tauri::command]
pub async fn set_junk_feedback(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
    is_junk: bool,
) -> Result<(), AppError> {
    junk::set_feedback(&state.db, &account_id, &email_id, is_junk).await?;
    crate::services::logger::log(
        "info",
        "system",
        if is_junk {
            "Marked message as junk"
        } else {
            "Marked message as not junk"
        },
    );
    Ok(())
}

/// Score already-synced mail that predates the feature.
///
/// Returns immediately; the work runs on the background queue and reports
/// through `app-log`, because a large mailbox takes long enough that awaiting it
/// in the webview would freeze the UI.
#[tauri::command]
pub async fn backfill_junk_scores(state: State<'_, AppState>, account_id: String) -> Result<(), AppError> {
    let db = std::sync::Arc::clone(&state.db);
    let label = format!("junk:backfill:{account_id}");
    state
        .ai_background
        .submit_named(&label, async move {
            match junk::backfill_account(&db, &account_id).await {
                Ok(n) => {
                    crate::services::logger::log("success", "system", format!("Junk backfill scored {n} message(s)"))
                }
                Err(e) => crate::services::logger::log("error", "system", format!("Junk backfill failed: {e}")),
            }
        })
        .await;
    Ok(())
}

/// Confirm a message really is junk, and file it on the server so the provider's
/// own filter learns from it.
///
/// This does NOT contradict the local-flag-only decision. That decision is about
/// the *detector* never moving mail on its own — an automated false positive
/// hides a message in every client the user owns. A move the user explicitly
/// asked for is the opposite: deliberate, attributable, and undoable from the
/// server's Junk folder.
///
/// The override is recorded either way, so the statistical layer learns from the
/// correction even when the provider cannot be told (Gmail and Graph do not
/// implement the move seam; only IMAP does).
#[tauri::command]
pub async fn report_junk_to_provider(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    account_id: String,
    email_id: String,
) -> Result<bool, AppError> {
    // Record the user's judgement first. It is the more valuable half — it
    // trains the model — and it must survive a failure to reach the server.
    junk::set_feedback(&state.db, &account_id, &email_id, true).await?;

    let spam_folder = state
        .db
        .list_folders(&account_id, Some(crate::models::FolderRole::Spam))?
        .into_iter()
        .next();
    let Some(folder) = spam_folder else {
        crate::services::logger::log(
            "info",
            "system",
            "Marked as junk locally: this account has no server-side Junk folder to file it in",
        );
        return Ok(false);
    };

    let (account, provider) = crate::commands::emails::account_and_provider(&state, app, &account_id).await?;
    let target = format!("folder:{}", folder.server_path);
    crate::services::emails::move_email(&state.db, &account, provider.as_ref(), &email_id, &target).await?;
    crate::services::logger::log(
        "success",
        "system",
        "Reported as junk and filed in the server's Junk folder",
    );
    Ok(true)
}

#[tauri::command]
pub async fn get_junk_config(state: State<'_, AppState>) -> Result<junk::config::JunkConfig, AppError> {
    Ok(junk::config::get_config(&state.db))
}

#[tauri::command]
pub async fn set_junk_config(state: State<'_, AppState>, config: junk::config::JunkConfig) -> Result<(), AppError> {
    junk::config::save_config(&state.db, &config)
}

/// Read-only status so the feature is not a black box: how much has been scored,
/// what it found, and whether the per-account models have enough labels to vote.
#[tauri::command]
pub async fn get_junk_stats(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<junk::config::JunkStats, AppError> {
    junk::config::get_stats(&state.db, &account_id)
}
