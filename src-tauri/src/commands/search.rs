use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::db::emails::list::ListView;
use crate::models::error::AppError;
use crate::models::ThreadCursor;
use crate::services;
use crate::services::list::{ListRequest, ThreadPage};
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

/// The largest page a caller may ask for: what a refresh of a long list needs.
const MAX_LIST_LIMIT: i32 = 500;

/// The account a page's `search` event belongs to, when it writes one: the first
/// page of a typed search. Refreshes, later pages and filter clicks are not
/// searches, and a unified search has no single owning account.
fn search_event_account(account_id: Option<&str>, is_first_page: bool, record_search: bool) -> Option<&str> {
    account_id.filter(|_| is_first_page && record_search)
}

/// One page of a search or filter. `account_id: None` lists every enabled
/// account, and a query spans every category: the category tabs narrow a
/// mailbox, not a query.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn list_threads(
    state: State<'_, AppState>,
    account_id: Option<String>,
    query: String,
    view: String,
    unread_only: bool,
    cursor: Option<ThreadCursor>,
    limit: i32,
    record_search: bool,
) -> Result<ThreadPage, AppError> {
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(AppError::InvalidInput(format!(
            "limit must be between 1 and {MAX_LIST_LIMIT}"
        )));
    }
    let is_first_page = cursor.is_none();
    let page = services::list::list_threads(
        &state.db,
        ListRequest {
            account_id: account_id.as_deref(),
            query: &query,
            view: ListView::parse(&view)?,
            unread_only,
            cursor,
            limit,
        },
    )?;
    if let Some(id) = search_event_account(account_id.as_deref(), is_first_page, record_search) {
        services::memory::on_search(&state.db, id, &query);
    }
    Ok(page)
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

#[cfg(test)]
mod tests {
    use super::search_event_account;

    #[test]
    fn only_the_first_page_of_a_typed_search_in_one_account_is_a_search_event() {
        assert_eq!(search_event_account(Some("acc"), true, true), Some("acc"));
        assert_eq!(search_event_account(Some("acc"), false, true), None, "a later page");
        assert_eq!(
            search_event_account(Some("acc"), true, false),
            None,
            "a refresh or a filter click"
        );
        assert_eq!(search_event_account(None, true, true), None, "a unified search");
    }
}
