//! Tauri command surface for the memory subsystem.
//!
//! Thin wrappers over `db::memory` and `services::memory`. The Tasks panel
//! (Phase A) drives the list/create/update commands; the Memory inspector
//! (Phase F) will add fact-management commands on top.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::models::error::AppError;
use crate::models::{CreateTaskRequest, MemoryFact, PendingTask, ThreadState};
use crate::services;
use crate::services::memory::config::MemoryConfig;
use crate::services::tasks::config::TaskConfig;
use crate::AppState;

/// Cancel flag for the active memory backfill. A second call to
/// `start_memory_backfill` while one is running is a no-op (we guard via the
/// `BACKFILL_RUNNING` flag); `cancel_memory_backfill` flips this to true so
/// the background loop exits after its current batch.
static BACKFILL_CANCEL: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
static BACKFILL_RUNNING: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
static TASK_BACKFILL_CANCEL: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
static TASK_BACKFILL_RUNNING: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();

fn backfill_cancel_flag() -> Arc<AtomicBool> {
    BACKFILL_CANCEL.get_or_init(|| Arc::new(AtomicBool::new(false))).clone()
}

fn backfill_running_flag() -> Arc<AtomicBool> {
    BACKFILL_RUNNING
        .get_or_init(|| Arc::new(AtomicBool::new(false)))
        .clone()
}

fn task_backfill_cancel_flag() -> Arc<AtomicBool> {
    TASK_BACKFILL_CANCEL
        .get_or_init(|| Arc::new(AtomicBool::new(false)))
        .clone()
}

fn task_backfill_running_flag() -> Arc<AtomicBool> {
    TASK_BACKFILL_RUNNING
        .get_or_init(|| Arc::new(AtomicBool::new(false)))
        .clone()
}

fn emit_log(_app: &AppHandle, level: &str, message: &str) {
    crate::services::logger::log(level, "memory", message);
}

fn emit_source_log(_app: &AppHandle, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

/// Aggregate counts used by the sidebar badge and the Tasks panel header.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCountsSummary {
    pub total_open: i32,
    pub overdue: i32,
    pub due_today: i32,
    pub awaiting_them: i32,
}

#[tauri::command]
pub async fn list_pending_tasks(
    state: State<'_, AppState>,
    account_id: String,
    status: Option<String>,
    due_before: Option<i64>,
    limit: Option<i32>,
) -> Result<Vec<PendingTask>, AppError> {
    state
        .db
        .list_pending_tasks(&account_id, status.as_deref(), due_before, limit.unwrap_or(200))
}

#[tauri::command]
pub async fn get_task_counts(state: State<'_, AppState>, account_id: String) -> Result<TaskCountsSummary, AppError> {
    let (total_open, overdue, due_today) = state.db.count_pending_tasks(&account_id)?;
    let awaiting_them = state.db.count_open_threads(&account_id, "them")?;
    Ok(TaskCountsSummary {
        total_open,
        overdue,
        due_today,
        awaiting_them,
    })
}

#[tauri::command]
pub async fn create_pending_task(state: State<'_, AppState>, req: CreateTaskRequest) -> Result<PendingTask, AppError> {
    services::tasks::create_task(&state.db, req)
}

#[tauri::command]
pub async fn update_pending_task_status(
    state: State<'_, AppState>,
    task_id: String,
    status: String,
) -> Result<(), AppError> {
    services::tasks::update_task_status(&state.db, &task_id, &status)
}

#[tauri::command]
pub async fn list_open_threads(
    state: State<'_, AppState>,
    account_id: String,
    awaiting: Option<String>,
    limit: Option<i32>,
) -> Result<Vec<ThreadState>, AppError> {
    state
        .db
        .list_open_thread_states(&account_id, awaiting.as_deref(), limit.unwrap_or(100))
}

// ── Memory inspector commands ───────────────────────────────────────────────

#[tauri::command]
pub async fn list_memory_facts(
    state: State<'_, AppState>,
    account_id: String,
    status: Option<String>,
    limit: Option<i32>,
) -> Result<Vec<MemoryFact>, AppError> {
    state
        .db
        .list_memory_facts(&account_id, status.as_deref(), limit.unwrap_or(200))
}

#[tauri::command]
pub async fn promote_memory_fact(state: State<'_, AppState>, fact_id: String) -> Result<(), AppError> {
    state
        .db
        .set_memory_fact_status(&fact_id, "promoted", chrono::Utc::now().timestamp())
}

