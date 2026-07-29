//! Per-process CLI session: the open database plus the resolved account/model
//! that every command (and every REPL turn) operates against.
//!
//! [`CliSession::bootstrap`] is the single place that wires the CLI into the
//! same seams the desktop app uses: it initialises the keychain, resolves the
//! data directory, opens the DB (running migrations), and installs the CLI's
//! [`Logger`](crate::services::logger) + [`EventSink`](crate::services::events)
//! backends. From that point on the command handlers call `services::*`
//! directly — no `AppHandle` required.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::db::Database;
use crate::models::error::{AppError, Result};

use super::output::CliLogger;
use super::{resolve_render_style, Cli, Command, OutputMode, RenderStyle};

/// State shared across one CLI invocation (one-shot) or one REPL session.
pub struct CliSession {
    pub db: Arc<Database>,
    /// Resolved account, if exactly one could be determined at bootstrap. `None`
    /// when there are zero or multiple enabled accounts and no `--account` hint;
    /// commands that need an account call [`CliSession::require_account`].
    pub account: Option<String>,
    pub model: String,
    pub mode: OutputMode,
    /// Resolved human-output styling for this invocation (see [`RenderStyle`]).
    /// `--json` → `Json` (no styling); pretty on a TTY → `Rich`; pretty piped or
    /// `NO_COLOR` → `Plain`. Drives ANSI color, table re-rendering, and the chat
    /// live-preview/redraw, all of which stay out of the agent/piped paths.
    pub style: RenderStyle,
    pub quiet: bool,
    /// Shared verbosity gate for the installed [`CliLogger`]. Flipped per chat
    /// turn so a chat without `--trace` suppresses the diagnostic app-log stream
    /// (route / retrieval / kv / stage lines) while non-chat commands keep their
    /// progress logs. `--quiet` is the hard floor (see [`chat_log_quiet`]).
    pub log_quiet: Arc<AtomicBool>,
    /// Directory the DB lives in — needed by `sync_account`.
    pub data_dir: PathBuf,
    /// Current REPL conversation id (carried across turns). Unused in one-shot
    /// mode.
    pub conversation_id: Option<String>,
}

/// Whether the diagnostic app-log stream should be suppressed for a chat turn.
/// Chat treats those lines as "the trace", so they only surface with `--trace`;
/// a global `--quiet` keeps them suppressed regardless. Pure so the rule is
/// unit-testable without installing a logger.
pub(crate) fn chat_log_quiet(global_quiet: bool, trace: bool) -> bool {
    global_quiet || !trace
}

/// Whether `Database::new`'s `[db-init]` startup-timing stream should print for
/// this invocation. Those lines are diagnostics too, so they only surface when a
/// command explicitly asked for a trace (`chat`/`search --trace`); every other
/// invocation — including the bare-REPL launch — keeps startup silent. Pure so
/// the rule is unit-testable. Mirrors [`chat_log_quiet`] for startup-time logs
/// that predate the logger seam.
pub(crate) fn startup_timing_enabled(command: Option<&Command>) -> bool {
    matches!(
        command,
        Some(Command::Chat { trace: true, .. }) | Some(Command::Search { trace: true, .. })
    )
}

impl CliSession {
    /// Wire up the process: keychain → data dir → DB → logger + event sink,
    /// then resolve the working account and model.
    pub fn bootstrap(cli: &Cli, mode: OutputMode) -> Result<Self> {
        // Token reads (sync/chat provider auth) require the native keychain
        // store to be initialised first, exactly as the desktop app does.
        crate::services::keychain::init_native_store()?;

        // Silence `Database::new`'s `[db-init]` startup timings unless a
        // `--trace` command asked for diagnostics. Must precede `Database::new`.
        crate::db::set_db_init_timing(startup_timing_enabled(cli.command.as_ref()));

        // Resolve the human-output style once: stdout TTY + NO_COLOR gate the
        // styling so agents (`--json`) and piped captures never pay for ANSI.
        let style = resolve_render_style(
            mode == OutputMode::Json,
            std::io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR").is_some(),
        );

        let data_dir = resolve_data_dir(cli.data_dir.clone());
        let db = Arc::new(Database::new(data_dir.clone())?);

        // Credential reads (sync auth, remote AI providers) resolve through a
        // process-global DB handle in dev builds. Bind it — but don't warm the
        // cache: a one-shot command shouldn't hit the keychain for accounts it
        // never touches.
        crate::services::accounts::bind_credential_db(&db);

        // Logs go to stderr so stdout stays a clean data/JSON channel. The gate
        // is shared with the session so chat turns can flip it per `--trace`.
        let log_quiet = Arc::new(AtomicBool::new(cli.quiet));
        crate::services::logger::install(Arc::new(CliLogger::new(log_quiet.clone())));
        // One-shot mode streams chat tokens straight to stdout; the REPL swaps
        // in a ChannelEventSink per turn (see `repl`).
        crate::services::events::install(Arc::new(super::output::CliEventSink::new(style)));

        let account = resolve_account(&db, cli.account.as_deref())?;
        let model = resolve_model(&db, cli.model.clone());

        Ok(Self {
            db,
            account,
            model,
            mode,
            style,
            quiet: cli.quiet,
            log_quiet,
            data_dir,
            conversation_id: None,
        })
    }

