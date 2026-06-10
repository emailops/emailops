//! `emailops-cli accounts` — list and add accounts.
//!
//! Listing is read-only. Adding reuses the exact same `services::accounts`
//! entry points the desktop app's "Add account" dialog calls, so the CLI needs
//! no `AppHandle`:
//!   - **Gmail / Outlook** → [`crate::services::accounts::add_account`], which
//!     runs the OAuth flow (opens the user's browser, listens on a loopback
//!     port, exchanges the code) and stores tokens in the OS keychain.
//!   - **IMAP** → [`crate::services::accounts::add_imap_account`], which verifies
//!     the credentials by logging in before persisting them to the keychain.
//!
//! The bootstrap already initialised the native keychain store, so credential
//! storage works the same as in the desktop app.

use clap::Subcommand;

use crate::models::error::{AppError, Result};
use crate::models::Account;
use crate::sync::imap::ImapCredentials;

use super::output;
use super::session::CliSession;
use super::OutputMode;

/// What to do under `accounts`. `None` (bare `accounts`) defaults to [`List`].
#[derive(Subcommand, Debug, Clone)]
pub enum AccountAction {
    /// List configured accounts (the default when no action is given).
    List,
    /// Add a new account (Gmail / Outlook via OAuth, or IMAP/SMTP).
    Add {
        #[command(subcommand)]
        provider: AddProvider,
    },
}

/// The provider to add and its connection details.
#[derive(Subcommand, Debug, Clone)]
pub enum AddProvider {
    /// Gmail via OAuth — opens your browser to authorize.
    Gmail {
        /// Only sync mail on/after this date (YYYY-MM-DD). Default: all history.
        #[arg(long, value_name = "YYYY-MM-DD")]
        sync_from: Option<String>,
    },
    /// Outlook / Microsoft 365 via OAuth — opens your browser to authorize.
    Outlook {
        /// Only sync mail on/after this date (YYYY-MM-DD). Default: all history.
        #[arg(long, value_name = "YYYY-MM-DD")]
        sync_from: Option<String>,
    },
    /// IMAP/SMTP with a username + (app) password.
    Imap {
        /// IMAP server host, e.g. imap.fastmail.com.
        #[arg(long)]
        host: String,
        /// IMAP port (TLS).
        #[arg(long, default_value_t = 993)]
        port: u16,
        /// Login username — usually the full email address.
        #[arg(long)]
        username: String,
        /// Password / app password. Omit to be prompted securely (no echo);
        /// required when running with `--json` (which cannot prompt).
        #[arg(long)]
        password: Option<String>,
        /// SMTP server host. Defaults to the IMAP host.
        #[arg(long)]
        smtp_host: Option<String>,
        /// SMTP port (STARTTLS).
        #[arg(long, default_value_t = 587)]
        smtp_port: u16,
        /// Display name for the account. Defaults to the username.
        #[arg(long)]
        name: Option<String>,
        /// Only sync mail on/after this date (YYYY-MM-DD). Default: all history.
        #[arg(long, value_name = "YYYY-MM-DD")]
        sync_from: Option<String>,
    },
}

/// Execute an `accounts` action. Bare `accounts` lists.
pub async fn run_account(session: &CliSession, action: Option<AccountAction>) -> Result<()> {
    match action.unwrap_or(AccountAction::List) {
        AccountAction::List => {
            let accounts = crate::services::accounts::list_accounts(&session.db)?;
            output::render_accounts(&accounts, session.mode)
        }
        AccountAction::Add { provider } => add_account(session, provider).await,
    }
}

