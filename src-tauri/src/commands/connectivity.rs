use tauri::State;

use crate::models::error::AppError;
use crate::AppState;

/// Last cached probe result from `ConnectivityMonitor`. The probe runs every
/// 15s in the background; the frontend uses this to seed its initial state
/// (and as a defensive fallback if it ever misses an `app-connectivity-changed`
/// event).
#[tauri::command]
pub async fn is_online(state: State<'_, AppState>) -> Result<bool, AppError> {
    Ok(state.connectivity.is_online())
}
