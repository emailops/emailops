use std::collections::BTreeMap;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Database error: {0}")]
    DbError(#[from] rusqlite::Error),

    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON parsing error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("OAuth error: {0}")]
    OAuthError(String),

    #[error("Sync error: {0}")]
    SyncError(String),

    #[error("Keyring error: {0}")]
    KeyringError(String),

    /// The account's stored credentials are missing or unreadable in a way
    /// that requires the user to re-authenticate (e.g. keychain entry was
    /// removed, or tokens were never persisted to the active backend).
    /// This is distinct from `KeyringError`, which represents an infrastructure
    /// failure where the keychain itself is unavailable.
    /// Display message contains "authentication" so frontend ErrorBanner's
    /// auth-substring detection surfaces the "Sign in again" button.
    #[error("Authentication required for account {account_id} — please sign in again")]
    NeedsReauth { account_id: String },

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("AI error: {0}")]
    AiError(String),

    #[error("AI features are disabled in Settings")]
    AiDisabled,

    #[error("IO error: {0}")]
    IoError(String),

    #[error("Budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("Cancelled by user")]
    Cancelled,
}

impl AppError {
    /// Stable, machine-readable identifier for this error variant. Used as the
    /// translation key under the `errors:` namespace on the frontend
    /// (e.g. `t("errors:codes." + code, params)`). Codes are stable across
    /// releases — renaming a code is a breaking change for translation files.
    pub fn code(&self) -> &'static str {
        match self {
            AppError::AuthError(_) => "auth",
            AppError::DbError(_) => "database",
            AppError::HttpError(_) => "http",
            AppError::JsonError(_) => "json",
            AppError::OAuthError(_) => "oauth",
            AppError::SyncError(_) => "sync",
            AppError::KeyringError(_) => "keyring",
            AppError::NeedsReauth { .. } => "needs_reauth",
            AppError::NotFound(_) => "not_found",
            AppError::InvalidInput(_) => "invalid_input",
            AppError::AiError(_) => "ai",
            AppError::AiDisabled => "ai_disabled",
            AppError::IoError(_) => "io",
            AppError::BudgetExceeded(_) => "budget_exceeded",
            AppError::Cancelled => "cancelled",
        }
    }

    /// Structured parameters interpolated into the localized message. Keys are
    /// stable per `code()` (e.g. `needs_reauth` always carries `accountId`).
    /// Variants without structured fields populate `detail` with the raw
    /// underlying message so localized templates can still render context.
    pub fn params(&self) -> BTreeMap<&'static str, String> {
        let mut p = BTreeMap::new();
        match self {
            AppError::AuthError(detail)
            | AppError::OAuthError(detail)
            | AppError::SyncError(detail)
            | AppError::KeyringError(detail)
            | AppError::NotFound(detail)
            | AppError::InvalidInput(detail)
            | AppError::AiError(detail)
            | AppError::IoError(detail)
            | AppError::BudgetExceeded(detail) => {
                p.insert("detail", detail.clone());
            }
            AppError::DbError(e) => {
                p.insert("detail", e.to_string());
            }
            AppError::HttpError(e) => {
                p.insert("detail", e.to_string());
            }
            AppError::JsonError(e) => {
                p.insert("detail", e.to_string());
            }
            AppError::NeedsReauth { account_id } => {
                p.insert("accountId", account_id.clone());
            }
            AppError::AiDisabled | AppError::Cancelled => {}
        }
        p
    }
}

/// Wire format for `AppError` at the Tauri command boundary. The frontend reads
/// `code` + `params` for localized rendering (via the `errors:` namespace) and
/// falls back to `message` when no translation is available.
///
/// Kept manual rather than `#[derive(Serialize)]` on `AppError` itself so the
/// shape is independent of the variant layout — adding a new variant only
/// requires extending `code()`/`params()`.
impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppError", 3)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("params", &self.params())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_stable_per_variant() {
        assert_eq!(AppError::AuthError("x".into()).code(), "auth");
        assert_eq!(AppError::AiDisabled.code(), "ai_disabled");
        assert_eq!(AppError::Cancelled.code(), "cancelled");
        assert_eq!(AppError::NeedsReauth { account_id: "a".into() }.code(), "needs_reauth");
    }

    #[test]
    fn needs_reauth_serializes_account_id_param() {
        let err = AppError::NeedsReauth {
            account_id: "acct-7".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&err).expect("serialize");
        assert_eq!(v["code"], "needs_reauth");
        assert_eq!(v["params"]["accountId"], "acct-7");
        assert!(v["message"].as_str().unwrap().contains("acct-7"));
    }

    #[test]
    fn detail_carrying_variants_expose_detail_param() {
        let err = AppError::SyncError("Gmail returned 503".into());
        let v: serde_json::Value = serde_json::to_value(&err).expect("serialize");
        assert_eq!(v["code"], "sync");
        assert_eq!(v["params"]["detail"], "Gmail returned 503");
    }

    #[test]
    fn parameterless_variants_emit_empty_params() {
        let v: serde_json::Value = serde_json::to_value(AppError::AiDisabled).expect("serialize");
        assert_eq!(v["code"], "ai_disabled");
        assert!(v["params"].as_object().unwrap().is_empty());
        assert!(v["message"].is_string());
    }

    #[test]
    fn invalid_input_carries_detail() {
        let err = AppError::InvalidInput("bad value".into());
        let v: serde_json::Value = serde_json::to_value(&err).expect("serialize");
        assert_eq!(v["code"], "invalid_input");
        assert_eq!(v["params"]["detail"], "bad value");
    }
}
