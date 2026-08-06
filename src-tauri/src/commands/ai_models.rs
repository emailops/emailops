// Tauri commands for llama.cpp model management.
//
// These commands are always compiled (even without the `llamacpp` feature)
// so the frontend can call them and receive a clean error on non-embedded
// backends. The actual download logic requires the model_manager module.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::ai::model_catalog::{ModelKind, CATALOG};
use crate::ai::model_fit::{model_fit, ModelFit};
use crate::ai::model_manager::{self, LocalModel, ModelDownloadProgress};
use crate::models::error::AppError;
use crate::AppState;

fn emit_log(_app: &AppHandle, level: &str, message: &str) {
    crate::services::logger::log(level, "ai_models", message);
}

// ── Catalog ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModelResponse {
    pub id: String,
    pub display_name: String,
    pub kind: String,
    pub size_bytes: u64,
    pub context_window: u32,
    pub license: String,
    pub min_ram_gb: u8,
    pub recommended: bool,
    pub supports_tools: bool,
    /// Whether this model is already downloaded locally.
    pub is_local: bool,
    /// Whether the local file is a symlink to a file elsewhere on disk
    /// (via `link_local_model`) rather than a downloaded copy. Only
    /// meaningful when `is_local` is true.
    pub is_linked: bool,
    /// How this entry stands on this device: `fits` | `tight` | `tooLarge` |
    /// `noDiskSpace`. See `ai::model_fit`.
    pub fit: String,
    /// Whether the UI should offer a download. False on a phone for entries
    /// whose weights cannot fit under the per-process memory ceiling.
    pub downloadable: bool,
}

#[tauri::command]
pub async fn list_catalog_models(state: State<'_, AppState>) -> Result<Vec<CatalogModelResponse>, AppError> {
    let local = model_manager::list_local_models(&state.app_data_dir);
    let local_by_id: HashMap<&str, &model_manager::LocalModel> = local.iter().map(|m| (m.id.as_str(), m)).collect();

    // Probed once for the whole list rather than per entry: both are syscalls,
    // and a listing where two rows disagreed about the free disk would be worse
    // than one that is a few kilobytes stale.
    let available_memory = crate::util::system::total_ram_bytes();
    let available_disk = crate::util::system::available_disk_bytes(&state.app_data_dir);
    let hard_limit = crate::util::system::memory_limit_is_hard();

    let fit_of = |m: &crate::ai::model_catalog::CatalogModel| {
        model_fit(m.size_bytes, m.min_ram_gb, available_memory, available_disk, hard_limit)
    };

    let models: Vec<CatalogModelResponse> = CATALOG
        .iter()
        .map(|m| {
            let fit = fit_of(m);
            let is_local = local_by_id.contains_key(m.id);
            CatalogModelResponse {
                id: m.id.to_string(),
                display_name: m.display_name.to_string(),
                kind: match m.kind {
                    ModelKind::Chat => "chat".to_string(),
                    ModelKind::Embedding => "embedding".to_string(),
                },
                size_bytes: m.size_bytes,
                context_window: m.context_window,
                license: m.license.to_string(),
                min_ram_gb: m.min_ram_gb,
                recommended: m.recommended,
                supports_tools: m.supports_tools,
                is_local,
                is_linked: local_by_id.get(m.id).map(|m| m.is_linked).unwrap_or(false),
                fit: fit.as_str().to_string(),
                // An already-downloaded model stays actionable (delete/select)
                // whatever the probes now say.
                downloadable: fit.is_downloadable() || is_local,
            }
        })
        .collect();

    Ok(models)
}

// ── Local models ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_local_models(state: State<'_, AppState>) -> Result<Vec<LocalModel>, AppError> {
    Ok(model_manager::list_local_models(&state.app_data_dir))
}

#[tauri::command]
pub async fn delete_local_model(state: State<'_, AppState>, model_id: String, kind: String) -> Result<(), AppError> {
    let model_kind = match kind.as_str() {
        "chat" => ModelKind::Chat,
        "embedding" => ModelKind::Embedding,
        _ => return Err(AppError::InvalidInput(format!("Unknown model kind: {}", kind))),
    };
    model_manager::delete_local_model(&state.app_data_dir, model_kind, &model_id)
}

// ── Download ──────────────────────────────────────────────────────────────────

/// Active download tasks keyed by model_id. Stored in AppState via a Mutex
/// so cancel commands can reach the right token. Populated at download start,
/// removed on completion or cancellation.
///
/// We keep this as a module-level Mutex rather than AppState field to avoid a
/// large refactor of AppState for the Step 3 skeleton.
static ACTIVE_DOWNLOADS: std::sync::OnceLock<Mutex<HashMap<String, model_manager::CancelToken>>> =
    std::sync::OnceLock::new();

fn active_downloads() -> &'static Mutex<HashMap<String, model_manager::CancelToken>> {
    ACTIVE_DOWNLOADS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Shared completion handling for both `start_model_download` and
