//! Settings of the OpenAI-compatible provider: a server the user runs (LM
//! Studio, vLLM, llama.cpp `llama-server`, LiteLLM, a proxy) reached at a base
//! URL they enter.
//!
//! Owns the server's preferences, its API key (in the secrets vault, apart from
//! the OpenRouter key) and [`plan_server_settings`], the pure check every save
//! goes through before anything is written. Building the provider and saving
//! the rest of the AI config stay in `services::ai`.

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::models::error::{AppError, Result};

/// Provider id of the OpenAI-compatible server in `ai_provider`.
pub const PROVIDER_ID: &str = "openai_compatible";

const BASE_URL_PREF: &str = "openai_compatible_base_url";
const EMBEDDING_SOURCE_PREF: &str = "openai_compatible_embedding_source";
const CONTEXT_WINDOW_PREF: &str = "openai_compatible_context_window";
/// The remote host the user last confirmed the disclosure for.
const DISCLOSED_HOST_PREF: &str = "openai_compatible_disclosed_host";
/// The server embedding model that passed the probe, as `{base_url} {model}`,
/// and how it is asked for vectors (`requested` / `native`).
pub(crate) const EMBED_VALIDATED_PREF: &str = "openai_compatible_embedding_validated";
pub(crate) const EMBED_DIMENSIONS_PREF: &str = "openai_compatible_embedding_dimensions";

const KEYRING_SERVICE: &str = "emailops";
const KEY_ID: &str = "openai_compatible_api_key";
/// Set while a key is stored, so its presence can be read without the keychain.
const KEY_ID_PREF: &str = "openai_compatible_api_key_id";
/// Debug builds keep the key here instead of the keychain, like the OpenRouter
/// key (see `AiService::use_dev_ai_keys`).
const DEV_KEY_PREF: &str = "openai_compatible_api_key_dev";

/// Smallest context window a user may set: below it no chat prompt fits.
const MIN_CONTEXT_WINDOW: u32 = 2_048;
/// Largest context window a user may set.
const MAX_CONTEXT_WINDOW: u32 = 1_048_576;

/// Where the email index is computed when chat runs on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingSource {
    /// The same server: mail goes to one destination only.
    #[default]
    Server,
    /// The bundled in-app embedding model, where the embedded runtime runs.
    InApp,
    /// An OpenRouter embedding model.
    OpenRouter,
}

impl EmbeddingSource {
    fn as_pref(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::InApp => "inapp",
            Self::OpenRouter => "openrouter",
        }
    }

    fn from_pref(raw: Option<&str>) -> Self {
        match raw {
            Some("inapp") => Self::InApp,
            Some("openrouter") => Self::OpenRouter,
            _ => Self::Server,
        }
    }
}

/// What Settings sends for the server when it is saved.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettingsInput {
    pub base_url: String,
    /// A new key; `None` or empty keeps the stored one.
    pub api_key: Option<String>,
    /// Delete the stored key.
    #[serde(default)]
    pub clear_api_key: bool,
    #[serde(default)]
    pub embedding_source: EmbeddingSource,
    /// Context window in tokens; `None` or `0` reads it from the server.
    pub context_window: Option<u32>,
    /// The remote host the user just confirmed the disclosure for.
    pub disclosure_accepted_host: Option<String>,
}

/// What happens to the stored key on save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyChange {
    Keep,
    Set(String),
    Clear,
}

/// A checked server configuration, ready to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSettingsPlan {
    /// Normalized: trimmed, no trailing slash, passed `validate_ai_base_url`.
    pub base_url: String,
    pub key: KeyChange,
    pub embedding_source: EmbeddingSource,
    /// `0` = read from the server.
    pub context_window: u32,
    /// The host to remember as disclosed; `None` for a loopback server.
    pub disclosed_host: Option<String>,
}

/// The stored server settings, as Settings shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettings {
    pub base_url: Option<String>,
    pub has_api_key: bool,
    pub embedding_source: EmbeddingSource,
    pub context_window: u32,
    pub disclosed_host: Option<String>,
}

