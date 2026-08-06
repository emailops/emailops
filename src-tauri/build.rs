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
    // Google issues a *separate*, iOS-type client for mobile: it is a public
    // client (PKCE, no secret) and its redirect is the reversed client ID as a
    // custom URI scheme, not a loopback port. The desktop client cannot be
    // reused — its registered redirect is loopback, and embedding the desktop
    // client_secret in an App Store binary would ship an extractable secret.
    "EMAILOPS_GMAIL_IOS_CLIENT_ID",
    "EMAILOPS_OUTLOOK_CLIENT_ID",
    "EMAILOPS_OUTLOOK_CLIENT_SECRET",
];

fn main() {
    println!("cargo:rerun-if-changed=../.env.local");
    println!("cargo:rerun-if-changed=../.env");

    for key in OAUTH_ENV_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        match resolve_build_env(key) {
            Some(value) => println!("cargo:rustc-env={key}={value}"),
            // Warn at build time, where it is one line to fix, instead of
            // letting it surface as a failed sign-in on a device an hour later.
            // Not an error: a headless or CI build legitimately has no secrets.
            None => println!("cargo:warning={key} is not set — sign-in with that provider will fail at runtime"),
        }
    }

    emit_git_build_metadata();
    request_common_controls_v6_for_tests();

    // `tauri-build` is an optional build-dependency, enabled only by the `desktop`
    // feature. Headless builds (`--no-default-features`) skip it entirely: there is
    // no tauri.conf.json to process, no capabilities to compile, and no context to
    // generate. Cargo compiles build scripts with the package's feature cfgs, so a
    // plain `#[cfg]` is enough here.
    #[cfg(feature = "desktop")]
    tauri_build::build();
}

/// Make Windows test binaries load comctl32 **version 6**.
///
/// `rfd` (pulled in by `tauri-plugin-dialog`) imports `TaskDialogIndirect`,
/// which only version 6 exports. Windows picks the version from a side-by-side
/// manifest: without one it loads the version 5 in System32, the import cannot
/// be resolved, and the process dies at load time with
/// STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) before `main` runs — which is
/// exactly what `cargo test` hit on windows-msvc.
///
/// The app binary is unaffected because `tauri_build::build()` embeds a
/// manifest into it. Cargo's test harnesses get no such manifest, so the
/// dependency is requested here via the linker instead.
///
/// Uses the blanket `rustc-link-arg` rather than `rustc-link-arg-tests`: cargo
/// scopes the latter to `tests/*.rs` integration targets, and the binary that
/// actually failed is the *lib* unit-test harness, which it does not cover.
/// The blanket form therefore also reaches the app binary and the cdylib —
/// harmless, because requesting a side-by-side dependency that the embedded
/// manifest already declares is idempotent.
fn request_common_controls_v6_for_tests() {
    let is_windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let is_msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !(is_windows && is_msvc) {
        return;
    }

    // Passed to link.exe as a single argument, so the value carries no shell
    // quoting of its own — only the inner single quotes the option's grammar
    // requires.
    println!(
        "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' \
         name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
         processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
    );
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
        .or_else(|| primary_worktree_root().and_then(|root| load_env_file(root.join(".env.local"), key)))
        .or_else(|| primary_worktree_root().and_then(|root| load_env_file(root.join(".env"), key)))
}

/// Root of the primary worktree, when this build is running inside a linked
/// worktree — otherwise `None`.
///
/// `.env.local` holds the OAuth client ids and is gitignored, so a `git
/// worktree` never has its own copy. Without this fallback every build from a
/// worktree compiles with no Gmail client id and fails at runtime with
/// "missing Gmail OAuth client ID — set EMAILOPS_GMAIL_CLIENT_ID", which looks
/// like a configuration mistake rather than a missing file one directory up.
///
/// A worktree's `.git` is a *file* containing `gitdir: <primary>/.git/worktrees/<name>`,
/// so the primary root is that path with the trailing `/worktrees/<name>` and
/// `/.git` removed. Parsed directly rather than shelling out to `git`: build
/// scripts run on every compile and this needs no subprocess.
fn primary_worktree_root() -> Option<PathBuf> {
    let contents = fs::read_to_string("../.git").ok()?;
    let gitdir = contents.strip_prefix("gitdir:")?.trim();
    let path = PathBuf::from(gitdir);
    // <primary>/.git/worktrees/<name> -> <primary>
    let worktrees = path.parent()?;
    let dot_git = worktrees.parent()?;
    if dot_git.file_name()? != ".git" {
        return None;
    }
    dot_git.parent().map(PathBuf::from)
}

fn load_env_file(path: impl Into<PathBuf>, key: &str) -> Option<String> {
    let contents = fs::read_to_string(path.into()).ok()?;

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
