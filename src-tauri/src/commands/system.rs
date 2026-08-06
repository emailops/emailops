//! Host capability probes used by the onboarding wizard.
//!
//! The first-run flow needs to know whether the user's machine can run AI
//! locally so it can pre-select "Use AI" or "Plain email client". Detection
//! lives in Rust because Tauri's webview does not expose CPU architecture or
//! physical memory reliably.
//!
//! This used to key entirely off `apple_silicon`, which meant every Linux and
//! Windows machine — including a 64 GB workstation with a discrete GPU — was
//! defaulted to the no-AI client. Capability is now decided by whether the
//! machine has enough RAM for the smallest chat model in the catalog, which
//! applies equally to all three platforms.

use tauri::State;

use crate::ai::model_catalog::{ModelKind, CATALOG};
use crate::models::error::AppError;
use crate::services::updates::UpdateAvailableEvent;
use crate::AppState;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCapability {
    /// True when running on macOS with an Apple Silicon CPU (aarch64).
    ///
    /// Retained because it still drives Metal-specific copy, but it is no
    /// longer the signal for "can this machine do local AI" — use
    /// [`AiCapability::local_ai_capable`] for that.
    pub apple_silicon: bool,
    /// True when the machine has enough RAM to run the smallest local chat
    /// model in the catalog AND this build actually contains the embedded
    /// runtime. This is what onboarding should branch on.
    pub local_ai_capable: bool,
    /// Whether the embedded llama.cpp runtime is compiled into this binary
    /// **and** can actually execute on this machine.
    ///
    /// Two ways to be false. The build may omit it (`--no-default-features` —
    /// CI packaging artifacts), in which case offering the option produces
    /// confusing Ollama connection errors instead. Or the host may be unable to
    /// run it: on an Intel Mac the Metal backend is compiled in but has no
    /// Apple7-family GPU to run on, so every decode fails. Either way the UI
    /// must not offer embedded AI — which is why this is one flag, not two.
    pub embedded_ai_available: bool,
    /// Physical RAM in whole GiB, or 0 when the probe failed.
    pub total_ram_gb: u64,
    /// RAM the smallest catalog chat model needs, so the UI can explain *why*
    /// local AI is unavailable instead of just hiding the option.
    pub min_ram_gb_for_local_ai: u64,
    /// e.g. "macos", "linux", "windows".
    pub os: String,
    /// e.g. "aarch64", "x86_64".
    pub arch: String,
}

/// RAM required by the least demanding chat model in the catalog.
///
/// Derived rather than hardcoded so adding a smaller model automatically
/// lowers the bar, and retiring the smallest one automatically raises it.
fn min_chat_model_ram_gb() -> u64 {
    CATALOG
        .iter()
        .filter(|m| matches!(m.kind, ModelKind::Chat))
        .map(|m| m.min_ram_gb as u64)
        .min()
        // A catalog with no chat models is not a real configuration, but
        // falling back to the historical 8 GB floor beats reporting 0 (which
        // would claim every machine is capable).
        .unwrap_or(8)
}

