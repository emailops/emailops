//! Per-process CLI session: the open database plus the resolved account/model
//! that every command (and every REPL turn) operates against.
//!
//! [`CliSession::bootstrap`] is the single place that wires the CLI into the
//! same seams the desktop app uses: it initialises the keychain, resolves the
//! data directory, opens the DB (running migrations), and installs the CLI's
//! [`Logger`](crate::services::logger) + [`EventSink`](crate::services::events)
//! backends. From that point on the command handlers call `services::*`
//! directly — no `AppHandle` required.

use std::path::PathBuf;
use std::sync::Arc;

use crate::db::Database;
use crate::models::error::{AppError, Result};

use super::output::CliLogger;
use super::{Cli, OutputMode};

/// State shared across one CLI invocation (one-shot) or one REPL session.
pub struct CliSession {
    pub db: Arc<Database>,
    /// Resolved account, if exactly one could be determined at bootstrap. `None`
    /// when there are zero or multiple enabled accounts and no `--account` hint;
    /// commands that need an account call [`CliSession::require_account`].
    pub account: Option<String>,
    pub model: String,
    pub mode: OutputMode,
    pub quiet: bool,
    /// Directory the DB lives in — needed by `sync_account`.
    pub data_dir: PathBuf,
    /// Current REPL conversation id (carried across turns). Unused in one-shot
    /// mode.
    pub conversation_id: Option<String>,
}

impl CliSession {
    /// Wire up the process: keychain → data dir → DB → logger + event sink,
    /// then resolve the working account and model.
    pub fn bootstrap(cli: &Cli, mode: OutputMode) -> Result<Self> {
        // Token reads (sync/chat provider auth) require the native keychain
        // store to be initialised first, exactly as the desktop app does.
        crate::services::keychain::init_native_store()?;

        let data_dir = resolve_data_dir(cli.data_dir.clone());
        let db = Arc::new(Database::new(data_dir.clone())?);

        // Logs go to stderr so stdout stays a clean data/JSON channel.
        crate::services::logger::install(Arc::new(CliLogger::new(cli.quiet)));
        // One-shot mode streams chat tokens straight to stdout; the REPL swaps
        // in a ChannelEventSink per turn (see `repl`).
        crate::services::events::install(Arc::new(super::output::CliEventSink::new(mode)));

        let account = resolve_account(&db, cli.account.as_deref())?;
        let model = resolve_model(&db, cli.model.clone());

        Ok(Self {
            db,
            account,
            model,
            mode,
            quiet: cli.quiet,
            data_dir,
            conversation_id: None,
        })
    }

    /// Account id for commands that require one. Produces a precise error when
    /// the working account is ambiguous (multiple enabled) or absent (none).
    pub fn require_account(&self) -> Result<String> {
        if let Some(a) = &self.account {
            return Ok(a.clone());
        }
        let enabled = self.db.list_accounts()?.into_iter().filter(|a| a.enabled).count();
        if enabled == 0 {
            Err(AppError::InvalidInput("no enabled accounts in DB".into()))
        } else {
            Err(AppError::InvalidInput(
                "multiple enabled accounts — pass --account <id|email> or set a default with \
                 `config set default-account <id|email>`"
                    .into(),
            ))
        }
    }
}

/// `--data-dir` flag → `$EMAILOPS_DATA_DIR` → platform default app data dir.
fn resolve_data_dir(flag: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = flag {
        return dir;
    }
    if let Ok(env) = std::env::var("EMAILOPS_DATA_DIR") {
        if !env.is_empty() {
            return PathBuf::from(env);
        }
    }
    default_data_dir()
}

