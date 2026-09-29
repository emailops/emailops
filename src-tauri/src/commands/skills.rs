//! Tauri commands backing Settings → Skills and the Skills view. Thin wrappers around
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

/// Switch one skill on or off for the chat (Skills view toggle).
#[tauri::command]
pub async fn set_skill_enabled(state: State<'_, AppState>, name: String, enabled: bool) -> Result<(), AppError> {
    skills::set_skill_enabled(&state.db, &name, enabled)
}

/// The raw `SKILL.md` of one skill, for the editor.
#[tauri::command]
pub async fn read_skill(state: State<'_, AppState>, name: String) -> Result<String, AppError> {
    skills::read_skill_source(&state.db, &name)
}

/// Save an edited `SKILL.md` and return the skill's name afterwards (the file
/// wins on the name, so a save can rename it). Rejected, with nothing written,
/// if the text would not load or the file changed on disk since `base` was read.
#[tauri::command]
pub async fn save_skill(
    state: State<'_, AppState>,
    name: String,
    content: String,
    base: String,
) -> Result<String, AppError> {
    skills::save_skill_source(&state.db, &name, &content, &base)
}

/// Create a new skill from the template.
#[tauri::command]
pub async fn create_skill(state: State<'_, AppState>, name: String) -> Result<(), AppError> {
    skills::create_skill(&state.db, &name)
}
