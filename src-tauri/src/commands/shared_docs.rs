//! Shared documents and sheets. Thin wrappers over `services::shared_docs`;
//! every command names its account and is refused for a document of another
//! one. Document content crosses this boundary as base64 Yjs bytes.

use tauri::{AppHandle, State};

use crate::models::error::AppError;
use crate::models::shared_docs::{DocKind, SharedDoc};
use crate::models::Account;
use crate::services::emails::build_provider;
use crate::services::shared_docs;
use crate::AppState;

fn account(state: &State<'_, AppState>, account_id: &str) -> Result<Account, AppError> {
    state
        .db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {account_id} not found")))
}

fn require_enabled(state: &State<'_, AppState>) -> Result<(), AppError> {
    if shared_docs::is_enabled(&state.db) {
        Ok(())
    } else {
        Err(AppError::InvalidInput(
            "Shared documents are turned off in Settings".into(),
        ))
    }
}

/// The documents of one account, or of every account when `None`.
#[tauri::command]
pub async fn list_shared_docs(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<SharedDoc>, AppError> {
    state.db.list_shared_docs(account_id.as_deref())
}

#[tauri::command]
pub async fn create_shared_doc(
    state: State<'_, AppState>,
    account_id: String,
    kind: DocKind,
    title: String,
) -> Result<SharedDoc, AppError> {
    require_enabled(&state)?;
    let account = account(&state, &account_id)?;
    shared_docs::create(&state.db, &account, kind, &title, crate::services::clock::now_secs())
}

/// The whole document, base64 Yjs v1 update.
#[tauri::command]
pub async fn get_shared_doc_state(
    state: State<'_, AppState>,
    account_id: String,
    doc_id: String,
) -> Result<String, AppError> {
    shared_docs::state(&state.db, &account_id, &doc_id)
}

/// What an editor at `state_vector` (base64) lacks, base64.
#[tauri::command]
pub async fn get_shared_doc_diff(
    state: State<'_, AppState>,
    account_id: String,
    doc_id: String,
    state_vector: String,
) -> Result<String, AppError> {
    shared_docs::diff_since(&state.db, &account_id, &doc_id, &state_vector)
}

/// Save an edit made in the editor (base64 Yjs update).
#[tauri::command]
pub async fn apply_shared_doc_update(
    state: State<'_, AppState>,
    account_id: String,
    doc_id: String,
    update: String,
) -> Result<(), AppError> {
    shared_docs::apply_local_update(
        &state.db,
        &account_id,
        &doc_id,
        &update,
        crate::services::clock::now_secs(),
    )
}

/// Share with `recipients` and mail them the invitation. Only called from the
/// share dialog, after the user confirmed that this document's changes will be
/// mailed to those addresses automatically.
#[tauri::command]
pub async fn share_shared_doc(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: String,
    doc_id: String,
    recipients: Vec<String>,
    snapshot_html: Option<String>,
) -> Result<SharedDoc, AppError> {
    require_enabled(&state)?;
    let account = account(&state, &account_id)?;
    let provider = build_provider(&account, Some(app)).await?;
    let result = shared_docs::share(
        &state.db,
        &account,
        provider.as_ref(),
        &doc_id,
        &recipients,
        snapshot_html.as_deref(),
        crate::services::clock::now_secs(),
    )
    .await;
    if let Err(e) = &result {
        crate::services::logger::log(
            "error",
            "sync",
            format!("[{}] The document could not be shared: {e}", account.email),
        );
    }
    result
}

#[tauri::command]
pub async fn accept_shared_doc(
    state: State<'_, AppState>,
    account_id: String,
    doc_id: String,
) -> Result<SharedDoc, AppError> {
    require_enabled(&state)?;
    shared_docs::accept(&state.db, &account_id, &doc_id, crate::services::clock::now_secs())
}

/// Decline an invitation or leave a document.
#[tauri::command]
pub async fn leave_shared_doc(
    state: State<'_, AppState>,
    account_id: String,
    doc_id: String,
) -> Result<SharedDoc, AppError> {
    shared_docs::leave(&state.db, &account_id, &doc_id, crate::services::clock::now_secs())
}

/// Mail the document's pending changes now ("Send now", closing the editor)
/// instead of waiting for the pause. Returns whether a message went out.
#[tauri::command]
pub async fn flush_shared_doc(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: String,
    doc_id: String,
) -> Result<bool, AppError> {
    require_enabled(&state)?;
    let account = account(&state, &account_id)?;
    crate::services::ownership::shared_doc_in_account(&state.db, &account_id, &doc_id)?;
    let provider = build_provider(&account, Some(app)).await?;
    shared_docs::flush(&state.db, &account, provider.as_ref(), &doc_id).await
}