/// Pure decision: can this machine plausibly run a local chat model?
///
/// `total_ram_gb` is floor-rounded, and firmware plus integrated-GPU
/// reservations routinely shave a few hundred MB off nominal RAM — a nominal
/// 8 GB laptop commonly reports 7. One GiB of slack keeps those machines on the
/// capable side of a threshold they nominally meet.
pub fn ai_capability_from(
    os: &str,
    arch: &str,
    total_ram_gb: u64,
    min_chat_ram_gb: u64,
    embedded_ai_available: bool,
) -> AiCapability {
    // A 32-bit process cannot map multi-gigabyte weights however much RAM the
    // box reports.
    let address_space_ok = matches!(arch, "aarch64" | "x86_64" | "riscv64" | "loongarch64");
    let enough_ram = total_ram_gb + 1 >= min_chat_ram_gb;

    // Compiled in is not the same as runnable: the universal macOS bundle ships
    // a Metal-backed runtime in its x86_64 slice that an Intel Mac cannot
    // execute. `embedded_runtime_supported` owns that rule so this probe and
    // the provider loader in `services::ai` cannot drift apart.
    let runtime_runs_here = embedded_ai_available && crate::ai::gpu_plan::embedded_runtime_supported(os, arch);

    AiCapability {
        apple_silicon: os == "macos" && arch == "aarch64",
        local_ai_capable: runtime_runs_here && address_space_ok && enough_ram,
        embedded_ai_available: runtime_runs_here,
        total_ram_gb,
        min_ram_gb_for_local_ai: min_chat_ram_gb,
        os: os.to_string(),
        arch: arch.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    /// Package version, e.g. "0.6.2".
    pub version: String,
    /// Git short sha the binary was built from. `None` when built outside a
    /// git checkout (e.g. from a source tarball).
    pub commit: Option<String>,
    /// True when the built commit is tagged `v{version}` — i.e. this binary
    /// corresponds to a published release rather than a local/dev build.
    pub is_release: bool,
}

/// Pure decision: derive the displayable build identity from compile-time git
/// metadata. `tags_at_head` is the comma-separated output of
/// `git tag --points-at HEAD` embedded by build.rs.
pub fn build_info_from(version: &str, git_sha: &str, tags_at_head: &str) -> BuildInfo {
    let release_tag = format!("v{version}");
    let is_release = tags_at_head.split(',').any(|t| t.trim() == release_tag);
    BuildInfo {
        version: version.to_string(),
        commit: (!git_sha.is_empty()).then(|| git_sha.to_string()),
        is_release,
    }
}

/// Build identity for the sidebar version label. Values are embedded at
/// compile time by build.rs, so this never touches the filesystem or git.
#[tauri::command]
pub async fn get_build_info() -> Result<BuildInfo, AppError> {
    Ok(build_info_from(
        env!("CARGO_PKG_VERSION"),
        option_env!("EMAILOPS_GIT_SHA").unwrap_or(""),
        option_env!("EMAILOPS_GIT_TAGS").unwrap_or(""),
    ))
}

/// Latest-known newer release, derived from the prefs the daily update check
/// persists. Backs the persistent sidebar download link, which — unlike the
/// once-per-version `app-update-available` toast — must survive restarts.
#[tauri::command]
pub async fn get_available_update(state: State<'_, AppState>) -> Result<Option<UpdateAvailableEvent>, AppError> {
    Ok(crate::services::updates::available_update(
        &state.db,
        env!("CARGO_PKG_VERSION"),
    ))
}

/// Whether the running process is Rosetta-translated, for the feedback form's
/// technical-info line. See [`crate::util::system::is_rosetta_translated`] —
/// without it, `x86_64` in a bug report cannot be told apart from an Intel Mac.
#[tauri::command]
pub async fn is_rosetta_translated() -> Result<bool, AppError> {
    Ok(crate::util::system::is_rosetta_translated())
}

#[tauri::command]
pub async fn detect_ai_capability() -> Result<AiCapability, AppError> {
    Ok(ai_capability_from(
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::util::system::total_ram_gb(),
        min_chat_model_ram_gb(),
        cfg!(feature = "llamacpp"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_info_from ───────────────────────────────────────────────────────

    #[test]
    fn release_when_head_is_tagged_with_the_package_version() {
        let info = build_info_from("0.6.2", "05ae613", "v0.6.2");
        assert!(info.is_release);
        assert_eq!(info.version, "0.6.2");
    }

    #[test]
    fn release_when_the_version_tag_is_one_of_several_tags_at_head() {
        let info = build_info_from("0.6.2", "05ae613", "nightly,v0.6.2,tested");
        assert!(info.is_release);
    }

    #[test]
    fn non_release_when_head_has_no_tags() {
        let info = build_info_from("0.6.2", "05ae613", "");
        assert!(!info.is_release);
        assert_eq!(info.commit.as_deref(), Some("05ae613"));
    }

    #[test]
    fn non_release_when_head_tag_does_not_match_the_package_version() {
        let info = build_info_from("0.6.3", "05ae613", "v0.6.2");
        assert!(!info.is_release);
    }

    #[test]
    fn missing_git_metadata_yields_no_commit() {
        // Built outside a git checkout (e.g. source tarball): no sha to show.
        let info = build_info_from("0.6.2", "", "");
        assert_eq!(info.commit, None);
    }

    #[test]
    fn tag_matching_ignores_surrounding_whitespace() {
        let info = build_info_from("0.6.2", "05ae613", " v0.6.2 ");
        assert!(info.is_release);
    }

    // ── ai_capability_from ────────────────────────────────────────────────────

    #[test]
    fn capability_is_decided_by_ram_on_every_platform() {
        // The regression this whole change exists for: a well-specced Linux or
        // Windows box used to be reported incapable purely because it was not a
        // Mac. (os, arch, ram_gb, expected_capable, label)
        let cases: &[(&str, &str, u64, bool, &str)] = &[
            ("macos", "aarch64", 16, true, "Apple Silicon with headroom"),
            // Not a RAM decision — see `an_intel_mac_can_never_run_the_embedded_runtime`.
            ("macos", "x86_64", 16, false, "Intel Mac, however much RAM"),
            ("linux", "x86_64", 64, true, "Linux workstation"),
            ("windows", "x86_64", 32, true, "Windows desktop"),
            ("linux", "aarch64", 16, true, "ARM Linux"),
            ("windows", "aarch64", 16, true, "ARM Windows"),
            ("linux", "x86_64", 4, false, "too little RAM to run anything"),
            (
                "macos",
                "aarch64",
                4,
                false,
                "Apple Silicon is not exempt from the RAM floor",
            ),
        ];
        for (os, arch, ram, want, label) in cases {
            let cap = ai_capability_from(os, arch, *ram, 8, true);
            assert_eq!(cap.local_ai_capable, *want, "{label}");
        }
    }

    #[test]
    fn a_build_without_the_embedded_runtime_is_never_capable() {
        // The regression behind "AI warmup failed … Ollama warmup failed" on a
        // machine the user had configured for embedded AI: the binary was
        // compiled with --no-default-features, so there is no local runtime at
        // any RAM size, and the saved provider silently fell through to Ollama.
        for ram in [8, 16, 64, 256] {
            let cap = ai_capability_from("windows", "x86_64", ram, 8, false);
            assert!(
                !cap.local_ai_capable,
                "{ram} GB cannot help when the runtime is not compiled in"
            );
            assert!(!cap.embedded_ai_available);
        }

        // Even an Apple Silicon Mac — the historical "always capable" case.
        let cap = ai_capability_from("macos", "aarch64", 64, 8, false);
        assert!(!cap.local_ai_capable);
        assert!(cap.apple_silicon, "hardware facts stay truthful");
    }

    #[test]
    fn availability_is_reported_separately_so_the_ui_can_explain_why() {
        // Plenty of RAM but no runtime vs. runtime present but too little RAM
        // are different problems and need different copy.
        let no_runtime = ai_capability_from("windows", "x86_64", 64, 8, false);
        assert!(!no_runtime.embedded_ai_available);
        assert!(no_runtime.total_ram_gb >= no_runtime.min_ram_gb_for_local_ai);

        let too_small = ai_capability_from("windows", "x86_64", 4, 8, true);
        assert!(too_small.embedded_ai_available);
        assert!(!too_small.local_ai_capable);
    }

    #[test]
    fn thirty_two_bit_targets_are_never_capable() {
        // Address space, not RAM, is the binding constraint here.
        for arch in ["x86", "arm", "mips"] {
            let cap = ai_capability_from("linux", arch, 64, 8, true);
            assert!(!cap.local_ai_capable, "{arch} cannot map multi-GB weights");
        }
    }

    #[test]
    fn nominal_ram_just_under_the_threshold_still_counts() {
        // A nominal 8 GB machine reports 7 GiB after firmware/iGPU reservation.
        let cap = ai_capability_from("windows", "x86_64", 7, 8, true);
        assert!(cap.local_ai_capable, "1 GiB of slack must absorb the reservation");

        // But the slack is exactly one GiB — 6 is genuinely too little.
        let cap = ai_capability_from("windows", "x86_64", 6, 8, true);
        assert!(!cap.local_ai_capable);
    }

    #[test]
    fn a_failed_ram_probe_reports_incapable_rather_than_capable() {
        // `total_ram_gb()` returns 0 when the probe fails; defaulting to
        // "capable" would offer a download that cannot possibly run.
        let cap = ai_capability_from("linux", "x86_64", 0, 8, true);
        assert!(!cap.local_ai_capable);
        assert_eq!(cap.total_ram_gb, 0);
    }

    #[test]
    fn an_intel_mac_can_never_run_the_embedded_runtime() {
        // The bug this guards: the macOS bundle is universal, so its x86_64
        // slice carries llama.cpp *with* Metal — `llama-cpp-sys-2` only
        // disables Metal for watchOS, not for Intel. On an Intel Mac the GPU is
        // not an Apple7 family device, so the very first prefill fails with an
        // opaque `Decode Error -3` (GGML_STATUS_FAILED) on every single turn.
        // RAM is irrelevant, and the CPU fallback is too slow to ship, so the
        // honest answer is "not available here".
        for ram in [8, 16, 64, 256] {
            let cap = ai_capability_from("macos", "x86_64", ram, 8, true);
            assert!(!cap.local_ai_capable, "{ram} GB does not make Metal work on Intel");
            assert!(
                !cap.embedded_ai_available,
                "{ram} GB: the runtime is compiled in but unusable — the UI must not offer it"
            );
        }

        // The same binary on the Apple Silicon slice is unaffected.
        let cap = ai_capability_from("macos", "aarch64", 16, 8, true);
        assert!(cap.local_ai_capable);
        assert!(cap.embedded_ai_available);

        // And x86_64 is still fine everywhere Metal is not involved.
        for os in ["linux", "windows"] {
            assert!(
                ai_capability_from(os, "x86_64", 16, 8, true).local_ai_capable,
                "{os} x86_64 has a working CPU/Vulkan/CUDA path"
            );
        }
    }

    #[test]
    fn apple_silicon_still_reported_for_metal_specific_copy() {
        assert!(ai_capability_from("macos", "aarch64", 16, 8, true).apple_silicon);
        assert!(!ai_capability_from("macos", "x86_64", 16, 8, true).apple_silicon);
        assert!(!ai_capability_from("linux", "aarch64", 16, 8, true).apple_silicon);
    }

    #[test]
    fn the_threshold_is_surfaced_so_the_ui_can_explain_itself() {
        let cap = ai_capability_from("linux", "x86_64", 4, 8, true);
        assert_eq!(cap.min_ram_gb_for_local_ai, 8);
        assert_eq!(cap.total_ram_gb, 4);
    }

    #[test]
    fn threshold_tracks_the_smallest_chat_model_in_the_catalog() {
        let expected = CATALOG
            .iter()
            .filter(|m| matches!(m.kind, ModelKind::Chat))
            .map(|m| m.min_ram_gb as u64)
            .min()
            .expect("catalog has chat models");
        assert_eq!(min_chat_model_ram_gb(), expected);
        // Embedding models are far smaller; they must not drag the floor down,
        // since being able to embed is not being able to chat.
        assert!(min_chat_model_ram_gb() > 1);
    }

    #[tokio::test]
    async fn capability_reports_current_target() {
        let cap = detect_ai_capability().await.expect("detect ok");
        // Self-consistent on whatever host runs the test: apple_silicon iff
        // (macos && aarch64). We can't assert a specific platform.
        let expected = std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64";
        assert_eq!(cap.apple_silicon, expected);
        assert_eq!(cap.os, std::env::consts::OS);
        assert_eq!(cap.arch, std::env::consts::ARCH);
        assert_eq!(cap.min_ram_gb_for_local_ai, min_chat_model_ram_gb());
        // Availability is "compiled in AND runnable here", so an Intel-mac host
        // reports false even from a build that does carry the runtime.
        let metal_ok = std::env::consts::OS != "macos" || std::env::consts::ARCH == "aarch64";
        assert_eq!(
            cap.embedded_ai_available,
            cfg!(feature = "llamacpp") && metal_ok,
            "availability must reflect how this binary was built AND where it is running"
        );
    }
}
