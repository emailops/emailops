use crate::{AppError, AppState};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub async fn get_pref(state: State<'_, AppState>, key: String) -> Result<Option<String>, AppError> {
    ensure_pref_key_exposed(&key)?;
    state.db.get_preference(&key)
}

/// Keys the backend owns outright and the generic pref IPC must never read or
/// write: `security.*` holds the main-password hash and its throttle counters
/// (writing them would remove the lock or reset the lockout without the
/// password — `commands::security` is the only way in), and `app_data_dir` is
/// written at startup and trusted for on-disk path resolution.
pub(crate) fn ensure_pref_key_exposed(key: &str) -> Result<(), AppError> {
    if key.starts_with("security.") || key == "app_data_dir" {
        return Err(AppError::InvalidInput(format!(
            "preference {key} is managed by the app and cannot be accessed here"
        )));
    }
    Ok(())
}

/// Validate a preference write before it hits the DB. Pure planner — no I/O,
/// no `AppState`. Splitting this out of the Tauri command makes it directly
/// unit-testable; the executor below just calls this and propagates the error.
///
/// Rejects backend-owned keys ([`ensure_pref_key_exposed`]) and enforces the language-pref allowlist (`ui_language` must be
/// one of `en`/`es`/`fr`/`de`; `ai_output_language_v2` accepts the same set
/// plus the empty string as the "Same as UI" sentinel resolved at read time).
/// Add more rules here as new typed preferences land.
pub(crate) fn validate_pref(key: &str, value: &str) -> Result<(), AppError> {
    use crate::services::i18n::{Language, PREF_AI_OUTPUT_LANGUAGE_V2, PREF_UI_LANGUAGE};

    ensure_pref_key_exposed(key)?;
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
    } else if key == "chat.n_ctx" {
        // Embedded llama.cpp context window. `0` = auto. Any other value must be
        // a token count in a generous absolute range; the real per-model clamp
        // to `[floor, model-trained]` happens at actor-spawn time in
        // `planner::effective_n_ctx`, so this only rejects nonsense early.
        const N_CTX_PREF_MIN: u32 = 1024;
        const N_CTX_PREF_MAX: u32 = 131_072;
        let parsed = value.parse::<u32>().map_err(|_| {
            AppError::InvalidInput(format!("chat.n_ctx must be a whole number of tokens, got: {value}"))
        })?;
        if parsed != 0 && !(N_CTX_PREF_MIN..=N_CTX_PREF_MAX).contains(&parsed) {
            return Err(AppError::InvalidInput(format!(
                "chat.n_ctx must be 0 (auto) or between {N_CTX_PREF_MIN} and {N_CTX_PREF_MAX}, got: {parsed}"
            )));
        }
    } else if key == crate::services::chat::research::REMOTE_N_CTX_BUDGET_PREF {
        // Prompt budget for remote (OpenRouter) models. `0` = default; the
        // model's own window still caps it at read time.
        const BUDGET_MIN: u32 = 4096;
        const BUDGET_MAX: u32 = 2_000_000;
        let parsed = value
            .parse::<u32>()
            .map_err(|_| AppError::InvalidInput(format!("{key} must be a whole number of tokens, got: {value}")))?;
        if parsed != 0 && !(BUDGET_MIN..=BUDGET_MAX).contains(&parsed) {
            return Err(AppError::InvalidInput(format!(
                "{key} must be 0 (default) or between {BUDGET_MIN} and {BUDGET_MAX}, got: {parsed}"
            )));
        }
    } else if key == "calendar_notify_minutes" {
        // Meeting-reminder lead time. The notifier clamps defensively at read
        // time too, but reject nonsense at the write boundary so Settings can
        // surface the error.
        const NOTIFY_MIN: i64 = 1;
        const NOTIFY_MAX: i64 = 120;
        let parsed = value.parse::<i64>().map_err(|_| {
            AppError::InvalidInput(format!("calendar_notify_minutes must be a whole number, got: {value}"))
        })?;
        if !(NOTIFY_MIN..=NOTIFY_MAX).contains(&parsed) {
            return Err(AppError::InvalidInput(format!(
                "calendar_notify_minutes must be between {NOTIFY_MIN} and {NOTIFY_MAX}, got: {parsed}"
            )));
        }
    } else if (key == "calendar_notifications_enabled" || key == crate::services::calendar::notify::SHOW_TITLE_PREF)
        && !matches!(value, "true" | "false")
    {
        return Err(AppError::InvalidInput(format!(
            "{key} must be true or false, got: {value}"
        )));
    } else if key.starts_with("calendar.enabled:") && !matches!(value, "true" | "false") {
        // Per-account calendar-integration opt-in (calendar.enabled:<account_id>).
        return Err(AppError::InvalidInput(format!(
            "calendar.enabled must be true or false, got: {value}"
        )));
    } else if is_new_mail_notification_switch(key) && !matches!(value, "true" | "false") {
        return Err(AppError::InvalidInput(format!(
            "{key} must be true or false, got: {value}"
        )));
    } else if key == crate::services::mail_notifications::PREF_CONTENT && !matches!(value, "preview" | "hidden") {
        return Err(AppError::InvalidInput(format!(
            "{key} must be preview or hidden, got: {value}"
        )));
    } else if key == crate::models::outbox::UNDO_SEND_DELAY_PREF {
        // Undo-send window: 0 (off) or one of the delays the outbox accepts.
        let valid = value
            .parse::<i64>()
            .is_ok_and(|secs| secs == 0 || crate::models::outbox::UNDO_SEND_DELAYS.contains(&secs));
        if !valid {
            return Err(AppError::InvalidInput(format!(
                "{key} must be 0 or one of {:?} seconds, got: {value}",
                crate::models::outbox::UNDO_SEND_DELAYS
            )));
        }
    }
    Ok(())
}

