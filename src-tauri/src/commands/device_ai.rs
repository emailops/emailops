//! What AI this device can run, for the settings UI and onboarding.
//!
//! Thin: probes Apple Intelligence, asks the memory probe, and hands both to
//! the pure planner in `ai::device_tier`. No policy lives here.

use serde::Serialize;
use tauri::State;

use crate::ai::device_tier::plan_device_ai;
use crate::ai::foundation_models::apple_intelligence_status;
use crate::ai::model_catalog::{ModelKind, CATALOG};
use crate::models::error::AppError;
use crate::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAiStatus {
    /// `available` | `deviceNotEligible` | `notEnabled` | `modelNotReady` |
    /// `unavailable` | `frameworkMissing`. The UI explains the difference:
    /// a downloading model is worth waiting for, ineligible hardware is not.
    pub apple_intelligence: String,
    /// Whether that state can still change without new hardware.
    pub apple_intelligence_transient: bool,
    /// Apple's model may serve short structured work (classification, tags,
    /// junk, translation, per-email summaries).
    pub foundation_models: bool,
    /// A downloaded GGUF may serve chat over retrieved threads.
    pub local_chat: bool,
    /// Neither runs here: every AI feature needs Ollama or OpenRouter.
    pub remote_only: bool,
    /// Bytes this process may hold, or 0 when the probe failed. Reported so
    /// the settings panel can explain *why* local chat is unavailable rather
    /// than just greying it out.
    pub available_memory_bytes: u64,
}

/// The smallest chat model the catalog offers, which is the threshold that
/// decides whether local chat is possible at all. Read from the catalog rather
/// than hard-coded so adding a smaller model automatically widens the tier.
fn smallest_chat_model_bytes() -> u64 {
    CATALOG
        .iter()
        .filter(|m| matches!(m.kind, ModelKind::Chat))
        .map(|m| m.size_bytes)
        .min()
        .unwrap_or(u64::MAX)
}

#[tauri::command]
pub async fn get_device_ai_status(_state: State<'_, AppState>) -> Result<DeviceAiStatus, AppError> {
    let status = apple_intelligence_status();
    let available_memory = crate::util::system::total_ram_bytes();
    let os_major = crate::util::system::os_major_version();

    let plan = plan_device_ai(
        os_major,
        status.is_available(),
        available_memory,
        smallest_chat_model_bytes(),
    );

    Ok(DeviceAiStatus {
        apple_intelligence: status.as_str().to_string(),
        apple_intelligence_transient: status.is_transient(),
        foundation_models: plan.foundation_models,
        local_chat: plan.local_chat,
        remote_only: plan.is_remote_only(),
        available_memory_bytes: available_memory.unwrap_or(0),
    })
}