#[cfg(target_os = "macos")]
fn default_data_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("Library").join("Application Support").join("com.emailops.app"))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(not(target_os = "macos"))]
fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .map(|d| d.join("com.emailops.app"))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve the working account. With an explicit `hint`, match an id or email
/// (a non-matching hint is fatal). Without a hint, prefer the saved
/// `cli_default_account` preference when it still names an enabled account, then
/// fall back to the single enabled account. Returns `None` (rather than
/// erroring) when nothing can be determined, so account-free commands (e.g.
/// `accounts`) still run.
fn resolve_account(db: &Arc<Database>, hint: Option<&str>) -> Result<Option<String>> {
    let accounts = db.list_accounts()?;
    match hint {
        Some(h) => {
            let h = h.trim();
            accounts
                .iter()
                .find(|a| a.id.eq_ignore_ascii_case(h) || a.email.eq_ignore_ascii_case(h))
                .map(|a| Some(a.id.clone()))
                .ok_or_else(|| AppError::NotFound(format!("account '{}' not found in DB", h)))
        }
        None => {
            // A saved CLI default (canonical account id) wins, but only while it
            // still names an enabled account — a stale default never traps the
            // user on a disabled mailbox.
            if let Some(default_id) = db.get_preference(super::config::CLI_DEFAULT_ACCOUNT_KEY)? {
                if let Some(a) = accounts
                    .iter()
                    .find(|a| a.enabled && a.id.eq_ignore_ascii_case(default_id.trim()))
                {
                    return Ok(Some(a.id.clone()));
                }
            }
            let mut enabled = accounts.into_iter().filter(|a| a.enabled);
            let first = enabled.next();
            match (first, enabled.next()) {
                (Some(a), None) => Ok(Some(a.id)),
                _ => Ok(None),
            }
        }
    }
}

/// Model resolution: `--model` override → `ai_model` preference → safe default.
fn resolve_model(db: &Arc<Database>, flag: Option<String>) -> String {
    if let Some(m) = flag {
        if !m.is_empty() {
            return m;
        }
    }
    match db.get_preference("ai_model") {
        Ok(Some(v)) if !v.is_empty() => v,
        _ => "qwen3.5-4b-q4_k_m".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_account(db: &Arc<Database>, id: &str, email: &str, enabled: bool) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES (?1, 'gmail', ?2, ?2, 0, 0, ?3)",
                rusqlite::params![id, email, enabled as i32],
            )
            .expect("seed account");
    }

    #[test]
    fn resolve_account_uses_single_enabled() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "solo@example.com", true);
        assert_eq!(resolve_account(&db, None).expect("resolve"), Some("a1".to_string()));
    }

    #[test]
    fn resolve_account_none_when_multiple_enabled() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "one@example.com", true);
        seed_account(&db, "a2", "two@example.com", true);
        assert_eq!(resolve_account(&db, None).expect("resolve"), None);
    }

    #[test]
    fn resolve_account_matches_email_hint_case_insensitively() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "Person@Example.com", true);
        assert_eq!(
            resolve_account(&db, Some("person@example.com")).expect("resolve"),
            Some("a1".to_string())
        );
    }

    #[test]
    fn resolve_account_uses_saved_default_when_multiple_enabled() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "one@example.com", true);
        seed_account(&db, "a2", "two@example.com", true);
        db.set_preference(super::super::config::CLI_DEFAULT_ACCOUNT_KEY, "a2")
            .expect("set default");
        assert_eq!(resolve_account(&db, None).expect("resolve"), Some("a2".to_string()));
    }

    #[test]
    fn resolve_account_ignores_stale_default_for_disabled_account() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "active@example.com", true);
        seed_account(&db, "a2", "off@example.com", false);
        // Default points at a now-disabled account → fall through to the single
        // enabled one rather than trapping on the stale id.
        db.set_preference(super::super::config::CLI_DEFAULT_ACCOUNT_KEY, "a2")
            .expect("set default");
        assert_eq!(resolve_account(&db, None).expect("resolve"), Some("a1".to_string()));
    }

    #[test]
    fn resolve_account_explicit_hint_overrides_saved_default() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "one@example.com", true);
        seed_account(&db, "a2", "two@example.com", true);
        db.set_preference(super::super::config::CLI_DEFAULT_ACCOUNT_KEY, "a2")
            .expect("set default");
        assert_eq!(
            resolve_account(&db, Some("one@example.com")).expect("resolve"),
            Some("a1".to_string())
        );
    }

    #[test]
    fn resolve_account_unknown_hint_is_error() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "one@example.com", true);
        assert!(resolve_account(&db, Some("ghost@example.com")).is_err());
    }

    #[test]
    fn resolve_model_prefers_flag() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        assert_eq!(resolve_model(&db, Some("custom-model".into())), "custom-model");
    }

    #[test]
    fn resolve_model_falls_back_to_default() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        assert_eq!(resolve_model(&db, None), "qwen3.5-4b-q4_k_m");
    }

    #[test]
    fn resolve_data_dir_prefers_flag() {
        let p = PathBuf::from("/tmp/explicit-dir");
        assert_eq!(resolve_data_dir(Some(p.clone())), p);
    }
}
