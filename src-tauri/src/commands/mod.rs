// The `unwrap_used` / `expect_used` deny is set crate-wide in `lib.rs`.

pub mod accounts;
pub mod ai_config;
pub mod ai_models;
pub mod attachments;
pub mod calendar;
pub mod chat;
pub mod classification;
pub mod connectivity;
pub mod contacts;
pub mod dashboard;
pub mod drafts;
pub mod emails;
pub mod filters;
pub mod junk;
pub mod lenses;
pub mod memory;
pub mod outbox;
pub mod preferences;
pub mod prompts;
pub mod search;
pub mod security;
pub mod sender_controls;
pub mod shared_docs;
pub mod skills;
pub mod system;
pub mod translation;
pub mod trusted_senders;

/// Architectural guard: every Tauri command is `async`.
///
/// Tauri runs a command declared without `async` on the main thread, the one
/// that drives the webview, so any database read it makes freezes the UI for
/// its full duration. `get_email_count` did that: in the All-accounts view its
/// thread-dedup count takes ~0.2 s on a large mailbox, and it ran on every
/// list refresh. An `async` command runs on the async runtime instead.
///
/// Whether a command runs on the main thread is decided by how it is declared,
/// not by anything observable at runtime from a test, so the rule is checked
/// where it lives: in the command sources.
#[cfg(test)]
mod architecture_tests {
    use std::path::{Path, PathBuf};

    fn command_sources() -> Vec<PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("read src/commands")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "rs"))
            .collect();
        files.sort();
        files
    }

    /// The `fn` lines of every `#[tauri::command]` in `source` that are not
    /// declared `async`.
    fn sync_commands(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut in_command = false;
        for line in source.lines() {
            let code = line.trim();
            if code.starts_with("#[tauri::command") {
                in_command = true;
                continue;
            }
            if !in_command || code.is_empty() || code.starts_with("//") || code.starts_with("#[") {
                continue;
            }
            if !code.contains("async fn") {
                found.push(code.to_string());
            }
            in_command = false;
        }
        found
    }

    #[test]
    fn the_scanner_flags_a_sync_command() {
        let source = "#[tauri::command]\npub fn get_count(state: State<'_, AppState>) -> Result<i32, AppError> {";
        assert_eq!(sync_commands(source).len(), 1);
    }

    #[test]
    fn the_scanner_accepts_an_async_command_behind_other_attributes() {
        let source = "#[tauri::command]\n#[allow(clippy::too_many_arguments)]\n/// docs\npub async fn get_count() {}";
        assert!(sync_commands(source).is_empty());
    }

    #[test]
    fn every_tauri_command_is_async() {
        let files = command_sources();
        assert!(!files.is_empty(), "found no sources under src/commands");

        let mut violations = Vec::new();
        for path in files {
            // This file holds the guard, not commands.
            if path.file_name().is_some_and(|n| n == "mod.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read command source");
            for code in sync_commands(&source) {
                violations.push(format!("{}: {code}", path.display()));
            }
        }
        assert!(
            violations.is_empty(),
            "Tauri commands must be `async` so they never block the UI thread:\n{}",
            violations.join("\n")
        );
    }
}
