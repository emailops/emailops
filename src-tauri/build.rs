use std::env;
use std::fs;
use std::path::PathBuf;

// Keys forwarded from `.env.local` / `.env` / process env into compile-time
// `option_env!` constants in `src/sync/oauth.rs`. The Outlook secret is
// included so confidential-client setups can bundle it, but it is optional —
// Azure AD public-client (native app) registrations use PKCE and intentionally
// reject any client_secret. Missing/empty values are simply skipped.
const OAUTH_ENV_KEYS: &[&str] = &[
    "EMAILOPS_GMAIL_CLIENT_ID",
    "EMAILOPS_GMAIL_CLIENT_SECRET",
    "EMAILOPS_OUTLOOK_CLIENT_ID",
    "EMAILOPS_OUTLOOK_CLIENT_SECRET",
];

fn main() {
    println!("cargo:rerun-if-changed=../.env.local");
    println!("cargo:rerun-if-changed=../.env");

    for key in OAUTH_ENV_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        if let Some(value) = resolve_build_env(key) {
            println!("cargo:rustc-env={key}={value}");
        }
    }

    emit_git_build_metadata();

    tauri_build::build()
}

/// Embed the git short sha and any tags pointing at HEAD so the app can show
/// which commit a non-release build came from (`get_build_info` command).
/// Best-effort: outside a git checkout both vars are empty and the version
/// label degrades to the bare version.
fn emit_git_build_metadata() {
    // HEAD changes on commit/checkout; the tag refs change when tagging.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/tags");

    let sha = git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let tags = git_output(&["tag", "--points-at", "HEAD"])
        .map(|out| out.split_whitespace().collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    println!("cargo:rustc-env=EMAILOPS_GIT_SHA={sha}");
    println!("cargo:rustc-env=EMAILOPS_GIT_TAGS={tags}");
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let trimmed = text.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn resolve_build_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| load_env_file("../.env.local", key))
        .or_else(|| load_env_file("../.env", key))
}

fn load_env_file(path: &str, key: &str) -> Option<String> {
    let path = PathBuf::from(path);
    let contents = fs::read_to_string(path).ok()?;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let (candidate_key, raw_value) = trimmed.split_once('=')?;
        if candidate_key.trim() != key {
            continue;
        }

        let value = raw_value.trim().trim_matches('"').trim_matches('\'').to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }

    None
}
