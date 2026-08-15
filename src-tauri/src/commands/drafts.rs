use tauri::{AppHandle, State};

use crate::models::error::AppError;
use crate::models::{Draft, DraftAttachment, SaveDraftRequest};
use crate::services::emails;
use crate::sync::provider::provider_supports_drafts;
use crate::AppState;

#[tauri::command]
pub async fn list_drafts(state: State<'_, AppState>, account_id: String) -> Result<Vec<Draft>, AppError> {
    state.db.list_drafts(&account_id)
}

#[tauri::command]
pub async fn get_draft(state: State<'_, AppState>, draft_id: String) -> Result<Option<Draft>, AppError> {
    state.db.get_draft(&draft_id)
}

/// Pull the provider's Drafts folder on demand, so a draft edited in Gmail
/// shows up when the user opens the Drafts screen or a draft instead of only
/// after the next account sync. Returns how many drafts were re-read.
///
/// Best-effort by design: an unreachable provider leaves the local drafts as
/// they are and reports zero rather than failing the screen the user just
/// opened. Throttled per account inside the service.
#[tauri::command]
pub async fn refresh_drafts(state: State<'_, AppState>, app: AppHandle, account_id: String) -> Result<usize, AppError> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {account_id} not found")))?;
    if !provider_supports_drafts(&account.provider) {
        return Ok(0);
    }
    let provider = match emails::build_provider(&account, Some(app)).await {
        Ok(p) => p,
        Err(e) => {
            crate::services::logger::log(
                "debug",
                "drafts",
                format!("Draft refresh skipped — provider unavailable: {e}"),
            );
            return Ok(0);
        }
    };
    match emails::refresh_provider_drafts(
        &state.db,
        &account_id,
        provider.as_ref(),
        crate::services::clock::now_secs(),
    )
    .await
    {
        Ok(count) => Ok(count),
        Err(e) => {
            crate::services::logger::log("warn", "drafts", format!("Draft refresh failed: {e}"));
            Ok(0)
        }
    }
}

#[tauri::command]
pub async fn list_draft_attachments(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<Vec<DraftAttachment>, AppError> {
    state.db.list_draft_attachments(&draft_id)
}

/// Save (create/upsert) a draft. When the account's provider supports
/// server-side drafts, the draft is also pushed to the Drafts folder. The push
/// is best-effort: if the provider can't be built (e.g. offline), the draft
/// still saves locally and a later sync reconciles it.
#[tauri::command]
pub async fn save_draft(state: State<'_, AppState>, app: AppHandle, req: SaveDraftRequest) -> Result<Draft, AppError> {
    let account = state
        .db
        .get_account(&req.account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", req.account_id)))?;

    let input = emails::ComposeInput {
        draft_id: req.id,
        account_id: req.account_id,
        email_id: req.email_id,
        to: req.to_addresses,
        cc: req.cc_addresses,
        subject: req.subject,
        body: req.body,
        body_html: req.body_html,
        attachments: req.attachments,
    };

    let provider = if provider_supports_drafts(&account.provider) {
        match emails::build_provider(&account, Some(app)).await {
            Ok(p) => Some(p),
            Err(e) => {
                crate::services::logger::log(
                    "debug",
                    "drafts",
                    format!("Saving draft locally only — provider unavailable: {e}"),
                );
                None
            }
        }
    } else {
        None
    };

    emails::compose_draft(&state.db, &account, input, provider.as_deref()).await
}

/// Send a saved draft, then remove it locally and from the provider's Drafts
/// folder. Requires a live provider connection.
#[tauri::command]
pub async fn send_draft(
    state: State<'_, AppState>,
    app: AppHandle,
    draft_id: String,
    account_id: String,
) -> Result<(), AppError> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {account_id} not found")))?;
    let provider = emails::build_provider(&account, Some(app)).await?;
    emails::send_draft(&state.db, &account, &draft_id, provider.as_ref()).await
}

/// Delete a draft locally, and — when it is linked to a provider draft and the
/// provider is reachable — from the provider's Drafts folder too (best-effort).
#[tauri::command]
pub async fn delete_draft(
    state: State<'_, AppState>,
    app: AppHandle,
    draft_id: String,
    account_id: String,
) -> Result<(), AppError> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {account_id} not found")))?;
    let provider = if provider_supports_drafts(&account.provider) {
        emails::build_provider(&account, Some(app)).await.ok()
    } else {
        None
    };
    emails::delete_draft(&state.db, &account, &draft_id, provider.as_deref()).await
}