/// Check what Settings sent for the server. Pure.
///
/// `previously_disclosed` is the remote host the user already confirmed the
/// disclosure for; a remote server on any other host needs
/// `disclosure_accepted_host` to name it. `in_app_supported` is whether the
/// embedded runtime runs on this machine.
pub fn plan_server_settings(
    input: &ServerSettingsInput,
    previously_disclosed: Option<&str>,
    in_app_supported: bool,
) -> Result<ServerSettingsPlan> {
    let base_url = crate::services::ai::normalize_ai_base_url(&input.base_url)?;
    let (host, loopback) = crate::services::ai::ai_base_url_host(&base_url)
        .ok_or_else(|| AppError::InvalidInput("The server URL has no host.".to_string()))?;
    let disclosed_host = if loopback {
        None
    } else {
        let confirmed = |candidate: Option<&str>| candidate.is_some_and(|h| h.eq_ignore_ascii_case(&host));
        if !confirmed(previously_disclosed) && !confirmed(input.disclosure_accepted_host.as_deref()) {
            return Err(AppError::InvalidInput(format!(
                "Confirm that email content will be sent to {host} before saving this server."
            )));
        }
        Some(host)
    };

    let new_key = input.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty());
    let key = match (new_key, input.clear_api_key) {
        (Some(_), true) => {
            return Err(AppError::InvalidInput(
                "Either enter a new API key or remove the stored one, not both.".to_string(),
            ))
        }
        (Some(k), false) => KeyChange::Set(k.to_string()),
        (None, true) => KeyChange::Clear,
        (None, false) => KeyChange::Keep,
    };

    if input.embedding_source == EmbeddingSource::InApp && !in_app_supported {
        return Err(AppError::InvalidInput(
            "The in-app embedding model cannot run on this computer. Use the server or OpenRouter for the \
             email index."
                .to_string(),
        ));
    }

    let context_window = input.context_window.unwrap_or(0);
    if context_window != 0 && !(MIN_CONTEXT_WINDOW..=MAX_CONTEXT_WINDOW).contains(&context_window) {
        return Err(AppError::InvalidInput(format!(
            "The context size must be between {MIN_CONTEXT_WINDOW} and {MAX_CONTEXT_WINDOW} tokens, or 0 to \
             read it from the server."
        )));
    }

    Ok(ServerSettingsPlan {
        base_url,
        key,
        embedding_source: input.embedding_source,
        context_window,
        disclosed_host,
    })
}

/// Write `plan`'s preferences (the key is written by [`apply_key_change`]).
pub(crate) fn store_server_settings(db: &Database, plan: &ServerSettingsPlan) -> Result<()> {
    db.set_preference(BASE_URL_PREF, &plan.base_url)?;
    db.set_preference(EMBEDDING_SOURCE_PREF, plan.embedding_source.as_pref())?;
    db.set_preference(CONTEXT_WINDOW_PREF, &plan.context_window.to_string())?;
    match &plan.disclosed_host {
        Some(host) => db.set_preference(DISCLOSED_HOST_PREF, host)?,
        None => db.delete_preference(DISCLOSED_HOST_PREF)?,
    }
    Ok(())
}

/// The stored server settings.
pub fn load_server_settings(db: &Database) -> Result<ServerSettings> {
    Ok(ServerSettings {
        base_url: db.get_preference(BASE_URL_PREF)?,
        has_api_key: has_api_key(db)?,
        embedding_source: EmbeddingSource::from_pref(db.get_preference(EMBEDDING_SOURCE_PREF)?.as_deref()),
        context_window: db
            .get_preference(CONTEXT_WINDOW_PREF)?
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0),
        disclosed_host: db.get_preference(DISCLOSED_HOST_PREF)?,
    })
}

fn has_api_key(db: &Database) -> Result<bool> {
    Ok(db.get_preference(KEY_ID_PREF)?.is_some())
}

/// The stored server key, or an empty string when there is none (many local
/// servers need no key).
pub fn load_api_key(db: &Database) -> Result<String> {
    load_api_key_from(db, cfg!(debug_assertions))
}

