//! Tauri commands backing Settings → Skills. Thin wrappers around
//! `services::skills`.

use tauri::State;

use crate::services::skills::{self, SkillsOverview};
use crate::{AppError, AppState};

#[tauri::command]
pub async fn list_skills(state: State<'_, AppState>) -> Result<SkillsOverview, AppError> {
    Ok(skills::overview(&state.db))
}

/// Create the skills folder if it does not exist yet and open it in the OS
/// file manager, so the user can drop a `SKILL.md` folder in.
#[tauri::command]
pub async fn open_skills_folder(state: State<'_, AppState>) -> Result<(), AppError> {
    let dir = skills::ensure_skills_dir(&state.db)?;
    crate::services::attachments::reveal_in_file_manager(&dir)
}
