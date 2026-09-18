use tauri::{AppHandle, State};

use crate::models::error::AppError;
use crate::models::{Account, AccountSettings};
use crate::services;
use crate::sync::imap::ImapCredentials;
use crate::AppState;

#[tauri::command]
pub async fn add_account(
    state: State<'_, AppState>,
    provider: String,
    sync_from_timestamp: Option<i64>,
) -> Result<Account, AppError> {
    let account = match provider.as_str() {
        "gmail" | "outlook" => services::accounts::add_account(&state.db, &provider, sync_from_timestamp).await?,
        _ => return Err(AppError::InvalidInput(format!("Unknown provider: {}", provider))),
    };
    // The scheduler only enumerates accounts at startup, so without this the
    // account just added would have no poll loop for the rest of the session.
    state.scheduler.watch_account(&account);
    Ok(account)
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>, AppError> {
    services::accounts::list_accounts(&state.db)
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<(), AppError> {
    // Signal any in-progress sync for this account to abort at the next batch boundary.
    services::emails::request_sync_abort(&state.sync_abort_flags, &account_id);
    // Stop the background loops before the data goes away, so no IDLE watcher
    // or poll tick keeps running against an account that no longer exists.
    state.scheduler.unwatch_account(&account_id);
    services::accounts::remove_account(&state.db, &account_id, &state.app_data_dir)?;
    // Drop the per-account queue/lock/abort-flag entries the sync paths created.
    state.forget_account(&account_id);
    Ok(())
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
    services::accounts::set_account_enabled(&state.db, &account_id, enabled)?;
    // Keep the watched set in step with the `enabled` filter `start()` applies,
    // so toggling an account takes effect without restarting the app.
    if enabled {
        match state.db.get_account(&account_id) {
            Ok(Some(account)) => state.scheduler.watch_account(&account),
            Ok(None) => services::logger::log(
                "error",
                "account",
                format!("enabled {account_id} but it no longer exists — background sync not started"),
            ),
            Err(e) => services::logger::log(
                "error",
                "account",
                format!("enabled {account_id} but could not start its background sync: {e}"),
            ),
        }
    } else {
        state.scheduler.unwatch_account(&account_id);
    }
    Ok(())
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

/// Load IMAP server settings for the re-auth/edit dialog.
///
/// Never returns the password — only `hasPassword`, so the dialog knows whether
/// to offer "leave blank to keep the current password". When the keychain is
/// missing *or* unreadable, the server fields still come back from the DB mirror
/// and `keychainError` explains why the password is unavailable.
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

    // An empty password means the user left the box untouched — the dialog is
    // never given the stored password to echo back, so reuse it here.
    let password = services::accounts::resolve_update_password(&account_id, &password)?;

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
    let account =
        services::accounts::add_imap_account(&state.db, credentials, display_name, sync_from_timestamp).await?;
    // Start this account's IMAP IDLE watcher now; the scheduler's own account
    // enumeration only ever runs at startup.
    state.scheduler.watch_account(&account);
    Ok(account)
}

/// Persist a new sync range and make it take effect *now*.
///
/// A range change that only lands in the DB is invisible for as long as the
/// current sync runs — and an IMAP account only syncs on IDLE notifications, so
/// "as long as" can mean "until new mail happens to arrive". The in-flight run
/// is therefore asked to stop at its next batch boundary (already-downloaded
/// emails are kept) and a replacement sync is queued behind it, which reads the
/// new range when it starts.
#[tauri::command]
pub async fn update_account_sync_from(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    sync_from_timestamp: Option<i64>,
) -> Result<Account, AppError> {
    let (account, change) = services::accounts::update_account_sync_from(&state.db, &account_id, sync_from_timestamp)?;

    if change.changed {
        services::emails::request_sync_abort(&state.sync_abort_flags, &account_id);
        crate::commands::emails::enqueue_account_sync_with_contention(
            &app,
            &state,
            account_id,
            services::emails::SyncContention::Wait,
        )
        .await;
    }

    Ok(account)
}

/// Rename an account. The name is the sender name on mail sent from it.
#[tauri::command]
pub async fn update_account_name(
    state: State<'_, AppState>,
    account_id: String,
    name: String,
) -> Result<Account, AppError> {
    services::accounts::update_account_name(&state.db, &account_id, &name)
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