/// Apply `change` to the stored key: in the secrets vault, or in a preference
/// on debug builds (`dev`).
pub(crate) fn apply_key_change(db: &Database, change: &KeyChange, dev: bool) -> Result<()> {
    match change {
        KeyChange::Keep => {}
        KeyChange::Set(key) if dev => {
            db.set_preference(DEV_KEY_PREF, key)?;
            db.set_preference(KEY_ID_PREF, KEY_ID)?;
        }
        KeyChange::Set(key) => {
            crate::services::secrets_vault::set(KEYRING_SERVICE, KEY_ID, key)?;
            db.set_preference(KEY_ID_PREF, KEY_ID)?;
        }
        KeyChange::Clear => {
            if dev {
                db.delete_preference(DEV_KEY_PREF)?;
            } else {
                crate::services::secrets_vault::delete(KEYRING_SERVICE, KEY_ID)?;
            }
            db.delete_preference(KEY_ID_PREF)?;
        }
    }
    Ok(())
}

/// [`load_api_key`] reading from the vault or, on debug builds (`dev`), the
/// preference.
pub(crate) fn load_api_key_from(db: &Database, dev: bool) -> Result<String> {
    if !has_api_key(db)? {
        return Ok(String::new());
    }
    let key = if dev {
        db.get_preference(DEV_KEY_PREF)?
    } else {
        crate::services::secrets_vault::get(KEYRING_SERVICE, KEY_ID)?
    };
    Ok(key.unwrap_or_default())
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    fn input(base_url: &str) -> ServerSettingsInput {
        ServerSettingsInput {
            base_url: base_url.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_loopback_server_needs_no_disclosure() {
        let plan = plan_server_settings(&input(" http://localhost:1234/v1/ "), None, true).expect("loopback");

        assert_eq!(plan.base_url, "http://localhost:1234/v1");
        assert_eq!(plan.disclosed_host, None);
        assert_eq!(plan.key, KeyChange::Keep);
        assert_eq!(plan.embedding_source, EmbeddingSource::Server);
        assert_eq!(plan.context_window, 0);
    }

    #[test]
    fn the_base_url_rule_is_the_shared_one() {
        for refused in crate::services::ai::REFUSED_AI_BASE_URLS {
            assert!(
                plan_server_settings(&input(refused), Some("api.example.com"), true).is_err(),
                "{refused} must be refused"
            );
        }
    }

    #[test]
    fn a_remote_server_is_refused_until_its_disclosure_is_confirmed() {
        let err = plan_server_settings(&input("https://llm.example.com/v1"), None, true).expect_err("no disclosure");

        assert!(
            matches!(&err, AppError::InvalidInput(m) if m.contains("llm.example.com")),
            "{err:?}"
        );
    }

    #[test]
    fn a_confirmed_remote_host_is_remembered() {
        let confirmed = ServerSettingsInput {
            disclosure_accepted_host: Some("llm.example.com".to_string()),
            ..input("https://llm.example.com/v1")
        };

        let plan = plan_server_settings(&confirmed, None, true).expect("confirmed");

        assert_eq!(plan.disclosed_host.as_deref(), Some("llm.example.com"));
    }

    #[test]
    fn a_host_confirmed_before_needs_no_new_confirmation() {
        let plan = plan_server_settings(&input("https://llm.example.com/v2"), Some("llm.example.com"), true)
            .expect("same host");

        assert_eq!(plan.disclosed_host.as_deref(), Some("llm.example.com"));
    }

    #[test]
    fn moving_to_another_remote_host_asks_again() {
        let moved = ServerSettingsInput {
            // Confirming the old host does not cover the new one.
            disclosure_accepted_host: Some("llm.example.com".to_string()),
            ..input("https://other.example.net/v1")
        };

        assert!(plan_server_settings(&moved, Some("llm.example.com"), true).is_err());
    }

    #[test]
    fn a_new_key_is_set_and_a_blank_one_keeps_the_stored_key() {
        let with_key = ServerSettingsInput {
            api_key: Some("  sk-local  ".to_string()),
            ..input("http://127.0.0.1:8080/v1")
        };
        let blank = ServerSettingsInput {
            api_key: Some("   ".to_string()),
            ..input("http://127.0.0.1:8080/v1")
        };

        assert_eq!(
            plan_server_settings(&with_key, None, true).unwrap().key,
            KeyChange::Set("sk-local".to_string())
        );
        assert_eq!(plan_server_settings(&blank, None, true).unwrap().key, KeyChange::Keep);
    }

    #[test]
    fn a_stored_key_can_be_cleared() {
        let clear = ServerSettingsInput {
            clear_api_key: true,
            ..input("http://127.0.0.1:8080/v1")
        };

        assert_eq!(plan_server_settings(&clear, None, true).unwrap().key, KeyChange::Clear);
    }

    #[test]
    fn clearing_and_setting_a_key_at_once_is_refused() {
        let both = ServerSettingsInput {
            api_key: Some("sk".to_string()),
            clear_api_key: true,
            ..input("http://127.0.0.1:8080/v1")
        };

        assert!(matches!(
            plan_server_settings(&both, None, true),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn in_app_embeddings_are_refused_where_the_runtime_cannot_run() {
        let in_app = ServerSettingsInput {
            embedding_source: EmbeddingSource::InApp,
            ..input("http://localhost:1234/v1")
        };

        assert!(plan_server_settings(&in_app, None, true).is_ok());
        let err = plan_server_settings(&in_app, None, false).expect_err("unsupported");
        assert!(
            matches!(&err, AppError::InvalidInput(m) if m.contains("in-app")),
            "{err:?}"
        );
    }

    #[test]
    fn a_context_window_is_kept_within_bounds() {
        let with = |n: u32| ServerSettingsInput {
            context_window: Some(n),
            ..input("http://localhost:1234/v1")
        };

        assert_eq!(plan_server_settings(&with(0), None, true).unwrap().context_window, 0);
        assert_eq!(
            plan_server_settings(&with(32_768), None, true).unwrap().context_window,
            32_768
        );
        assert!(plan_server_settings(&with(512), None, true).is_err());
        assert!(plan_server_settings(&with(4_000_000), None, true).is_err());
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use crate::services::keychain::Keychain;

    fn setup() -> (
        std::sync::MutexGuard<'static, ()>,
        std::sync::Arc<crate::services::keychain::InMemoryKeychain>,
        Database,
    ) {
        let guard = crate::services::events::seam_test_lock();
        crate::services::secrets_vault::reset_for_testing();
        let kc = crate::services::keychain::install_for_testing();
        (guard, kc, Database::new_for_testing().expect("db"))
    }

    #[test]
    fn the_key_lives_in_the_vault_and_never_in_preferences() {
        let (_guard, kc, db) = setup();

        apply_key_change(&db, &KeyChange::Set("sk-server".to_string()), false).unwrap();

        assert_eq!(load_api_key_from(&db, false).unwrap(), "sk-server");
        assert!(kc.get_password(KEYRING_SERVICE, "vault").unwrap().is_some());
        let prefs: Vec<String> = db
            .reader()
            .prepare("SELECT value FROM user_preferences")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(!prefs.iter().any(|v| v.contains("sk-server")), "{prefs:?}");
        assert!(load_server_settings(&db).unwrap().has_api_key);
    }

    #[test]
    fn the_server_key_is_separate_from_the_openrouter_key() {
        let (_guard, _kc, db) = setup();
        crate::services::secrets_vault::set(KEYRING_SERVICE, "openrouter_api_key", "sk-or").unwrap();

        apply_key_change(&db, &KeyChange::Set("sk-server".to_string()), false).unwrap();

        assert_eq!(
            crate::services::secrets_vault::get(KEYRING_SERVICE, "openrouter_api_key").unwrap(),
            Some("sk-or".to_string())
        );
    }

    #[test]
    fn a_cleared_key_is_deleted_from_the_vault() {
        let (_guard, _kc, db) = setup();
        apply_key_change(&db, &KeyChange::Set("sk-server".to_string()), false).unwrap();

        apply_key_change(&db, &KeyChange::Clear, false).unwrap();

        assert_eq!(
            crate::services::secrets_vault::get(KEYRING_SERVICE, KEY_ID).unwrap(),
            None
        );
        assert_eq!(load_api_key_from(&db, false).unwrap(), "");
        assert!(!load_server_settings(&db).unwrap().has_api_key);
    }

    #[test]
    fn keep_leaves_the_stored_key_alone() {
        let (_guard, _kc, db) = setup();
        apply_key_change(&db, &KeyChange::Set("sk-server".to_string()), false).unwrap();

        apply_key_change(&db, &KeyChange::Keep, false).unwrap();

        assert_eq!(load_api_key_from(&db, false).unwrap(), "sk-server");
    }
}