/// The boolean new-mail notification switches: master, per-account, focus.
fn is_new_mail_notification_switch(key: &str) -> bool {
    use crate::services::mail_notifications as n;
    key == n::PREF_ENABLED || key == n::PREF_ONLY_UNFOCUSED || key.starts_with(n::PREF_ACCOUNT_PREFIX)
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
    if matches!(
        key.as_str(),
        "ai_provider" | "ai_model" | "ai_embedding_model" | "chat.n_ctx"
    ) {
        let _ = app.emit("ai-config-updated", serde_json::Value::Null);
    }
    Ok(())
}

/// The context window the embedded runtime picks on THIS machine when
/// `chat.n_ctx` is unset/0 (RAM tier: 8192 / 16384 / 32768, hard-capped at
/// 32k). The Settings UI shows it as the auto default so saving without
/// touching the field never downgrades the machine's auto choice. Per-model
/// clamps (trained context, KV fit next to the weights) still happen at
/// actor-spawn time — this is the machine tier only.
#[tauri::command]
pub async fn get_auto_n_ctx() -> Result<u32, AppError> {
    Ok(crate::util::system::auto_n_ctx_tier(
        crate::util::system::total_ram_bytes(),
    ))
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
    fn the_undo_send_delay_is_off_or_an_offered_window() {
        for ok in ["0", "5", "10", "20", "30"] {
            assert!(validate_pref("compose.undo_send_delay_secs", ok).is_ok(), "{ok}");
        }
        for bad in ["7", "-5", "60", "ten", ""] {
            assert!(validate_pref("compose.undo_send_delay_secs", bad).is_err(), "{bad}");
        }
    }

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
    fn validate_pref_accepts_a_remote_context_budget_or_the_default() {
        for v in ["0", "4096", "32768", "200000"] {
            assert!(
                validate_pref("chat.remote_n_ctx_budget", v).is_ok(),
                "should accept {v}"
            );
        }
    }

    #[test]
    fn validate_pref_rejects_a_nonsense_remote_context_budget() {
        for v in ["lots", "-1", "512", "99999999"] {
            let err = validate_pref("chat.remote_n_ctx_budget", v).unwrap_err();
            assert!(matches!(err, AppError::InvalidInput(_)), "should reject {v}");
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

    #[test]
    fn validate_pref_accepts_valid_n_ctx() {
        // 0 = auto sentinel; the floor, a mid value, and the absolute ceiling.
        for v in ["0", "1024", "8192", "32768", "131072"] {
            assert!(validate_pref("chat.n_ctx", v).is_ok(), "should accept {v}");
        }
    }

    #[test]
    fn validate_pref_rejects_out_of_range_n_ctx() {
        // Below the floor (but non-zero) and above the absolute ceiling.
        for v in ["1", "1023", "200000"] {
            let err = validate_pref("chat.n_ctx", v).unwrap_err();
            assert!(
                matches!(err, AppError::InvalidInput(_)),
                "should reject {v} as InvalidInput"
            );
        }
    }

    #[test]
    fn validate_pref_accepts_valid_calendar_notify_minutes() {
        for v in ["1", "10", "30", "120"] {
            assert!(validate_pref("calendar_notify_minutes", v).is_ok(), "should accept {v}");
        }
    }

    #[test]
    fn validate_pref_rejects_out_of_range_calendar_notify_minutes() {
        for v in ["0", "121", "-5", "soon"] {
            let err = validate_pref("calendar_notify_minutes", v).unwrap_err();
            assert!(
                matches!(err, AppError::InvalidInput(_)),
                "should reject {v} as InvalidInput"
            );
        }
    }

    #[test]
    fn validate_pref_calendar_notification_show_title_must_be_boolean() {
        let key = crate::services::calendar::notify::SHOW_TITLE_PREF;
        assert!(validate_pref(key, "true").is_ok());
        assert!(validate_pref(key, "false").is_ok());
        assert!(matches!(validate_pref(key, "maybe"), Err(AppError::InvalidInput(_))));
    }

    #[test]
    fn validate_pref_calendar_notifications_enabled_must_be_boolean() {
        assert!(validate_pref("calendar_notifications_enabled", "true").is_ok());
        assert!(validate_pref("calendar_notifications_enabled", "false").is_ok());
        let err = validate_pref("calendar_notifications_enabled", "yes").unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[test]
    fn validate_pref_new_mail_notification_prefs() {
        use crate::services::mail_notifications as n;
        let account_key = format!("{}acc1", n::PREF_ACCOUNT_PREFIX);
        for key in [n::PREF_ENABLED, n::PREF_ONLY_UNFOCUSED, account_key.as_str()] {
            assert!(validate_pref(key, "true").is_ok());
            assert!(validate_pref(key, "false").is_ok());
            assert!(
                matches!(validate_pref(key, "yes"), Err(AppError::InvalidInput(_))),
                "{key}"
            );
        }
        assert!(validate_pref(n::PREF_CONTENT, "preview").is_ok());
        assert!(validate_pref(n::PREF_CONTENT, "hidden").is_ok());
        assert!(matches!(
            validate_pref(n::PREF_CONTENT, "body"),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn validate_pref_per_account_calendar_enabled_must_be_boolean() {
        assert!(validate_pref("calendar.enabled:acc1", "true").is_ok());
        assert!(validate_pref("calendar.enabled:acc1", "false").is_ok());
        let err = validate_pref("calendar.enabled:acc1", "yes").unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[test]
    fn reserved_pref_keys_are_rejected_for_reads() {
        for key in [
            "security.main_password_hash",
            "security.main_password_failed_attempts",
            "security.main_password_last_failed_at",
            "app_data_dir",
        ] {
            let err = ensure_pref_key_exposed(key).unwrap_err();
            assert!(matches!(err, AppError::InvalidInput(_)), "should reject read of {key}");
        }
    }

    #[test]
    fn reserved_pref_keys_are_rejected_for_writes() {
        // Clearing the hash (empty value) or resetting the throttle counter
        // through the generic IPC would bypass the lock screen.
        for (key, value) in [
            ("security.main_password_hash", ""),
            ("security.main_password_failed_attempts", "0"),
            ("security.main_password_last_failed_at", "0"),
            ("app_data_dir", "/tmp/elsewhere"),
        ] {
            let err = validate_pref(key, value).unwrap_err();
            assert!(matches!(err, AppError::InvalidInput(_)), "should reject write of {key}");
        }
    }

    #[test]
    fn ordinary_pref_keys_are_exposed() {
        assert!(ensure_pref_key_exposed("ai_provider").is_ok());
        assert!(ensure_pref_key_exposed("privacy.allow_remote_content").is_ok());
    }

    #[test]
    fn validate_pref_rejects_non_numeric_n_ctx() {
        let err = validate_pref("chat.n_ctx", "lots").unwrap_err();
        match err {
            AppError::InvalidInput(msg) => assert!(msg.contains("chat.n_ctx")),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }
}
