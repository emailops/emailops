//! Host capability probes used by the onboarding wizard.
//!
//! The first-run flow needs to know whether the user's machine can run AI
//! locally so we can pre-select "Use AI" on Apple Silicon and "Plain email
//! client" everywhere else. Detection lives in Rust because Tauri's webview
//! does not expose CPU architecture reliably.

use tauri::State;

use crate::models::error::AppError;
use crate::services::updates::UpdateAvailableEvent;
use crate::AppState;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCapability {
    /// True when running on macOS with an Apple Silicon CPU (aarch64).
    /// This is the only configuration where local llama.cpp / Ollama gets
    /// a meaningful Metal acceleration boost out of the box.
    pub apple_silicon: bool,
    /// e.g. "macos", "linux", "windows".
    pub os: String,
    /// e.g. "aarch64", "x86_64".
    pub arch: String,
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

#[tauri::command]
pub async fn detect_ai_capability() -> Result<AiCapability, AppError> {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let apple_silicon = os == "macos" && arch == "aarch64";
    Ok(AiCapability {
        apple_silicon,
        os,
        arch,
    })
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

    #[tokio::test]
    async fn capability_reports_current_target() {
        let cap = detect_ai_capability().await.expect("detect ok");
        // Self-consistent on whatever host runs the test: apple_silicon iff
        // (macos && aarch64). We can't assert a specific platform.
        let expected = std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64";
        assert_eq!(cap.apple_silicon, expected);
        assert_eq!(cap.os, std::env::consts::OS);
        assert_eq!(cap.arch, std::env::consts::ARCH);
    }
}
