//! The automatic database backup, as Settings → Privacy shows and changes it.
//!
//! The webview never names a path: every command works on the `backups/`
//! folder inside the data folder.

use tauri::State;

use crate::models::error::Result;
use crate::services::db_backup::{self, BackupFrequency, BackupSettings, BackupStatus};
use crate::AppState;

#[tauri::command]
pub async fn get_db_backups(state: State<'_, AppState>) -> Result<BackupStatus> {
    db_backup::status(&state.db, &db_backup::backup_dir(&state.app_data_dir))
}

/// Store how often the backup runs and how many copies it keeps, dropping the
/// copies beyond `keep` now.
#[tauri::command]
pub async fn set_db_backup_settings(
    state: State<'_, AppState>,
    frequency: BackupFrequency,
    keep: usize,
) -> Result<BackupStatus> {
    let dir = db_backup::backup_dir(&state.app_data_dir);
    db_backup::save_settings(&state.db, &dir, BackupSettings { frequency, keep })?;
    db_backup::status(&state.db, &dir)
}

#[tauri::command]
pub async fn delete_db_backups(state: State<'_, AppState>) -> Result<BackupStatus> {
    let dir = db_backup::backup_dir(&state.app_data_dir);
    db_backup::delete_all(&dir)?;
    db_backup::status(&state.db, &dir)
}