async fn add_account(session: &CliSession, provider: AddProvider) -> Result<()> {
    let account = match provider {
        AddProvider::Gmail { sync_from } => {
            let ts = parse_sync_from(sync_from.as_deref())?;
            announce_oauth("Gmail");
            crate::services::accounts::add_account(&session.db, "gmail", ts).await?
        }
        AddProvider::Outlook { sync_from } => {
            let ts = parse_sync_from(sync_from.as_deref())?;
            announce_oauth("Outlook");
            crate::services::accounts::add_account(&session.db, "outlook", ts).await?
        }
        AddProvider::Imap {
            host,
            port,
            username,
            password,
            smtp_host,
            smtp_port,
            name,
            sync_from,
        } => {
            let ts = parse_sync_from(sync_from.as_deref())?;
            let password = resolve_password(session.mode, password)?;
            let credentials = ImapCredentials {
                smtp_host: smtp_host.unwrap_or_else(|| host.clone()),
                host,
                port,
                username,
                password,
                smtp_port,
            };
            crate::services::accounts::add_imap_account(&session.db, credentials, name, ts).await?
        }
    };

    render_added(&account, session.mode)
}

/// Tell the user a browser is about to open. Goes to **stderr** so it never
/// pollutes the `--json` stdout document.
fn announce_oauth(provider: &str) {
    eprintln!("Opening your browser to authorize {provider}… (complete the sign-in, then return here)");
}

/// Resolve the IMAP password: use the flag if given, otherwise prompt without
/// echo. In `--json` mode there is no interactive prompt, so the flag is
/// required — fail with a clear message rather than blocking on a hidden read.
fn resolve_password(mode: OutputMode, provided: Option<String>) -> Result<String> {
    match provided {
        Some(p) => Ok(p),
        None => match mode {
            OutputMode::Json => Err(AppError::InvalidInput(
                "--password is required when using --json (cannot prompt securely)".to_string(),
            )),
            OutputMode::Pretty => rpassword::prompt_password("IMAP password: ")
                .map_err(|e| AppError::IoError(format!("failed to read password: {e}"))),
        },
    }
}

/// Parse an optional `YYYY-MM-DD` "sync from" date into a Unix timestamp
/// (midnight UTC). `None` input → `None` (sync all history). Pure so the parsing
/// is unit-testable.
fn parse_sync_from(date: Option<&str>) -> Result<Option<i64>> {
    let Some(raw) = date else {
        return Ok(None);
    };
    let parsed = chrono::NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map_err(|_| AppError::InvalidInput(format!("invalid --sync-from date '{raw}' (expected YYYY-MM-DD)")))?;
    let ts = parsed
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::InvalidInput(format!("invalid --sync-from date '{raw}'")))?
        .and_utc()
        .timestamp();
    Ok(Some(ts))
}

fn render_added(account: &Account, mode: OutputMode) -> Result<()> {
    if mode == OutputMode::Json {
        return output::emit_ok(account);
    }
    println!("Added {} account {} ({}).", account.provider, account.email, account.id);
    println!(
        "Set it as the CLI default with: emailops-cli config set default-account {}",
        account.email
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_from_none_is_none() {
        assert_eq!(parse_sync_from(None).expect("ok"), None);
    }

    #[test]
    fn sync_from_parses_iso_date_to_utc_midnight() {
        // 2024-01-01T00:00:00Z = 1704067200
        assert_eq!(parse_sync_from(Some("2024-01-01")).expect("ok"), Some(1_704_067_200));
    }

    #[test]
    fn sync_from_trims_whitespace() {
        assert_eq!(
            parse_sync_from(Some("  2024-01-01  ")).expect("ok"),
            Some(1_704_067_200)
        );
    }

    #[test]
    fn sync_from_rejects_garbage() {
        let err = parse_sync_from(Some("01/01/2024")).expect_err("must reject");
        assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
    }

    #[test]
    fn password_flag_is_used_verbatim() {
        assert_eq!(resolve_password(OutputMode::Json, Some("pw".into())).expect("ok"), "pw");
        assert_eq!(
            resolve_password(OutputMode::Pretty, Some("pw".into())).expect("ok"),
            "pw"
        );
    }

    #[test]
    fn password_required_in_json_mode_when_omitted() {
        let err = resolve_password(OutputMode::Json, None).expect_err("must require flag");
        assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
    }
}
