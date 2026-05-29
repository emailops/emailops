use tauri::State;

use crate::models::error::AppError;
use crate::models::{FilterSuggestion, FilteredEmailsResult, QuickFilterStats, SmartFilterPref, SmartFilterSuggestion};
use crate::services;
use crate::AppState;

/// Recalculate filter suggestions (excludes removed), persist to DB, return fresh stats
#[tauri::command]
pub async fn refresh_filter_stats(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<QuickFilterStats, AppError> {
    services::filters::refresh_filter_stats(&state.db, &account_id)
}

/// Load previously saved suggestions from DB (no recalculation)
#[tauri::command]
pub async fn get_saved_suggestions(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<SmartFilterSuggestion>, AppError> {
    services::filters::get_saved_suggestions(&state.db, &account_id)
}

#[tauri::command]
pub async fn get_filtered_emails(
    state: State<'_, AppState>,
    account_id: String,
    domain: Option<String>,
    sender_email: Option<String>,
    tag_type: Option<String>,
    tag_value: Option<String>,
    attachment_ext: Option<String>,
    limit: Option<i32>,
    offset: Option<i32>,
) -> Result<FilteredEmailsResult, AppError> {
    services::filters::get_filtered_emails(
        &state.db,
        &account_id,
        domain.as_deref(),
        sender_email.as_deref(),
        tag_type.as_deref(),
        tag_value.as_deref(),
        attachment_ext.as_deref(),
        limit.unwrap_or(50),
        offset.unwrap_or(0),
    )
}

#[tauri::command]
pub async fn get_attachment_ext_stats(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<FilterSuggestion>, AppError> {
    state.db.get_attachment_ext_stats(&account_id)
}

#[tauri::command]
pub async fn get_filter_prefs(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<SmartFilterPref>, AppError> {
    services::filters::get_filter_prefs(&state.db, &account_id)
}

#[tauri::command]
pub async fn pin_filter(
    state: State<'_, AppState>,
    account_id: String,
    filter_type: String,
    filter_value: String,
) -> Result<(), AppError> {
    services::filters::pin_filter(&state.db, &account_id, &filter_type, &filter_value)
}

#[tauri::command]
pub async fn remove_filter(
    state: State<'_, AppState>,
    account_id: String,
    filter_type: String,
    filter_value: String,
) -> Result<(), AppError> {
    services::filters::remove_filter(&state.db, &account_id, &filter_type, &filter_value)
}

#[tauri::command]
pub async fn delete_filter_pref(
    state: State<'_, AppState>,
    account_id: String,
    filter_type: String,
    filter_value: String,
) -> Result<(), AppError> {
    services::filters::delete_filter_pref(&state.db, &account_id, &filter_type, &filter_value)
}
