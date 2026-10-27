//! Data location: where EmailOps keeps its data folder, and moving it.
//!
//! The webview never names a path. `pick_data_folder` opens the native dialog,
//! checks the folder and remembers it; `move_data_location` moves the data
//! either there or back to the default, and restarts the app. A compromised
//! webview therefore cannot copy the mailbox to a folder of its choosing.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, State};

use crate::models::error::{AppError, Result};
use crate::services::data_move;
use crate::util::data_location::DataDirSource;
use crate::AppState;

/// Set once at startup: how the data folder was chosen, the platform default,
/// and the folder the user last picked, waiting for confirmation.
pub struct DataLocationState {
    pub source: DataDirSource,
    pub default_dir: PathBuf,
    picked: Mutex<Option<PathBuf>>,
}

impl DataLocationState {
    pub fn new(source: DataDirSource, default_dir: PathBuf) -> Self {
        Self {
            source,
            default_dir,
            picked: Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataLocationInfo {
    /// False on iOS/Android, whose sandbox decides where app data lives.
    pub supported: bool,
    pub path: String,
    pub default_path: String,
    pub is_default: bool,
    /// `EMAILOPS_DATA_DIR` decides the folder; it cannot be moved from the app.
    pub set_by_env: bool,
    /// The folder the data was moved from, until the user dismisses the note.
    pub previous_path: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedDataFolder {
    pub path: String,
    pub needed_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataMoveProgress {
    pub copied_bytes: u64,
    pub total_bytes: u64,
}

#[tauri::command]
pub async fn get_data_location(
    state: State<'_, AppState>,
    location: State<'_, DataLocationState>,
) -> Result<DataLocationInfo> {
    let size_bytes =
        crate::services::storage_stats::collect_storage_stats(&state.app_data_dir, state.db.db_path())?.total_bytes;
    Ok(DataLocationInfo {
        supported: !cfg!(any(target_os = "ios", target_os = "android")),
        path: state.app_data_dir.display().to_string(),
        default_path: location.default_dir.display().to_string(),
        is_default: state.app_data_dir == location.default_dir,
        set_by_env: data_move::ensure_movable(location.source).is_err(),
        previous_path: state.db.get_preference(data_move::PREVIOUS_LOCATION_PREF)?,
        size_bytes,
    })
}

/// Forget the "old copy is still at …" note once the user has read it.
#[tauri::command]
pub async fn dismiss_previous_data_location(state: State<'_, AppState>) -> Result<()> {
    state.db.delete_preference(data_move::PREVIOUS_LOCATION_PREF)
}

/// Let the user pick a folder for the data. `None` when the dialog was
/// cancelled. The data would go into an `EmailOps` folder inside it; that
/// target is checked now so the user hears about a full or busy disk before
/// confirming, and remembered for `move_data_location`.
#[tauri::command]
pub async fn pick_data_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    location: State<'_, DataLocationState>,
) -> Result<Option<PickedDataFolder>> {
    use tauri_plugin_dialog::DialogExt;
    data_move::ensure_movable(location.source)?;
    let picked = tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(|e| AppError::IoError(format!("The folder dialog failed: {e}")))?;
    let Some(folder) = picked else {
        return Ok(None);
    };
    let folder = folder
        .into_path()
        .map_err(|e| AppError::InvalidInput(format!("Not a local folder: {e}")))?;
    let target = data_move::target_for_picked(&folder);
    let current = state.app_data_dir.clone();
    let check_target = target.clone();
    let needed_bytes = tauri::async_runtime::spawn_blocking(move || data_move::check_move(&current, &check_target))
        .await
        .map_err(|e| AppError::IoError(format!("Checking the folder failed: {e}")))??;
    *location.picked.lock().unwrap_or_else(PoisonError::into_inner) = Some(target.clone());
    Ok(Some(PickedDataFolder {
        path: target.display().to_string(),
        needed_bytes,
    }))
}

/// Move the data to the folder last picked, or back to the default when
/// `to_default` is set, then restart into it. Progress arrives as
/// `data-move-progress` events. On success the app exits and starts again;
/// on failure nothing was changed and the error comes back.
#[tauri::command]
pub async fn move_data_location(
    app: AppHandle,
    state: State<'_, AppState>,
    location: State<'_, DataLocationState>,
    to_default: bool,
) -> Result<()> {
    data_move::ensure_movable(location.source)?;
    let target = if to_default {
        location.default_dir.clone()
    } else {
        location
            .picked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| AppError::InvalidInput("no folder was picked for the data".into()))?
    };
    let current = state.app_data_dir.clone();
    // Refuse before stopping anything, so a refusal leaves sync running.
    let (check_current, check_target) = (current.clone(), target.clone());
    tauri::async_runtime::spawn_blocking(move || data_move::check_move(&check_current, &check_target))
        .await
        .map_err(|e| AppError::IoError(format!("Checking the folder failed: {e}")))??;

    crate::services::logger::log(
        "info",
        "system",
        format!("Moving the data folder to {}", target.display()),
    );
    // Sync writes would land in the old copy after the snapshot. The app
    // restarts once the move is done, which starts sync again.
    state.scheduler.stop();
    let db = state.db.clone();
    let default_dir = location.default_dir.clone();
    let moved = tauri::async_runtime::spawn_blocking(move || move_with_progress(&db, &current, &target, &default_dir))
        .await
        .map_err(|e| AppError::IoError(format!("Moving the data failed: {e}")))?;
    if let Err(e) = moved {
        crate::services::logger::log(
            "error",
            "system",
            format!("Moving the data folder failed: {e}. Restart EmailOps to resume syncing."),
        );
        return Err(e);
    }
    crate::services::logger::log("success", "system", "Data folder moved; restarting".to_string());
    // Not `app.restart()`: from a command thread it relaunches after
    // `RunEvent::Exit`, which our exit handler never returns from. That
    // handler relaunches instead (`util::relaunch`).
    crate::util::relaunch::request();
    app.exit(0);
    Ok(())
}

fn move_with_progress(db: &crate::db::Database, current: &Path, target: &Path, default_dir: &Path) -> Result<()> {
    // One event per whole percent: a folder of small attachments would
    // otherwise flood the webview with thousands of events.
    let mut last_percent = None;
    data_move::move_data_dir(db, current, target, default_dir, &mut |copied, total| {
        let percent = copied.saturating_mul(100).checked_div(total).unwrap_or(100);
        if last_percent != Some(percent) {
            last_percent = Some(percent);
            crate::services::events::emit(
                "data-move-progress",
                DataMoveProgress {
                    copied_bytes: copied,
                    total_bytes: total,
                },
            );
        }
    })
}
