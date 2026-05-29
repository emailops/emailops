use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::models::error::AppError;
use crate::models::{Account, AccountSettings};
use crate::services;
use crate::sync::imap::ImapCredentials;
use crate::AppState;

/// Frontend-facing DTO for IMAP credentials (camelCase field names).
/// The backend `ImapCredentials` uses snake_case for on-disk keychain JSON, so
/// we expose a separate DTO here to keep the JS boundary camelCase without
/// changing the stored serialization format.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapCredentialsDto {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub smtp_host: String,
    pub smtp_port: u16,
}

impl From<ImapCredentials> for ImapCredentialsDto {
    fn from(c: ImapCredentials) -> Self {
        Self {
            host: c.host,
            port: c.port,
            username: c.username,
            password: c.password,
            smtp_host: c.smtp_host,
            smtp_port: c.smtp_port,
        }
    }
}

#[tauri::command]
pub async fn add_account(
    state: State<'_, AppState>,
    provider: String,
    sync_from_timestamp: Option<i64>,
) -> Result<Account, AppError> {
    match provider.as_str() {
        "gmail" | "outlook" => services::accounts::add_account(&state.db, &provider, sync_from_timestamp).await,
        _ => Err(AppError::InvalidInput(format!("Unknown provider: {}", provider))),
    }
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>, AppError> {
    services::accounts::list_accounts(&state.db)
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<(), AppError> {
    // Signal any in-progress sync for this account to abort at the next batch boundary.
    {
        let mut flags = state.sync_abort_flags.lock().unwrap_or_else(|e| e.into_inner());
        flags
            .entry(account_id.clone())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .store(true, Ordering::Relaxed);
    }
    services::accounts::remove_account(&state.db, &account_id, &state.app_data_dir)
}

#[tauri::command]
pub async fn reauthenticate_account(state: State<'_, AppState>, account_id: String) -> Result<(), AppError> {
    services::accounts::reauthenticate_account(&state.db, &account_id).await
}

#[tauri::command]
pub async fn reorder_accounts(state: State<'_, AppState>, account_ids: Vec<String>) -> Result<(), AppError> {
    services::accounts::reorder_accounts(&state.db, &account_ids)
}

#[tauri::command]
pub async fn set_account_enabled(
    state: State<'_, AppState>,
    account_id: String,
    enabled: bool,
) -> Result<(), AppError> {
    services::accounts::set_account_enabled(&state.db, &account_id, enabled)
}

#[tauri::command]
pub async fn test_imap_connection(
    host: String,
    port: u16,
    username: String,
    password: String,
    smtp_host: String,
    smtp_port: u16,
) -> Result<(), AppError> {
    let credentials = ImapCredentials {
        host,
        port,
        username,
        password,
        smtp_host,
        smtp_port,
    };
    services::accounts::test_imap_connection(credentials).await
}

#[tauri::command]
pub async fn get_imap_credentials(account_id: String) -> Result<ImapCredentialsDto, AppError> {
    services::accounts::get_imap_credentials(&account_id).map(Into::into)
}

/// Load IMAP server settings for the re-auth/edit dialog. Unlike
/// `get_imap_credentials`, this returns a partial result (with `hasPassword:
/// false` and an empty password string) when only the keychain entry is
/// missing — so the dialog can still pre-fill host/port/username/smtp settings
/// from the DB and prompt the user only for the password.
#[tauri::command]
pub async fn get_imap_settings(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<services::accounts::ImapEditSettings, AppError> {
    services::accounts::load_imap_settings_for_edit(&state.db, &account_id)
}

#[tauri::command]
pub async fn update_imap_credentials(
    state: State<'_, AppState>,
    account_id: String,
    host: String,
    port: u16,
    username: String,
    password: String,
    smtp_host: String,
    smtp_port: u16,
) -> Result<(), AppError> {
    // Ensure the account exists and is IMAP
    let accounts = services::accounts::list_accounts(&state.db)?;
    let account = accounts
        .into_iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| AppError::InvalidInput(format!("Unknown account: {}", account_id)))?;
    if account.provider != "imap" {
        return Err(AppError::InvalidInput("Account is not an IMAP account".to_string()));
    }

    let credentials = ImapCredentials {
        host,
        port,
        username,
        password,
        smtp_host,
        smtp_port,
    };

    // Verify credentials work before saving.
    services::accounts::test_imap_connection(credentials.clone()).await?;
    services::accounts::store_imap_credentials(&account_id, &credentials)?;
    Ok(())
}

#[tauri::command]
pub async fn add_imap_account(
    state: State<'_, AppState>,
    host: String,
    port: u16,
    username: String,
    password: String,
    smtp_host: String,
    smtp_port: u16,
    display_name: Option<String>,
    sync_from_timestamp: Option<i64>,
) -> Result<Account, AppError> {
    let credentials = ImapCredentials {
        host,
        port,
        username,
        password,
        smtp_host,
        smtp_port,
    };
    services::accounts::add_imap_account(&state.db, credentials, display_name, sync_from_timestamp).await
}

#[tauri::command]
pub async fn update_account_sync_from(
    state: State<'_, AppState>,
    account_id: String,
    sync_from_timestamp: Option<i64>,
) -> Result<Account, AppError> {
    services::accounts::update_account_sync_from(&state.db, &account_id, sync_from_timestamp)
}

#[tauri::command]
pub async fn get_account_settings(state: State<'_, AppState>, account_id: String) -> Result<AccountSettings, AppError> {
    let key = format!("account_settings:{}", account_id);
    match state.db.get_preference(&key)? {
        Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
        None => Ok(AccountSettings::default()),
    }
}

#[tauri::command]
pub async fn set_account_settings(
    state: State<'_, AppState>,
    account_id: String,
    settings: AccountSettings,
) -> Result<(), AppError> {
    let key = format!("account_settings:{}", account_id);
    let json = serde_json::to_string(&settings).map_err(|e| AppError::InvalidInput(e.to_string()))?;
    state.db.set_preference(&key, &json)?;
    Ok(())
}

/// Categories that should appear as Inbox filter tabs for the given account.
/// Provider-aware: Gmail uses the user's opt-in list, Outlook returns its
/// fixed focused/other pair, IMAP returns empty. See
/// `services::accounts::available_categories` for the decision logic.
#[tauri::command]
pub async fn get_available_categories(state: State<'_, AppState>, account_id: String) -> Result<Vec<String>, AppError> {
    services::accounts::available_categories_for_account(&state.db, &account_id)
}
