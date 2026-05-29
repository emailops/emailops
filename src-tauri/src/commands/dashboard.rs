//! Tauri command surface for the developer-facing dashboard.
//!
//! Three commands:
//! - `get_dashboard_stats` aggregates per-account counts in one call.
//! - `refresh_server_total` fetches the provider-side message count for one
//!   account and caches it in `user_preferences`.
//! - `get_queue_state` snapshots the in-memory state of the three task queues
//!   so the UI can show currently-running and pending tasks.

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::models::error::{AppError, Result};
use crate::services::dashboard::{self, AccountDashboard, ServerTotalCache};
use crate::services::storage_stats::{self, StorageStats};
use crate::services::task_queue::QueueStateSnapshot;
use crate::AppState;

fn emit_log(_app: &AppHandle, level: &str, message: &str) {
    crate::services::logger::log(level, "dashboard", message);
}

#[tauri::command]
pub async fn get_dashboard_stats(state: State<'_, AppState>) -> Result<Vec<AccountDashboard>> {
    dashboard::collect_dashboards(&state.db)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshServerTotalResponse {
    /// `None` when the provider does not (yet) expose a total — IMAP and
    /// Outlook fall through to this in v1.
    pub count: Option<i64>,
    pub fetched_at: Option<i64>,
}

#[tauri::command]
pub async fn refresh_server_total(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<RefreshServerTotalResponse> {
    let account = state
        .db
        .get_account(&account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account not found: {account_id}")))?;

    emit_log(
        &app,
        "info",
        &format!("Refreshing server total for {} ({})", account.email, account.provider),
    );

    let count: Option<i64> = match account.provider.as_str() {
        "gmail" => {
            // GmailClient transparently refreshes its access token on 401, so
            // we don't need to pre-refresh through services::emails::build_provider.
            // We construct it directly because `get_messages_total` is a
            // Gmail-only method that isn't exposed on the EmailProvider trait.
            let tokens = crate::services::accounts::get_tokens(&account.id)?;
            let client = crate::sync::gmail::GmailClient::new(
                tokens.access_token,
                tokens.refresh_token,
                Some(app.clone()),
                Some(account.id.clone()),
            );
            client.get_messages_total().await?
        }
        // IMAP / Outlook: not wired in v1 — return None so the UI shows "—".
        _ => None,
    };

    let fetched_at = chrono::Utc::now().timestamp();
    if let Some(c) = count {
        let cache = ServerTotalCache { count: c, fetched_at };
        dashboard::write_server_total_cache(&state.db, &account.id, &cache)?;
        emit_log(&app, "success", &format!("Server total for {}: {}", account.email, c));
        Ok(RefreshServerTotalResponse {
            count: Some(c),
            fetched_at: Some(fetched_at),
        })
    } else {
        emit_log(
            &app,
            "info",
            &format!(
                "Provider {} does not expose a server total (account {})",
                account.provider, account.email
            ),
        );
        Ok(RefreshServerTotalResponse {
            count: None,
            fetched_at: None,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllQueuesState {
    pub ai: QueueStateSnapshot,
    pub ai_background: QueueStateSnapshot,
    pub db: QueueStateSnapshot,
    pub sync: QueueStateSnapshot,
}

#[tauri::command]
pub async fn get_queue_state(state: State<'_, AppState>) -> Result<AllQueuesState> {
    Ok(AllQueuesState {
        ai: state.ai_queue.snapshot(),
        ai_background: state.ai_background.snapshot(),
        db: state.db_queue.snapshot(),
        sync: state.sync_queue_snapshot(),
    })
}

/// Snapshot of EmailOps' on-disk footprint. Pure filesystem scan — no SQL
/// is executed — so this is safe to call inline on dashboard mount.
#[tauri::command]
pub async fn get_storage_stats(app: AppHandle, state: State<'_, AppState>) -> Result<StorageStats> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::IoError(format!("resolve app_data_dir: {e}")))?;
    storage_stats::collect_storage_stats(&app_data_dir, state.db.db_path())
}
