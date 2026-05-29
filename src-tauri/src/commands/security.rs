use tauri::State;

use crate::models::error::{AppError, Result};
use crate::services::password;
use crate::AppState;

const PREF_KEY: &str = "security.main_password_hash";

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
    if new_password.is_empty() {
        return Err(AppError::InvalidInput("Password cannot be empty.".into()));
    }

    let existing = state.db.get_preference(PREF_KEY)?.filter(|v| !v.is_empty());

    if let Some(stored) = existing {
        let current = current_password
            .ok_or_else(|| AppError::InvalidInput("Current password is required to change it.".into()))?;
        if !password::verify_password(&current, &stored)? {
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
    let stored = state.db.get_preference(PREF_KEY)?.filter(|v| !v.is_empty());
    let Some(stored_hash) = stored else {
        return Ok(false);
    };

    if !password::verify_password(&password, &stored_hash)? {
        return Ok(false);
    }

    // Transparently upgrade legacy SHA-256 hashes to Argon2 on successful verify.
    if password::needs_rehash(&stored_hash) {
        let new_hash = password::hash_password(&password)?;
        state.db.set_preference(PREF_KEY, &new_hash)?;
    }

    Ok(true)
}

/// Remove the main password. Requires the current password to confirm.
#[tauri::command]
pub async fn remove_main_password(state: State<'_, AppState>, password: String) -> Result<()> {
    let stored = state
        .db
        .get_preference(PREF_KEY)?
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::InvalidInput("No main password is currently set.".into()))?;

    if !password::verify_password(&password, &stored)? {
        return Err(AppError::AuthError("Password is incorrect.".into()));
    }

    state.db.set_preference(PREF_KEY, "")?;
    Ok(())
}
