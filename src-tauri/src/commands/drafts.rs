use tauri::State;

use crate::models::error::AppError;
use crate::models::{Draft, SaveDraftRequest};
use crate::AppState;

#[tauri::command]
pub async fn list_drafts(state: State<'_, AppState>, account_id: String) -> Result<Vec<Draft>, AppError> {
    state.db.list_drafts(&account_id)
}

#[tauri::command]
pub async fn save_draft(state: State<'_, AppState>, req: SaveDraftRequest) -> Result<Draft, AppError> {
    state.db.save_draft(&req)
}

#[tauri::command]
pub async fn delete_draft(state: State<'_, AppState>, draft_id: String, account_id: String) -> Result<(), AppError> {
    state.db.delete_draft(&draft_id, &account_id)
}
