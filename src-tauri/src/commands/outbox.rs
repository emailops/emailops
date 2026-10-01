//! Outbox commands: undo send and scheduled send. Thin wrappers over
//! `services::outbox`.

use tauri::{AppHandle, State};

use crate::models::error::AppError;
use crate::models::outbox::{OutboxEntry, OutboxSchedule, OutgoingMessage};
use crate::services::{clock, emails, outbox};
use crate::sync::provider::provider_supports_drafts;
use crate::AppState;

/// Queue a composed message to go out when its undo window closes
/// (`schedule = {type: "undo", delaySecs}`) or at a chosen time
/// (`{type: "at", sendAt}`). `draft_id` is the composer's saved draft, which
/// leaves Drafts once the message is queued (its file attachments are read
/// into the queued message first).
#[tauri::command]
pub async fn queue_outgoing_email(
    app: AppHandle,
    state: State<'_, AppState>,
    message: OutgoingMessage,
    schedule: OutboxSchedule,
    draft_id: Option<String>,
) -> Result<OutboxEntry, AppError> {
    let db = state.db.clone();
    Box::pin(async move {
        // The provider is only needed to remove the draft's provider copy;
        // without one (offline) the local draft still goes.
        let draft_account = match draft_id.as_deref() {
            Some(id) => match db.get_draft(id)? {
                Some(draft) => db.get_account(&draft.account_id)?,
                None => None,
            },
            None => None,
        };
        let provider = match draft_account {
            Some(account) if provider_supports_drafts(&account.provider) => {
                match emails::build_provider(&account, Some(app)).await {
                    Ok(provider) => Some(provider),
                    Err(e) => {
                        crate::services::logger::log(
                            "debug",
                            "drafts",
                            format!("Removing the queued message's draft locally only — provider unavailable: {e}"),
                        );
                        None
                    }
                }
            }
            _ => None,
        };
        outbox::queue_outgoing(
            &db,
            message,
            schedule,
            draft_id.as_deref(),
            provider.as_deref(),
            clock::now_secs(),
        )
        .await
    })
    .await
}

/// Take a waiting or failed message out of the outbox (undo, edit, delete) and
/// return it so the composer can reopen with it. Fails with
/// `outbox_not_pending` once it is being sent.
#[tauri::command]
pub async fn cancel_outbox_message(state: State<'_, AppState>, id: String) -> Result<OutgoingMessage, AppError> {
    outbox::cancel_outbox_message(&state.db, &id, clock::now_secs())
}

/// Send a waiting message now, or retry a failed one.
#[tauri::command]
pub async fn send_outbox_message_now(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    outbox::send_outbox_message_now(&state.db, &id, clock::now_secs())
}

/// Waiting and failed messages of one account, or of every enabled account.
#[tauri::command]
pub async fn list_outbox(state: State<'_, AppState>, account_id: Option<String>) -> Result<Vec<OutboxEntry>, AppError> {
    outbox::list_outbox(&state.db, account_id.as_deref())
}
