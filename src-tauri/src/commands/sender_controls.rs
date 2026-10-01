//! Block sender and one-click unsubscribe. Thin wrappers over
//! `services::sender_controls` and `services::unsubscribe`.

use tauri::{AppHandle, State};

use crate::models::error::AppError;
use crate::models::{Account, BlockedSender};
use crate::services::emails::{build_provider, ProviderAccess};
use crate::services::sender_controls::{self, SenderMoveReport, SenderStatus};
use crate::services::unsubscribe::{self, UnsubscribeKind};
use crate::sync::provider::{provider_supports_mailbox_writes, EmailProvider};
use crate::AppState;

fn account(state: &State<'_, AppState>, account_id: &str) -> Result<Account, AppError> {
    state
        .db
        .get_account(account_id)?
        .ok_or_else(|| AppError::NotFound(format!("Account {account_id} not found")))
}

/// The account's provider when it takes mailbox writes: `Ok(None)` for one
/// that does not, `Err` when it could not be built (offline, expired login).
async fn writable_provider(account: &Account, app: AppHandle) -> Result<Option<Box<dyn EmailProvider>>, AppError> {
    if !provider_supports_mailbox_writes(&account.provider) {
        return Ok(None);
    }
    build_provider(account, Some(app)).await.map(Some)
}

/// What the reading pane shows about a message's sender: blocked, how to
/// unsubscribe (derived — never the raw headers), already unsubscribed.
#[tauri::command]
pub async fn get_sender_status(
    state: State<'_, AppState>,
    account_id: String,
    email_id: String,
) -> Result<SenderStatus, AppError> {
    sender_controls::sender_status(&state.db, &account_id, &email_id)
}

/// Unsubscribe from the list a message came from (one-click POST or mailto;
/// a link the frontend already opened is only recorded). Contacts a third
/// party, so only ever called on an explicit, confirmed user action.
#[tauri::command]
pub async fn unsubscribe_from_sender(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: String,
    email_id: String,
) -> Result<UnsubscribeKind, AppError> {
    let account = account(&state, &account_id)?;
    let client = unsubscribe::one_click_client()?;
    // Only a mailto needs the provider; building it refreshes OAuth tokens, so
    // it is skipped for the other methods.
    let status = sender_controls::sender_status(&state.db, &account_id, &email_id)?;
    let provider = match status.unsubscribe.as_ref().map(|o| o.kind) {
        Some(UnsubscribeKind::Mailto) => Some(build_provider(&account, Some(app)).await?),
        _ => None,
    };
    let result = unsubscribe::unsubscribe(
        &state.db,
        &account,
        &email_id,
        &client,
        provider.as_deref(),
        crate::services::clock::now_secs(),
    )
    .await;
    if let Err(e) = &result {
        crate::services::logger::log(
            "error",
            "account",
            format!("[{}] Unsubscribe failed: {e}", account.email),
        );
    }
    result
}

/// Block a sender in one account; with `move_existing`, their inbox and
/// archived mail is marked junk and filed in Spam too.
#[tauri::command]
pub async fn block_sender(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: String,
    address: String,
    move_existing: bool,
) -> Result<SenderMoveReport, AppError> {
    let account = account(&state, &account_id)?;
    let now = crate::services::clock::now_secs();
    if !move_existing {
        return sender_controls::block_sender(&state.db, &account, &address, false, ProviderAccess::LocalOnly, now)
            .await;
    }
    match writable_provider(&account, app).await {
        Ok(Some(provider)) => {
            sender_controls::block_sender(
                &state.db,
                &account,
                &address,
                true,
                ProviderAccess::Ready(provider.as_ref()),
                now,
            )
            .await
        }
        Ok(None) => {
            sender_controls::block_sender(&state.db, &account, &address, true, ProviderAccess::LocalOnly, now).await
        }
        Err(e) => {
            sender_controls::block_sender(
                &state.db,
                &account,
                &address,
                true,
                ProviderAccess::Unreachable(&e),
                now,
            )
            .await
        }
    }
}

/// Unblock a sender; with `restore`, their mail in Spam comes back to the
/// inbox.
#[tauri::command]
pub async fn unblock_sender(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: String,
    address: String,
    restore: bool,
) -> Result<SenderMoveReport, AppError> {
    let account = account(&state, &account_id)?;
    if !restore {
        return sender_controls::unblock_sender(&state.db, &account, &address, false, ProviderAccess::LocalOnly).await;
    }
    match writable_provider(&account, app).await {
        Ok(Some(provider)) => {
            sender_controls::unblock_sender(
                &state.db,
                &account,
                &address,
                true,
                ProviderAccess::Ready(provider.as_ref()),
            )
            .await
        }
        Ok(None) => {
            sender_controls::unblock_sender(&state.db, &account, &address, true, ProviderAccess::LocalOnly).await
        }
        Err(e) => {
            sender_controls::unblock_sender(&state.db, &account, &address, true, ProviderAccess::Unreachable(&e)).await
        }
    }
}

/// Every blocked sender, or one account's.
#[tauri::command]
pub async fn list_blocked_senders(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<BlockedSender>, AppError> {
    state.db.list_blocked_senders(account_id.as_deref())
}
