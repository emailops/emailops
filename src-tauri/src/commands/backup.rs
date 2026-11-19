//! The backups, as Settings → Privacy & Security shows and changes them.
//!
//! The webview never names a path: every command works on the backup folders
//! inside the data folder, and a restore names a copy those folders list.

use std::time::Duration;

use tauri::{AppHandle, State};

use crate::models::error::{AppError, Result};
use crate::services::backup::{self, BackupFrequency, BackupSettings, BackupStatus};
use crate::AppState;

/// How long a restore waits for sync and AI work to stop before giving up.
const BACKGROUND_STOP_TIMEOUT: Duration = Duration::from_secs(60);

#[tauri::command]
pub async fn get_backups(state: State<'_, AppState>) -> Result<BackupStatus> {
    backup::status(&state.db, &state.app_data_dir)
}

/// Store the backup hour, how often each copy is made and how many full copies
/// are kept, dropping the full copies beyond `keep` now.
#[tauri::command]
pub async fn set_backup_settings(
    state: State<'_, AppState>,
    hour: u32,
    local: BackupFrequency,
    full: BackupFrequency,
    keep: usize,
) -> Result<BackupStatus> {
    backup::save_settings(
        &state.db,
        &state.app_data_dir,
        BackupSettings {
            hour,
            local,
            full,
            keep,
        },
    )?;
    backup::status(&state.db, &state.app_data_dir)
}

#[tauri::command]
pub async fn delete_full_backups(state: State<'_, AppState>) -> Result<BackupStatus> {
    backup::delete_full(&state.db, &state.app_data_dir, chrono::Utc::now())?;
    backup::status(&state.db, &state.app_data_dir)
}

/// Restore the local data copy `name` and restart the app, so every cache and
/// background job starts again from the restored rows.
#[tauri::command]
pub async fn restore_local_backup(app: AppHandle, state: State<'_, AppState>, name: String) -> Result<()> {
    crate::services::logger::log("info", "system", format!("Restoring local data from {name}"));
    // Sync and AI jobs write to the tables a restore replaces.
    backup::pause();
    state.scheduler.stop();
    if !state.stop_background_work(BACKGROUND_STOP_TIMEOUT).await {
        let message = "Restoring local data failed: background work did not stop. Restart EmailOps and try again.";
        crate::services::logger::log("error", "system", message);
        return Err(AppError::IoError(message.into()));
    }
    let (db, data_dir) = (state.db.clone(), state.app_data_dir.clone());
    let restored = tauri::async_runtime::spawn_blocking(move || backup::restore_local(&db, &data_dir, &name))
        .await
        .map_err(|e| AppError::IoError(format!("Restoring local data failed: {e}")))?;
    let rows = match restored {
        Ok(rows) => rows,
        Err(e) => {
            crate::services::logger::log(
                "error",
                "system",
                format!("Restoring local data failed: {e}. Restart EmailOps to resume syncing."),
            );
            return Err(e);
        }
    };
    crate::services::logger::log(
        "success",
        "system",
        format!("Restored {rows} rows of local data; restarting"),
    );
    // Not `app.restart()`: see `move_data_location`.
    crate::util::relaunch::request();
    app.exit(0);
    Ok(())
}
