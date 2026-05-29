//! Host capability probes used by the onboarding wizard.
//!
//! The first-run flow needs to know whether the user's machine can run AI
//! locally so we can pre-select "Use AI" on Apple Silicon and "Plain email
//! client" everywhere else. Detection lives in Rust because Tauri's webview
//! does not expose CPU architecture reliably.

use crate::models::error::AppError;

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
