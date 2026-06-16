//! `emailops-cli config` — get/set CLI-local preferences.
//!
//! Values live in the shared SQLite `user_preferences` table under a `cli_`
//! namespace, so they never collide with the desktop app's own settings (a CLI
//! default account does not change what the desktop app has selected, and vice
//! versa). Today the only key is the default account used when `--account` is
//! omitted; [`ConfigKey`] is the single place to register more.

use clap::{Subcommand, ValueEnum};

use crate::models::error::{AppError, Result};

use super::output;
use super::session::CliSession;
use super::OutputMode;

/// Storage key for the CLI's default account. Read at bootstrap by
/// `session::resolve_account`; written here by `config set default-account`.
pub const CLI_DEFAULT_ACCOUNT_KEY: &str = "cli_default_account";

/// What to do with a [`ConfigKey`].
#[derive(Subcommand, Debug, Clone)]
pub enum ConfigAction {
    /// Print a setting's stored value (or `(unset)`).
    Get {
        /// Which setting to read.
        key: ConfigKey,
    },
    /// Store a setting's value.
    Set {
        /// Which setting to write.
        key: ConfigKey,
        /// The value to store.
        value: String,
    },
    /// Clear a setting, reverting to the CLI's default behaviour.
    Unset {
        /// Which setting to clear.
        key: ConfigKey,
    },
    /// List every CLI setting and its current value.
    List,
}

/// The set of CLI-local settings. Each maps to a `cli_`-namespaced key in the
/// shared `user_preferences` table. clap renders the variants in kebab-case
/// (`DefaultAccount` → `default-account`).
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKey {
    /// Account (id or email) used when `--account` is omitted and more than one
    /// account is enabled.
    DefaultAccount,
}

impl ConfigKey {
    /// The `user_preferences` storage key — always `cli_`-namespaced.
    fn pref_key(self) -> &'static str {
        match self {
            ConfigKey::DefaultAccount => CLI_DEFAULT_ACCOUNT_KEY,
        }
    }

    /// The kebab-case name shown in output (matches clap's value parsing).
    fn cli_name(self) -> &'static str {
        match self {
            ConfigKey::DefaultAccount => "default-account",
        }
    }
}

/// Execute a `config` action against the session's DB.
pub fn run_config(session: &CliSession, action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Get { key } => {
            let value = session.db.get_preference(key.pref_key())?;
            emit_setting(session.mode, key, value.as_deref())
        }
        ConfigAction::Set { key, value } => {
            let stored = canonicalize_value(session, key, &value)?;
            session.db.set_preference(key.pref_key(), &stored)?;
            emit_setting(session.mode, key, Some(&stored))
        }
        ConfigAction::Unset { key } => {
            session.db.delete_preference(key.pref_key())?;
            emit_setting(session.mode, key, None)
        }
        ConfigAction::List => {
            let mut entries: Vec<(ConfigKey, Option<String>)> = Vec::new();
            for key in ConfigKey::value_variants() {
                entries.push((*key, session.db.get_preference(key.pref_key())?));
            }
            if session.mode == OutputMode::Json {
                let data: Vec<_> = entries
                    .iter()
                    .map(|(k, v)| serde_json::json!({ "key": k.cli_name(), "value": v }))
                    .collect();
                return output::emit_ok(data);
            }
            for (key, value) in entries {
                print_setting_line(key, value.as_deref());
            }
            Ok(())
        }
    }
}

/// Validate a value for `key` and return the canonical form to store. For
/// `default-account` the input (id or email) must match a known account; we
/// store the canonical account id so resolution is stable regardless of how the
/// user typed it.
fn canonicalize_value(session: &CliSession, key: ConfigKey, value: &str) -> Result<String> {
    match key {
        ConfigKey::DefaultAccount => {
            let needle = value.trim();
            session
                .db
                .list_accounts()?
                .into_iter()
                .find(|a| a.id.eq_ignore_ascii_case(needle) || a.email.eq_ignore_ascii_case(needle))
                .map(|a| a.id)
                .ok_or_else(|| AppError::NotFound(format!("no account matches '{}'", needle)))
        }
    }
}

fn emit_setting(mode: OutputMode, key: ConfigKey, value: Option<&str>) -> Result<()> {
    if mode == OutputMode::Json {
        return output::emit_ok(serde_json::json!({ "key": key.cli_name(), "value": value }));
    }
    print_setting_line(key, value);
    Ok(())
}

fn print_setting_line(key: ConfigKey, value: Option<&str>) {
    match value {
        Some(v) => println!("{} = {}", key.cli_name(), v),
        None => println!("{} (unset)", key.cli_name()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::db::Database;

    use super::super::OutputMode;
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

    fn session(db: Arc<Database>) -> CliSession {
        CliSession {
            db,
            account: None,
            model: "test-model".to_string(),
            mode: OutputMode::Json,
            quiet: true,
            log_quiet: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            data_dir: PathBuf::from("/tmp/emailops-cli-test"),
            conversation_id: None,
        }
    }

    #[test]
    fn set_default_account_persists_canonical_id() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "Solo@Example.com", true);
        let s = session(db.clone());
        // Set via email (mixed case) — stored value is the canonical id.
        run_config(
            &s,
            ConfigAction::Set {
                key: ConfigKey::DefaultAccount,
                value: "solo@example.com".into(),
            },
        )
        .expect("set ok");
        assert_eq!(
            db.get_preference(CLI_DEFAULT_ACCOUNT_KEY).expect("read"),
            Some("a1".to_string())
        );
    }

    #[test]
    fn set_default_account_rejects_unknown() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "solo@example.com", true);
        let s = session(db);
        let err = run_config(
            &s,
            ConfigAction::Set {
                key: ConfigKey::DefaultAccount,
                value: "ghost@example.com".into(),
            },
        )
        .expect_err("unknown account must error");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }

    #[test]
    fn unset_clears_stored_value() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        seed_account(&db, "a1", "solo@example.com", true);
        let s = session(db.clone());
        run_config(
            &s,
            ConfigAction::Set {
                key: ConfigKey::DefaultAccount,
                value: "a1".into(),
            },
        )
        .expect("set ok");
        run_config(
            &s,
            ConfigAction::Unset {
                key: ConfigKey::DefaultAccount,
            },
        )
        .expect("unset ok");
        assert_eq!(db.get_preference(CLI_DEFAULT_ACCOUNT_KEY).expect("read"), None);
    }
}