/// `link_local_model`'s background task: emit the terminal progress event,
/// log it, drop the cancel token from `ACTIVE_DOWNLOADS`, and — on success —
/// auto-select the model as the configured default if none was configured
/// yet. Kept in one place so the two flows can't drift.
fn finish_model_op(
    app: &AppHandle,
    db: &crate::db::Database,
    app_data_dir: &std::path::Path,
    model_id: &str,
    entry_kind: ModelKind,
    result: Result<std::path::PathBuf, AppError>,
) {
    match result {
        Ok(_path) => {
            let _ = app.emit(
                "model-download-progress",
                ModelDownloadProgress {
                    model_id: model_id.to_string(),
                    downloaded_bytes: 0,
                    total_bytes: 0,
                    status: "complete".to_string(),
                    error: None,
                },
            );
            emit_log(app, "success", &format!("Model ready: {}", model_id));
            if let Ok(mut map) = active_downloads().lock() {
                map.remove(model_id);
            }

            // Auto-select: if no model of this kind is configured — or the
            // configured one is not actually available locally — set this
            // model as default so the user doesn't have to click Save.
            let raw_chat = db.get_preference("ai_model").ok().flatten();
            let raw_embed = db.get_preference("ai_embedding_model").ok().flatten();
            let provider_saved = db.get_preference("ai_provider").ok().flatten();

            let local = model_manager::list_local_models(app_data_dir);
            let local_chat_ids: std::collections::HashSet<&str> = local
                .iter()
                .filter(|m| m.kind == ModelKind::Chat)
                .map(|m| m.id.as_str())
                .collect();
            let local_embed_ids: std::collections::HashSet<&str> = local
                .iter()
                .filter(|m| m.kind == ModelKind::Embedding)
                .map(|m| m.id.as_str())
                .collect();

            let chat_missing = raw_chat
                .as_deref()
                .map(|s| s.is_empty() || !local_chat_ids.contains(s))
                .unwrap_or(true);
            let embed_missing = raw_embed
                .as_deref()
                .map(|s| s.is_empty() || !local_embed_ids.contains(s))
                .unwrap_or(true);
            // Only auto-select the embedded provider where it can actually run.
            // On an Intel Mac the universal bundle carries a Metal runtime with
            // no Apple7 GPU to execute it, so defaulting to it would hand the
            // user a provider that fails every turn with `Decode Error -3`.
            let no_provider = provider_saved.is_none()
                && crate::ai::gpu_plan::embedded_runtime_supported(std::env::consts::OS, std::env::consts::ARCH);
            // Helper: persist a preference and surface failures in the
            // output panel. CLAUDE.md mandates we never swallow DB errors
            // silently; auto-select would otherwise look like it succeeded
            // while the next launch re-prompts.
            let save = |key: &str, value: &str| {
                if let Err(e) = db.set_preference(key, value) {
                    emit_log(app, "error", &format!("failed to persist preference {key}: {e}"));
                }
            };
            let selected = match entry_kind {
                ModelKind::Chat if chat_missing => {
                    save("ai_model", model_id);
                    if no_provider {
                        save("ai_provider", "llamacpp");
                    }
                    true
                }
                ModelKind::Embedding if embed_missing => {
                    save("ai_embedding_model", model_id);
                    if no_provider {
                        save("ai_provider", "llamacpp");
                    }
                    true
                }
                _ => false,
            };
            if selected {
                let _ = app.emit("ai-config-updated", serde_json::Value::Null);
            }
        }
        Err(AppError::Cancelled) => {
            // User cancelled — emit a distinct "cancelled" status so the UI
            // can clear progress without showing a red error banner.
            let _ = app.emit(
                "model-download-progress",
                ModelDownloadProgress {
                    model_id: model_id.to_string(),
                    downloaded_bytes: 0,
                    total_bytes: 0,
                    status: "cancelled".to_string(),
                    error: None,
                },
            );
            emit_log(app, "info", &format!("Cancelled: {}", model_id));
            if let Ok(mut map) = active_downloads().lock() {
                map.remove(model_id);
            }
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = app.emit(
                "model-download-progress",
                ModelDownloadProgress {
                    model_id: model_id.to_string(),
                    downloaded_bytes: 0,
                    total_bytes: 0,
                    status: "error".to_string(),
                    error: Some(msg.clone()),
                },
            );
            emit_log(
                app,
                "error",
                &format!("Model operation failed for {}: {}", model_id, msg),
            );
            if let Ok(mut map) = active_downloads().lock() {
                map.remove(model_id);
            }
        }
    }
}

