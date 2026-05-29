use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::models::error::AppError;
use crate::services;
use crate::services::search::SearchResult;
use crate::AppState;

fn emit_log(_app: &AppHandle, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddingProgressEvent {
    status: String,
    current: u32,
    total: u32,
    message: String,
}

/// Resolve a human-readable account label (the email address) for log
/// messages. Returns `None` when no account is specified (i.e. the action
/// applies to every account).
fn resolve_account_label(state: &AppState, account_id: Option<&str>) -> Result<Option<String>, AppError> {
    match account_id {
        Some(id) => Ok(state.db.get_account(id)?.map(|a| a.email)),
        None => Ok(None),
    }
}

fn emit_embedding_error(app: &AppHandle, message: &str) {
    let _ = app.emit(
        "embedding-progress",
        EmbeddingProgressEvent {
            status: "error".to_string(),
            current: 0,
            total: 0,
            message: message.to_string(),
        },
    );
}

#[tauri::command]
pub async fn search_emails(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    query: String,
    use_ai: Option<bool>,
    categories: Option<Vec<String>>,
) -> Result<SearchResult, AppError> {
    let cmd_start = std::time::Instant::now();
    // Master AI switch overrides any caller request: when AI is disabled the
    // search service must fall back to keyword/FTS only. Reading the flag
    // here keeps the gate at the command boundary and avoids threading a new
    // parameter through the service signature.
    let ai_master_enabled = state.db.is_ai_enabled()?;
    let use_ai_flag = use_ai.unwrap_or(true) && ai_master_enabled;
    let category_count = categories.as_ref().map_or(0, Vec::len);
    emit_log(
        &app,
        "info",
        "search",
        &format!(
            "Searching: \"{}\" (AI: {}, categories: {})",
            query, use_ai_flag, category_count
        ),
    );

    let result = services::search::search_emails(
        &state.db,
        &account_id,
        &query,
        use_ai_flag,
        categories.as_deref(),
        Some(app.clone()),
    )
    .await?;
    emit_log(
        &app,
        "debug",
        "search",
        &format!(
            "[timing] total={:.0}ms, query={:?}, results={}",
            cmd_start.elapsed().as_secs_f64() * 1000.0,
            query,
            result.emails.len(),
        ),
    );

    emit_log(
        &app,
        "success",
        "search",
        &format!("Found {} results via {:?}", result.emails.len(), result.search_method,),
    );

    services::memory::on_search(&state.db, &account_id, &query);

    Ok(result)
}

#[tauri::command]
pub async fn list_ollama_models() -> Result<Vec<String>, AppError> {
    services::search::list_ollama_models().await
}

#[tauri::command]
pub async fn get_ai_model(state: State<'_, AppState>) -> Result<String, AppError> {
    services::search::get_ai_model(&state.db).await
}

#[tauri::command]
pub async fn set_ai_model(app: AppHandle, state: State<'_, AppState>, model: String) -> Result<(), AppError> {
    services::search::set_ai_model(&state.db, &model)?;
    // Refresh dependent UI (LogPanel selectors, AI Settings) so the new model
    // shows up immediately without re-opening the panel.
    let _ = app.emit("ai-config-updated", serde_json::Value::Null);
    Ok(())
}

/// Generate embeddings for emails that don't have them yet
#[tauri::command]
pub async fn generate_embeddings(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<u32, AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    let label = resolve_account_label(&state, account_id.as_deref())?;
    services::embeddings::generate_embeddings(
        &state.db,
        account_id.as_deref(),
        Some(app),
        100, // Generate up to 100 embeddings at a time
        label.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn start_generate_embeddings(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<(), AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    let db = state.db.clone();
    let app_for_task = app.clone();
    let app_for_errors = app.clone();
    let queue = state.ai_background.clone();
    let label = resolve_account_label(&state, account_id.as_deref())?;
    let label_for_log = label.clone().unwrap_or_else(|| "all accounts".to_string());

    emit_log(
        &app,
        "info",
        "embeddings",
        &format!("Queued embedding generation for {}", label_for_log),
    );

    let task_label = format!("embeddings:generate:{}", account_id.as_deref().unwrap_or("all"));
    queue
        .submit_named(&task_label, async move {
            if let Err(error) = services::embeddings::generate_embeddings(
                &db,
                account_id.as_deref(),
                Some(app_for_task),
                100,
                label.as_deref(),
            )
            .await
            {
                let message = error.to_string();
                emit_embedding_error(&app_for_errors, &message);
                emit_log(
                    &app_for_errors,
                    "error",
                    "embeddings",
                    &format!("Embedding generation failed: {}", message),
                );
            }
        })
        .await;

    Ok(())
}

/// Get count of emails without embeddings
#[tauri::command]
pub async fn get_pending_embeddings_count(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<i32, AppError> {
    services::embeddings::count_pending_embeddings(&state.db, account_id.as_deref())
}

/// Delete all embeddings and regenerate them from scratch
/// Use this when the embedding model or content extraction logic changes
#[tauri::command]
pub async fn regenerate_embeddings(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<u32, AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    let label = resolve_account_label(&state, account_id.as_deref())?;
    services::embeddings::regenerate_embeddings(&state.db, account_id.as_deref(), Some(app), 500, label.as_deref())
        .await
}

#[tauri::command]
pub async fn start_regenerate_embeddings(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<(), AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    let db = state.db.clone();
    let app_for_task = app.clone();
    let app_for_errors = app.clone();
    let queue = state.ai_background.clone();
    let label = resolve_account_label(&state, account_id.as_deref())?;
    let label_for_log = label.clone().unwrap_or_else(|| "all accounts".to_string());

    emit_log(
        &app,
        "info",
        "embeddings",
        &format!("Queued search index rebuild for {}", label_for_log),
    );

    let task_label = format!("embeddings:rebuild:{}", account_id.as_deref().unwrap_or("all"));
    queue
        .submit_named(&task_label, async move {
            if let Err(error) = services::embeddings::regenerate_embeddings(
                &db,
                account_id.as_deref(),
                Some(app_for_task),
                500,
                label.as_deref(),
            )
            .await
            {
                let message = error.to_string();
                emit_embedding_error(&app_for_errors, &message);
                emit_log(
                    &app_for_errors,
                    "error",
                    "embeddings",
                    &format!("Embedding regeneration failed: {}", message),
                );
            }
        })
        .await;

    Ok(())
}

/// Rebuild the FTS (full-text search) index
/// Call this once after adding FTS support to index existing emails
#[tauri::command]
pub async fn rebuild_fts_index(state: State<'_, AppState>) -> Result<u32, AppError> {
    state.db.rebuild_fts_index()
}
