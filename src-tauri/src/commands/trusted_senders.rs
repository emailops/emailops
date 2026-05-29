use tauri::State;

use crate::models::error::AppError;
use crate::AppState;

#[tauri::command]
pub async fn add_trusted_sender(
    state: State<'_, AppState>,
    account_id: String,
    sender_email: String,
) -> Result<(), AppError> {
    state.db.add_trusted_sender(&account_id, &sender_email)
}

#[tauri::command]
pub async fn remove_trusted_sender(
    state: State<'_, AppState>,
    account_id: String,
    sender_email: String,
) -> Result<(), AppError> {
    state.db.remove_trusted_sender(&account_id, &sender_email)
}

#[tauri::command]
pub async fn list_trusted_senders(state: State<'_, AppState>, account_id: String) -> Result<Vec<String>, AppError> {
    state.db.list_trusted_senders(&account_id)
}

#[tauri::command]
pub async fn is_sender_trusted(
    state: State<'_, AppState>,
    account_id: String,
    sender_email: String,
) -> Result<bool, AppError> {
    state.db.is_sender_trusted(&account_id, &sender_email)
}