#[tauri::command]
pub async fn start_model_download(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), AppError> {
    // Validate the model exists in the catalog.
    let entry = crate::ai::model_catalog::find(&model_id)
        .ok_or_else(|| AppError::NotFound(format!("Model '{}' not found in catalog", model_id)))?;

    // Refuse a download this device cannot use, on the same decision the
    // catalog listing renders from — the UI hides these entries, but a stale
    // list or a direct call must not start a 3 GB transfer that ends in a
    // jetsam kill or a full disk.
    let app_data_dir = state.app_data_dir.clone();
    let available_disk = crate::util::system::available_disk_bytes(&app_data_dir);
    match crate::ai::model_fit::model_fit(
        entry.size_bytes,
        entry.min_ram_gb,
        crate::util::system::total_ram_bytes(),
        available_disk,
        crate::util::system::memory_limit_is_hard(),
    ) {
        ModelFit::NoDiskSpace => {
            return Err(AppError::AiError(format!(
                "Not enough disk space: {} needs {:.1} GB plus room to spare, available {:.1} GB",
                entry.display_name,
                entry.size_bytes as f64 / 1e9,
                available_disk.unwrap_or(0) as f64 / 1e9
            )));
        }
        ModelFit::TooLarge => {
            return Err(AppError::AiError(format!(
                "{} needs about {:.1} GB of memory, more than this device allows a single app",
                entry.display_name,
                entry.size_bytes as f64 / 1e9
            )));
        }
        ModelFit::Fits | ModelFit::Tight => {}
    }

    let model_id_clone = model_id.clone();
    let app_clone = app.clone();
    let db_clone = state.db.clone();
    let entry_kind = entry.kind; // Copy

    // Create the cancel token + handle BEFORE submitting the task and store
    // the sender side in ACTIVE_DOWNLOADS so cancel_model_download can reach
    // it. The handle (receiver) travels into download_model and is polled
    // between chunks. Without this, cancel was a no-op (token was never
    // stored — see prior bug).
    let (cancel_token, cancel_handle) = model_manager::CancelToken::new();
    if let Ok(mut map) = active_downloads().lock() {
        map.insert(model_id_clone.clone(), cancel_token);
    }

    emit_log(&app, "info", &format!("Starting download: {}", entry.display_name));

    // Submit download to the db_queue (fast I/O, not AI-queue).
    let task_label = format!("model_download:{}", model_id_clone);
    state
        .db_queue
        .submit_named(&task_label, async move {
            let progress_app = app_clone.clone();
            let mid = model_id_clone.clone();

            let result = model_manager::download_model(
                &app_data_dir,
                &mid,
                Some(cancel_handle),
                move |progress: ModelDownloadProgress| {
                    let _ = progress_app.emit("model-download-progress", &progress);
                },
            )
            .await;

            finish_model_op(&app_clone, &db_clone, &app_data_dir, &mid, entry_kind, result);
        })
        .await;

    Ok(())
}

/// Point `<app_data_dir>/models/{chat,embed}/<model_id>.gguf` at a GGUF file
/// the user already has on disk (a symlink, not a copy — see
/// `model_manager::link_local_model`), after verifying it matches the
/// catalog entry's pinned SHA-256. Lets the user skip re-downloading a model
/// they've already fetched some other way.
#[tauri::command]
pub async fn link_local_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
    source_path: String,
) -> Result<(), AppError> {
    // Validate the model exists in the catalog.
    let entry = crate::ai::model_catalog::find(&model_id)
        .ok_or_else(|| AppError::NotFound(format!("Model '{}' not found in catalog", model_id)))?;

    // Check RAM ceiling — a linked model still has to run. No disk-space
    // check: linking creates a symlink, not a copy, so it doesn't consume
    // meaningful additional disk space.
    let ram_gb = crate::util::system::total_ram_gb();
    if ram_gb > 0 && ram_gb < entry.min_ram_gb as u64 {
        return Err(AppError::AiError(format!(
            "This model requires {} GB RAM but your system has {} GB",
            entry.min_ram_gb, ram_gb
        )));
    }

    let app_data_dir = state.app_data_dir.clone();
    let source = std::path::PathBuf::from(source_path);
    let model_id_clone = model_id.clone();
    let app_clone = app.clone();
    let db_clone = state.db.clone();
    let entry_kind = entry.kind;
    let entry_owned = entry.clone();

    let (cancel_token, cancel_handle) = model_manager::CancelToken::new();
    if let Ok(mut map) = active_downloads().lock() {
        map.insert(model_id_clone.clone(), cancel_token);
    }

    emit_log(
        &app,
        "info",
        &format!("Using existing file for: {}", entry.display_name),
    );

    let task_label = format!("model_link:{}", model_id_clone);
    state
        .db_queue
        .submit_named(&task_label, async move {
            let progress_app = app_clone.clone();
            let mid = model_id_clone.clone();

            let result = model_manager::link_local_model(
                &app_data_dir,
                &entry_owned,
                &source,
                Some(cancel_handle),
                move |progress: ModelDownloadProgress| {
                    let _ = progress_app.emit("model-download-progress", &progress);
                },
            )
            .await;

            finish_model_op(&app_clone, &db_clone, &app_data_dir, &mid, entry_kind, result);
        })
        .await;

    Ok(())
}

#[tauri::command]
pub async fn cancel_model_download(_state: State<'_, AppState>, model_id: String) -> Result<(), AppError> {
    if let Ok(mut map) = active_downloads().lock() {
        if let Some(token) = map.remove(&model_id) {
            token.cancel();
        }
    }
    Ok(())
}

// ── System helpers ────────────────────────────────────────────────────────────
//
// Both probes used to live here as macOS/unix-only `libc` calls: RAM returned 0
// on anything but macOS and disk space returned `Err` on anything but unix,
// which silently disabled the pre-download guards on Windows. They now delegate
// to the single portable implementation in `util::system`.