#[tauri::command]
pub async fn retire_memory_fact(state: State<'_, AppState>, fact_id: String) -> Result<(), AppError> {
    state
        .db
        .set_memory_fact_status(&fact_id, "retired", chrono::Utc::now().timestamp())
}

#[tauri::command]
pub async fn update_memory_fact(state: State<'_, AppState>, fact_id: String, fact: String) -> Result<(), AppError> {
    let trimmed = fact.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInput("fact cannot be empty".into()));
    }
    state
        .db
        .update_memory_fact_text(&fact_id, trimmed, chrono::Utc::now().timestamp())
}

#[tauri::command]
pub async fn delete_memory_fact(state: State<'_, AppState>, fact_id: String) -> Result<(), AppError> {
    state.db.delete_memory_fact(&fact_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCountsSummary {
    pub total: i32,
    pub promoted: i32,
    pub candidate: i32,
}

#[tauri::command]
pub async fn get_memory_counts(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<MemoryCountsSummary, AppError> {
    let total = state.db.count_memory_facts(&account_id, None)?;
    let promoted = state.db.count_memory_facts(&account_id, Some("promoted"))?;
    let candidate = state.db.count_memory_facts(&account_id, Some("candidate"))?;
    Ok(MemoryCountsSummary {
        total,
        promoted,
        candidate,
    })
}

// ── Memory configuration ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_memory_config(state: State<'_, AppState>) -> Result<MemoryConfig, AppError> {
    services::memory::config::get_config(&state.db)
}

#[tauri::command]
pub async fn set_memory_config(state: State<'_, AppState>, config: MemoryConfig) -> Result<(), AppError> {
    services::memory::config::save_config(&state.db, &config)
}

#[tauri::command]
pub async fn get_task_config(state: State<'_, AppState>) -> Result<TaskConfig, AppError> {
    services::tasks::config::get_config(&state.db)
}

#[tauri::command]
pub async fn set_task_config(state: State<'_, AppState>, config: TaskConfig) -> Result<(), AppError> {
    services::tasks::config::save_config(&state.db, &config)
}

// ── Backfill ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillStatus {
    pub running: bool,
    pub remaining: i32,
}

#[tauri::command]
pub async fn get_memory_backfill_status(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<BackfillStatus, AppError> {
    let mem_cfg = services::memory::config::get_config(&state.db)?;
    let remaining = state
        .db
        .count_memory_unextracted_emails(&account_id, &mem_cfg.categories, None)?;
    Ok(BackfillStatus {
        running: backfill_running_flag().load(Ordering::SeqCst),
        remaining,
    })
}

#[tauri::command]
pub async fn start_memory_backfill(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    let running = backfill_running_flag();
    if running.swap(true, Ordering::SeqCst) {
        emit_log(&app, "info", "Backfill already in progress");
        return Ok(());
    }
    // Reset cancel flag for the new run.
    backfill_cancel_flag().store(false, Ordering::SeqCst);

    let db = state.db.clone();
    let cancel = backfill_cancel_flag();
    let running_clone = running.clone();

    let task_label = format!("memory:backfill:{}", account_id);
    state
        .ai_background
        .submit_named(&task_label, async move {
            emit_log(&app, "info", &format!("Memory backfill started for {account_id}"));
            let mut total_processed: u32 = 0;
            loop {
                if cancel.load(Ordering::SeqCst) {
                    emit_log(&app, "info", "Memory backfill cancelled");
                    break;
                }
                let mem_cfg = match services::memory::config::get_config(&db) {
                    Ok(c) => c,
                    Err(e) => {
                        emit_log(&app, "error", &format!("Backfill: memory config load failed: {e}"));
                        break;
                    }
                };
                if !mem_cfg.enabled {
                    emit_log(&app, "warn", "Backfill halted: memory is disabled");
                    break;
                }
                match services::memory::extractor::extract_batch(&db, &app, &account_id, &mem_cfg, Some(&cancel)).await
                {
                    Ok(0) => {
                        emit_log(
                            &app,
                            "success",
                            &format!("Memory backfill complete ({total_processed} emails processed)"),
                        );
                        break;
                    }
                    Ok(n) => {
                        total_processed += n;
                        // Embed + consolidate every batch so the UI reflects
                        // progress. Errors are logged and ignored — the
                        // extractor already marked the rows.
                        if let Err(e) = services::memory::embeddings::embed_pending_facts(&db, &app, &account_id).await
                        {
                            emit_log(&app, "warn", &format!("Backfill: embedding step failed: {e}"));
                        }
                        if let Err(e) = services::memory::consolidation::run_consolidation(&db, Some(&app), &account_id)
                        {
                            emit_log(&app, "warn", &format!("Backfill: consolidation failed: {e}"));
                        }
                        // Notify the frontend that facts changed so the Memory
                        // inspector and sidebar counts refresh without manual
                        // reload or polling.
                        let _ = app.emit("memory-facts-changed", serde_json::json!({ "accountId": account_id }));
                    }
                    Err(e) => {
                        emit_log(&app, "error", &format!("Backfill failed: {e}"));
                        break;
                    }
                }
            }
            running_clone.store(false, Ordering::SeqCst);
        })
        .await;

    Ok(())
}

#[tauri::command]
pub async fn cancel_memory_backfill() -> Result<(), AppError> {
    backfill_cancel_flag().store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn get_task_backfill_status(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<BackfillStatus, AppError> {
    let cfg = services::tasks::config::get_config(&state.db)?;
    let min_ts = cfg.backfill_min_timestamp(chrono::Utc::now().timestamp());
    let remaining = state
        .db
        .count_task_unextracted_emails(&account_id, &cfg.categories, min_ts)?;
    Ok(BackfillStatus {
        running: task_backfill_running_flag().load(Ordering::SeqCst),
        remaining,
    })
}

#[tauri::command]
pub async fn start_task_backfill(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    let running = task_backfill_running_flag();
    if running.swap(true, Ordering::SeqCst) {
        emit_source_log(&app, "info", "tasks", "Task backfill already in progress");
        return Ok(());
    }
    task_backfill_cancel_flag().store(false, Ordering::SeqCst);

    let db = state.db.clone();
    let cancel = task_backfill_cancel_flag();
    let running_clone = running.clone();
    let task_label = format!("tasks:backfill:{}", account_id);
    state
        .ai_background
        .submit_named(&task_label, async move {
            emit_source_log(
                &app,
                "info",
                "tasks",
                &format!("Task backfill started for {account_id}"),
            );
            let mut total_processed: u32 = 0;
            loop {
                if cancel.load(Ordering::SeqCst) {
                    emit_source_log(&app, "info", "tasks", "Task backfill cancelled");
                    break;
                }
                let cfg = match services::tasks::config::get_config(&db) {
                    Ok(c) => c,
                    Err(e) => {
                        emit_source_log(
                            &app,
                            "error",
                            "tasks",
                            &format!("Backfill: task config load failed: {e}"),
                        );
                        break;
                    }
                };
                if !cfg.enabled {
                    emit_source_log(&app, "warn", "tasks", "Backfill halted: tasks are disabled");
                    break;
                }
                match services::tasks::extractor::extract_batch(&db, &app, &account_id, &cfg, Some(&cancel)).await {
                    Ok(0) => {
                        emit_source_log(
                            &app,
                            "success",
                            "tasks",
                            &format!("Task backfill complete ({total_processed} emails processed)"),
                        );
                        break;
                    }
                    Ok(n) => total_processed += n,
                    Err(e) => {
                        emit_source_log(&app, "error", "tasks", &format!("Task backfill failed: {e}"));
                        break;
                    }
                }
            }
            running_clone.store(false, Ordering::SeqCst);
        })
        .await;

    Ok(())
}

#[tauri::command]
pub async fn cancel_task_backfill() -> Result<(), AppError> {
    task_backfill_cancel_flag().store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn reset_memory_extraction(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<u32, AppError> {
    let count = state.db.reset_memory_extraction(&account_id)?;
    emit_log(
        &app,
        "info",
        &format!("Memory extraction reset: {count} emails re-queued for extraction"),
    );
    Ok(count)
}

#[tauri::command]
pub async fn reset_task_extraction(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<u32, AppError> {
    let count = state.db.reset_task_extraction(&account_id)?;
    emit_source_log(
        &app,
        "info",
        "tasks",
        &format!("Task extraction reset: {count} emails re-queued for extraction"),
    );
    Ok(count)
}

// ── Consolidation trigger ───────────────────────────────────────────────────

#[tauri::command]
pub async fn run_memory_consolidation(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    services::memory::consolidation::run_consolidation(&state.db, Some(&app), &account_id)?;
    Ok(())
}