    /// Gate the diagnostic app-log stream for a chat turn on `--trace`: a chat
    /// without it stays quiet (only the streamed answer + errors reach the
    /// terminal); `--trace` re-enables the full stream. Honours the `--quiet`
    /// floor via [`chat_log_quiet`].
    pub fn apply_chat_log_quiet(&self, trace: bool) {
        self.log_quiet
            .store(chat_log_quiet(self.quiet, trace), Ordering::Relaxed);
    }

    /// Restore the logger to the session default (the global `--quiet` flag), so
    /// non-chat work that follows a chat turn (notably in the REPL) keeps its
    /// progress logs.
    pub fn restore_log_quiet(&self) {
        self.log_quiet.store(self.quiet, Ordering::Relaxed);
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
pub(crate) fn resolve_account(db: &Arc<Database>, hint: Option<&str>) -> Result<Option<String>> {
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

    fn session_with_quiet(quiet: bool) -> CliSession {
        CliSession {
            db: Arc::new(Database::new_for_testing().expect("test db")),
            account: None,
            model: "test-model".to_string(),
            mode: OutputMode::Pretty,
            style: RenderStyle::Plain,
            quiet,
            log_quiet: Arc::new(AtomicBool::new(quiet)),
            data_dir: PathBuf::from("/tmp/emailops-cli-test"),
            conversation_id: None,
        }
    }

    #[test]
    fn apply_chat_log_quiet_suppresses_without_trace_then_restores() {
        let session = session_with_quiet(false);
        // Default chat (no --trace) suppresses the diagnostic stream …
        session.apply_chat_log_quiet(false);
        assert!(session.log_quiet.load(Ordering::Relaxed));
        // … and restoring returns to the session default (logs on).
        session.restore_log_quiet();
        assert!(!session.log_quiet.load(Ordering::Relaxed));
    }

    #[test]
    fn apply_chat_log_quiet_shows_stream_with_trace() {
        let session = session_with_quiet(false);
        session.apply_chat_log_quiet(true);
        assert!(!session.log_quiet.load(Ordering::Relaxed));
    }

    #[test]
    fn apply_chat_log_quiet_keeps_quiet_floor_even_with_trace() {
        let session = session_with_quiet(true);
        session.apply_chat_log_quiet(true);
        assert!(session.log_quiet.load(Ordering::Relaxed));
    }

    #[test]
    fn startup_timing_on_only_for_trace_commands() {
        assert!(startup_timing_enabled(Some(&Command::Chat {
            questions: vec!["q".into()],
            trace: true,
            conversation: None,
            fresh: false,
            prewarm: false,
        })));
        assert!(startup_timing_enabled(Some(&Command::Search {
            query: "q".into(),
            limit: 25,
            offset: 0,
            trace: true,
        })));
    }

    #[test]
    fn startup_timing_off_without_trace_and_for_repl() {
        assert!(!startup_timing_enabled(Some(&Command::Chat {
            questions: vec!["q".into()],
            trace: false,
            conversation: None,
            fresh: false,
            prewarm: false,
        })));
        assert!(!startup_timing_enabled(Some(&Command::Doctor)));
        // Bare invocation (REPL) → command is None → startup stays silent.
        assert!(!startup_timing_enabled(None));
    }

    #[test]
    fn chat_log_quiet_suppresses_logs_without_trace() {
        // Default chat: no --trace, no --quiet → suppress the diagnostic stream.
        assert!(chat_log_quiet(false, false));
    }

    #[test]
    fn chat_log_quiet_shows_logs_with_trace() {
        // --trace re-enables the full app-log stream.
        assert!(!chat_log_quiet(false, true));
    }

    #[test]
    fn chat_log_quiet_global_quiet_is_a_hard_floor() {
        // --quiet wins even when --trace is set.
        assert!(chat_log_quiet(true, true));
        assert!(chat_log_quiet(true, false));
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
