//! Tauri commands backing the user-editable prompt registry.
//!
//! Thin wrappers around `services::prompts`. The settings panel uses these to
//! list, override, and reset prompts.

use tauri::State;

use crate::services::prompts::{self, PromptInfo};
use crate::{AppError, AppState};

#[tauri::command]
pub async fn list_prompts(state: State<'_, AppState>) -> Result<Vec<PromptInfo>, AppError> {
    prompts::list_prompts(&state.db)
}

#[tauri::command]
pub async fn set_prompt(state: State<'_, AppState>, id: String, template: String) -> Result<(), AppError> {
    prompts::set_template(&state.db, &id, &template)
}

#[tauri::command]
pub async fn reset_prompt(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    prompts::reset_template(&state.db, &id)
}
