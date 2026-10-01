use tauri::State;

use crate::models::error::{AppError, Result};
use crate::services::{clock, password};
use crate::AppState;

const PREF_KEY: &str = password::MAIN_PASSWORD_KEY;

#[tauri::command]
pub async fn has_main_password(state: State<'_, AppState>) -> Result<bool> {
    Ok(state
        .db
        .get_preference(PREF_KEY)?
        .map(|v| !v.is_empty())
        .unwrap_or(false))
}

/// Set or change the main password.
/// - If no password is currently set, `current_password` may be `None`.
/// - If one is already set, `current_password` must match it.
#[tauri::command]
pub async fn set_main_password(
    state: State<'_, AppState>,
    current_password: Option<String>,
    new_password: String,
) -> Result<()> {
    password::validate_new_password(&new_password)?;

    let existing = state.db.get_preference(PREF_KEY)?.filter(|v| !v.is_empty());

    if existing.is_some() {
        let current = current_password
            .ok_or_else(|| AppError::InvalidInput("Current password is required to change it.".into()))?;
        if !password::verify_main_password(&state.db, &current, clock::now_secs())? {
            return Err(AppError::AuthError("Current password is incorrect.".into()));
        }
    }

    state
        .db
        .set_preference(PREF_KEY, &password::hash_password(&new_password)?)?;
    Ok(())
}

#[tauri::command]
pub async fn verify_main_password(state: State<'_, AppState>, password: String) -> Result<bool> {
    password::verify_main_password(&state.db, &password, clock::now_secs())
}

/// Remove the main password. Requires the current password to confirm.
#[tauri::command]
pub async fn remove_main_password(state: State<'_, AppState>, password: String) -> Result<()> {
    if state.db.get_preference(PREF_KEY)?.filter(|v| !v.is_empty()).is_none() {
        return Err(AppError::InvalidInput("No main password is currently set.".into()));
    }

    if !password::verify_main_password(&state.db, &password, clock::now_secs())? {
        return Err(AppError::AuthError("Password is incorrect.".into()));
    }

    state.db.set_preference(PREF_KEY, "")?;
    Ok(())
}
