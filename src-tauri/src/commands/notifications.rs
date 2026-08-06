//! OS notification permission.
//!
//! Only meeting reminders notify today (`services::sync_scheduler`'s
//! meeting-notification loop). On desktop the plugin reports `Granted`
//! unconditionally and this is a no-op; on iOS nothing is ever delivered until
//! the user has answered the system prompt, and that prompt can only be raised
//! once — a denial is permanent until the user goes to Settings.
//!
//! So the request is deliberately *not* made at startup. The frontend calls
//! this when the user turns calendar reminders on, which is the moment the ask
//! explains itself: they have just asked to be reminded.

use tauri::AppHandle;
use tauri_plugin_notification::{NotificationExt, PermissionState};

use crate::models::error::AppError;

/// Stable wire value for a permission state.
///
/// Pure, and separate from the command, so the mapping the frontend branches on
/// is pinned by a test rather than by `Debug` formatting that a Tauri upgrade
/// could rename underneath us.
pub fn permission_label(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        PermissionState::Prompt | PermissionState::PromptWithRationale => "prompt",
    }
}

/// Whether the system prompt still has to be raised.
///
/// `PromptWithRationale` is Android's "you already declined once, explain
/// yourself" state; it is still a request we are allowed to make.
pub fn needs_request(state: PermissionState) -> bool {
    matches!(state, PermissionState::Prompt | PermissionState::PromptWithRationale)
}

/// Ask for notification permission if the system has not decided yet, and
/// report where things stand. Never raises the prompt twice.
#[tauri::command]
pub async fn ensure_notification_permission(app: AppHandle) -> Result<String, AppError> {
    let notifier = app.notification();
    let state = notifier
        .permission_state()
        .map_err(|e| AppError::IoError(format!("notification permission check failed: {e}")))?;

    let state = if needs_request(state) {
        notifier
            .request_permission()
            .map_err(|e| AppError::IoError(format!("notification permission request failed: {e}")))?
    } else {
        state
    };

    Ok(permission_label(state).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_stable_wire_values() {
        assert_eq!(permission_label(PermissionState::Granted), "granted");
        assert_eq!(permission_label(PermissionState::Denied), "denied");
        assert_eq!(permission_label(PermissionState::Prompt), "prompt");
        // Android's variant collapses into "prompt": the frontend's only
        // question is whether it may still be asked.
        assert_eq!(permission_label(PermissionState::PromptWithRationale), "prompt");
    }

    #[test]
    fn only_undecided_states_are_requested() {
        assert!(needs_request(PermissionState::Prompt));
        assert!(needs_request(PermissionState::PromptWithRationale));
        // Asking again after a denial does nothing on iOS — the prompt is
        // one-shot — and would hide the real fix (Settings) behind a no-op.
        assert!(!needs_request(PermissionState::Denied));
        assert!(!needs_request(PermissionState::Granted));
    }
}
