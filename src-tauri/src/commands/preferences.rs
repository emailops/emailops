use crate::{AppError, AppState};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub async fn get_pref(state: State<'_, AppState>, key: String) -> Result<Option<String>, AppError> {
    state.db.get_preference(&key)
}

/// Validate a preference write before it hits the DB. Pure planner — no I/O,
/// no `AppState`. Splitting this out of the Tauri command makes it directly
/// unit-testable; the executor below just calls this and propagates the error.
///
/// Today this only enforces the language-pref allowlist (`ui_language` must be
/// one of `en`/`es`/`fr`/`de`; `ai_output_language_v2` accepts the same set
/// plus the empty string as the "Same as UI" sentinel resolved at read time).
/// Add more rules here as new typed preferences land.
pub(crate) fn validate_pref(key: &str, value: &str) -> Result<(), AppError> {
    use crate::services::i18n::{Language, PREF_AI_OUTPUT_LANGUAGE_V2, PREF_UI_LANGUAGE};

    if key == PREF_UI_LANGUAGE {
        if !value.is_empty() && Language::from_pref(value).is_none() {
            return Err(AppError::InvalidInput(format!(
                "unsupported ui_language value: {value} (expected en/es/fr/de)"
            )));
        }
    } else if key == PREF_AI_OUTPUT_LANGUAGE_V2 && !value.is_empty() && Language::from_pref(value).is_none() {
        return Err(AppError::InvalidInput(format!(
            "unsupported ai_output_language_v2 value: {value} (expected en/es/fr/de or empty)"
        )));
    }
    Ok(())
}

#[tauri::command]
pub async fn set_pref(app: AppHandle, state: State<'_, AppState>, key: String, value: String) -> Result<(), AppError> {
    validate_pref(&key, &value)?;
    state.db.set_preference(&key, &value)?;

    // When AI provider/model preferences change (e.g. the LogPanel quick
    // selector flips backend), broadcast `ai-config-updated` so AI Settings,
    // the LogPanel itself, and any other listener re-read the live config.
    // Background tasks always resolve the provider on execution, so they
    // automatically pick up the new backend on their next run.
    if matches!(key.as_str(), "ai_provider" | "ai_model" | "ai_embedding_model") {
        let _ = app.emit("ai-config-updated", serde_json::Value::Null);
    }
    Ok(())
}

/// Called by the frontend once the React tree has mounted so the window is only
/// revealed after the UI is ready, avoiding the transparent blank-window flash.
#[tauri::command]
pub async fn show_main_window(window: tauri::WebviewWindow) -> Result<(), AppError> {
    window.show().map_err(|e| AppError::IoError(e.to_string()))
}

/// Return the OS-level locale mapped to a supported [`Language`] code
/// (`"en"`, `"es"`, `"fr"`, `"de"`). Falls back to `"en"` when the locale is
/// missing, unparseable, or names an unsupported language.
///
/// The frontend calls this once at startup, before i18next is initialised, to
/// pick a default UI language when the user has not explicitly set
/// `ui_language` in preferences. Storing the value is the caller's job — this
/// command only reads.
#[tauri::command]
pub async fn get_system_locale() -> Result<String, AppError> {
    let raw = sys_locale::get_locale();
    let lang = raw
        .as_deref()
        .and_then(crate::services::i18n::Language::from_pref)
        .unwrap_or_default();
    Ok(lang.as_code().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_pref_accepts_supported_ui_languages() {
        for code in ["en", "es", "fr", "de"] {
            assert!(validate_pref("ui_language", code).is_ok(), "should accept {code}");
        }
    }

    #[test]
    fn validate_pref_accepts_bcp47_ui_language_tags() {
        assert!(validate_pref("ui_language", "en-US").is_ok());
        assert!(validate_pref("ui_language", "es-MX").is_ok());
    }

    #[test]
    fn validate_pref_rejects_unknown_ui_language() {
        let err = validate_pref("ui_language", "pt").unwrap_err();
        match err {
            AppError::InvalidInput(msg) => assert!(msg.contains("ui_language")),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn validate_pref_rejects_unsupported_ai_language() {
        let err = validate_pref("ai_output_language_v2", "Italian").unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[test]
    fn validate_pref_allows_empty_ai_language_sentinel() {
        // Empty means "Same as UI" — resolved at read time, not stored as a Language.
        assert!(validate_pref("ai_output_language_v2", "").is_ok());
    }

    #[test]
    fn validate_pref_ignores_unrelated_keys() {
        // Non-language prefs must not be language-validated.
        assert!(validate_pref("ai_provider", "openrouter").is_ok());
        assert!(validate_pref("ui_density", "comfortable").is_ok());
    }
}
