use tauri::State;

use crate::models::error::AppError;
use crate::models::{FilterSuggestion, FilteredEmailsResult, QuickFilterStats, SmartFilterPref, SmartFilterSuggestion};
use crate::services;
use crate::AppState;

/// Recalculate filter suggestions (excludes removed), persist to DB, return fresh stats.
/// `account_id: None` = unified ("All accounts"): refreshes every enabled
/// account, returns aggregated stats.
#[tauri::command]
pub async fn refresh_filter_stats(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<QuickFilterStats, AppError> {
    services::filters::refresh_filter_stats(&state.db, account_id.as_deref())
}

/// Load previously saved suggestions from DB (no recalculation).
/// `account_id: None` aggregates across every enabled account.
#[tauri::command]
pub async fn get_saved_suggestions(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<SmartFilterSuggestion>, AppError> {
    services::filters::get_saved_suggestions(&state.db, account_id.as_deref())
}

/// `account_id: None` filters across every enabled account (unified view).
#[tauri::command]
pub async fn get_filtered_emails(
    state: State<'_, AppState>,
    account_id: Option<String>,
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
        account_id.as_deref(),
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

/// `account_id: None` returns unified prefs (pinned-beats-removed dedup).
#[tauri::command]
pub async fn get_filter_prefs(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<SmartFilterPref>, AppError> {
    services::filters::get_filter_prefs(&state.db, account_id.as_deref())
}

/// `account_id: None` fans the pin out to every enabled account.
#[tauri::command]
pub async fn pin_filter(
    state: State<'_, AppState>,
    account_id: Option<String>,
    filter_type: String,
    filter_value: String,
) -> Result<(), AppError> {
    services::filters::pin_filter(&state.db, account_id.as_deref(), &filter_type, &filter_value)
}

/// `account_id: None` fans the removal out to every enabled account.
#[tauri::command]
pub async fn remove_filter(
    state: State<'_, AppState>,
    account_id: Option<String>,
    filter_type: String,
    filter_value: String,
) -> Result<(), AppError> {
    services::filters::remove_filter(&state.db, account_id.as_deref(), &filter_type, &filter_value)
}

/// `account_id: None` deletes the pref from every enabled account.
#[tauri::command]
pub async fn delete_filter_pref(
    state: State<'_, AppState>,
    account_id: Option<String>,
    filter_type: String,
    filter_value: String,
) -> Result<(), AppError> {
    services::filters::delete_filter_pref(&state.db, account_id.as_deref(), &filter_type, &filter_value)
}
