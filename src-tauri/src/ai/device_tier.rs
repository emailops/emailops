//! Which AI backends a device may actually use.
//!
//! Pure decision over facts the caller probes: the OS version, whether Apple
//! Intelligence is available, and how much memory this process may hold. No
//! I/O, no `cfg!` — the platform arrives as data, which is the whole point.
//!
//! `docs/DECISIONS.md` ("iOS targets iOS 26; on-device AI is capability-tiered,
//! not dropped") settles the policy this implements: devices with Apple
//! Intelligence run Apple's Foundation Models for short structured work
//! (classification, tags, junk, translation, per-email summaries) and may
//! additionally download a 3–4B GGUF for chat over retrieved threads; every
//! other device is remote-only, Ollama-on-LAN preferred, OpenRouter fallback.
//!
//! The pure-function requirement is not stylistic. The 2026-07-29 Linux/Windows
//! entry records the same lesson learned the hard way: capability was keyed off
//! `apple_silicon` inside a `cfg`, and every other machine silently defaulted to
//! the no-AI client with no test able to catch it.

use crate::ai::model_fit::{model_fit, ModelFit};

/// What this device can run locally. Two independent bits rather than one
/// enum tier: they come from different places and can disagree.
///
/// A user who switched Apple Intelligence off in Settings still has the RAM for
/// a GGUF, and a device with Apple Intelligence but a tight memory budget can
/// still classify mail on-device while chat goes remote. Collapsing them would
/// silently take away whichever half survived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceAiPlan {
    /// Apple's on-device model may serve short structured work.
    pub foundation_models: bool,
    /// A downloaded GGUF may serve chat over retrieved threads.
    pub local_chat: bool,
}

impl DeviceAiPlan {
    /// Nothing runs on this device — every AI feature needs Ollama-on-LAN or
    /// OpenRouter, both of which are explicit user choices.
    pub fn is_remote_only(self) -> bool {
        !self.foundation_models && !self.local_chat
    }
}

/// Minimum major OS version exposing the Foundation Models framework.
/// The build already targets it (`scripts/ios_patch_project.sh` pins the
/// deployment target), so a lower value means the caller probed something
/// unexpected — treated as "no Apple model" rather than trusted.
pub const FOUNDATION_MODELS_MIN_OS_MAJOR: u32 = 26;

/// Decide what may run on this device.
///
/// * `os_major` — the running OS's major version.
/// * `apple_intelligence` — whether the system reports its on-device model as
///   available *right now*. This is a runtime answer, not a device-model
///   lookup: it goes false when the user disables Apple Intelligence, while the
///   model is still downloading, or in an unsupported region.
/// * `available_memory` — bytes this process may hold (`os_proc_available_memory`
///   on iOS). `None` means the probe failed and is treated as a small device.
/// * `smallest_chat_model_bytes` — the smallest chat GGUF the catalog offers,
///   so the memory threshold stays in one place (`ai::model_fit`) instead of
///   being restated here and drifting.
pub fn plan_device_ai(
    os_major: u32,
    apple_intelligence: bool,
    available_memory: Option<u64>,
    smallest_chat_model_bytes: u64,
) -> DeviceAiPlan {
    let foundation_models = apple_intelligence && os_major >= FOUNDATION_MODELS_MIN_OS_MAJOR;

    // Reuses the catalog's own fit rule with a hard memory limit, which is what
    // iOS enforces: `min_ram_gb` is deliberately 0 because it describes a whole
    // machine and has no meaning against a per-process allowance.
    let local_chat = matches!(
        model_fit(smallest_chat_model_bytes, 0, available_memory, None, true),
        ModelFit::Fits | ModelFit::Tight
    );

    DeviceAiPlan {
        foundation_models,
        local_chat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;
    /// The catalog's smallest chat entry today (Qwen 3.5 4B Q4, ~3.0 GB).
    const SMALLEST_CHAT: u64 = 3 * GIB;

    #[test]
    fn a_current_iphone_gets_both_backends() {
        // iPhone 15 Pro and later: Apple Intelligence, and ~4-5 GB of process
        // allowance under the increased-memory-limit entitlement.
        let plan = plan_device_ai(26, true, Some(4 * GIB + GIB / 2), SMALLEST_CHAT);
        assert_eq!(
            plan,
            DeviceAiPlan {
                foundation_models: true,
                local_chat: true
            }
        );
        assert!(!plan.is_remote_only());
    }

    #[test]
    fn apple_intelligence_without_room_for_a_gguf_still_classifies_on_device() {
        // Apple's model is a system service and costs this process nothing, so
        // a tight memory budget must not take it away too.
        let plan = plan_device_ai(26, true, Some(2 * GIB), SMALLEST_CHAT);
        assert!(plan.foundation_models);
        assert!(!plan.local_chat);
        assert!(!plan.is_remote_only());
    }

    #[test]
    fn a_user_who_turned_apple_intelligence_off_keeps_local_chat() {
        // The switch is in iOS Settings and says nothing about RAM. Dropping to
        // remote-only here would quietly send mail off-device.
        let plan = plan_device_ai(26, false, Some(4 * GIB + GIB / 2), SMALLEST_CHAT);
        assert!(!plan.foundation_models);
        assert!(plan.local_chat);
    }

    #[test]
    fn an_older_os_never_gets_the_apple_model() {
        // Defensive: the deployment target already pins 26, so this means the
        // probe returned something unexpected. Believe the probe, not the build.
        let plan = plan_device_ai(25, true, Some(4 * GIB + GIB / 2), SMALLEST_CHAT);
        assert!(!plan.foundation_models);
        assert!(plan.local_chat);
    }

    #[test]
    fn a_small_device_is_remote_only() {
        let plan = plan_device_ai(26, false, Some(GIB), SMALLEST_CHAT);
        assert!(plan.is_remote_only());
    }

    #[test]
    fn a_failed_memory_probe_does_not_promise_local_chat() {
        // `os_proc_available_memory` returns 0 from contexts where it is
        // unsupported. Promising a 3 GB download there ends in a jetsam kill.
        let plan = plan_device_ai(26, true, None, SMALLEST_CHAT);
        assert!(!plan.local_chat);
        assert!(plan.foundation_models);
    }

    #[test]
    fn the_memory_threshold_tracks_the_catalog_rather_than_a_copy_of_it() {
        // Same device, two catalogs: a smaller smallest-model must be able to
        // flip local chat on. If this ever stops holding, the rule has been
        // duplicated away from `ai::model_fit`.
        let tight = Some(2 * GIB);
        assert!(!plan_device_ai(26, true, tight, 3 * GIB).local_chat);
        assert!(plan_device_ai(26, true, tight, GIB / 2).local_chat);
    }
}
