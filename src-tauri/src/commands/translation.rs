//! Tauri commands for AI email translation.
//!
//! All three commands mirror the `generate_draft` shape: gate on the master
//! AI switch + the `ai_translation_enabled` feature toggle, return a
//! `request_id` immediately, run the work on a queue, and deliver the result
//! via a Tauri event keyed by that `request_id`.
//!
//! * `detect_email_language` → `ai_background` (lazy metadata; must never
//!   delay an interactive chat/draft task) → `language-detected`.
//! * `translate_email` / `translate_compose_text` → `ai_queue` (user-clicked,
//!   interactive) → `email-translated` / `compose-translated`, with
//!   `translation-failed` on error.

use tauri::{AppHandle, Emitter, State};

use crate::models::error::AppError;
use crate::services;
use crate::AppState;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageDetectedEvent {
    request_id: String,
    email_id: String,
    language: String,
    preferred_language: String,
    needs_translation: bool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EmailTranslatedEvent {
    request_id: String,
    email_id: String,
    target_language: String,
    text: String,
    truncated: bool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ComposeTranslatedEvent {
    request_id: String,
    target_language: String,
    text: String,
    truncated: bool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslationFailedEvent {
    request_id: String,
    email_id: String,
    error: String,
}

fn ensure_translation_enabled(state: &State<'_, AppState>) -> Result<(), AppError> {
    if !state.db.is_ai_enabled()? {
        return Err(AppError::AiDisabled);
    }
    if !state.db.is_ai_translation_enabled()? {
        return Err(AppError::InvalidInput("AI translation is disabled".into()));
    }
    Ok(())
}

fn emit_translation_failed(app: &AppHandle, request_id: &str, email_id: &str, error: &str) {
    if let Err(e) = app.emit(
        "translation-failed",
        TranslationFailedEvent {
            request_id: request_id.to_string(),
            email_id: email_id.to_string(),
            error: error.to_string(),
        },
    ) {
        crate::services::logger::log("error", "ai", format!("failed to emit translation-failed: {e}"));
    }
}

/// Detect the language of one email, lazily (fired when the reading view
/// expands an email). Result arrives on `language-detected`; failures are
/// logged at `debug` only — detection failing must never surface a banner,
/// the Translate button simply doesn't appear.
#[tauri::command]
pub async fn detect_email_language(
    app: AppHandle,
    state: State<'_, AppState>,
    email_id: String,
) -> Result<String, AppError> {
    ensure_translation_enabled(&state)?;

    let request_id = uuid::Uuid::new_v4().to_string();
    let db = state.db.clone();
    let app_for_task = app.clone();
    let request_id_for_task = request_id.clone();
    let task_label = format!("detect-lang:{email_id}");

    state
        .ai_background
        .submit_named(&task_label, async move {
            match services::translation::detect_email_language(&db, &email_id).await {
                Ok(result) => {
                    if let Err(e) = app_for_task.emit(
                        "language-detected",
                        LanguageDetectedEvent {
                            request_id: request_id_for_task,
                            email_id: result.email_id,
                            language: result.language,
                            preferred_language: result.preferred,
                            needs_translation: result.needs_translation,
                        },
                    ) {
                        crate::services::logger::log("error", "ai", format!("failed to emit language-detected: {e}"));
                    }
                }
                Err(err) => {
                    // Fail closed and quietly: no button, no banner.
                    crate::services::logger::log(
                        "debug",
                        "ai",
                        format!("language detection failed for {email_id}: {err}"),
                    );
                }
            }
        })
        .await;

    Ok(request_id)
}

/// Translate one email's body. `target_language: None` → the user's preferred
/// AI language. Result arrives on `email-translated` / `translation-failed`.
#[tauri::command]
pub async fn translate_email(
    app: AppHandle,
    state: State<'_, AppState>,
    email_id: String,
    target_language: Option<String>,
) -> Result<String, AppError> {
    ensure_translation_enabled(&state)?;

    let request_id = uuid::Uuid::new_v4().to_string();
    let db = state.db.clone();
    let app_for_task = app.clone();
    let request_id_for_task = request_id.clone();
    let task_label = format!("translate:{email_id}");

    state
        .ai_queue
        .submit_named(&task_label, async move {
            crate::services::logger::log("info", "ai", format!("translating email {email_id}…"));
            match services::translation::translate_email(&db, &email_id, target_language.as_deref()).await {
                Ok(result) => {
                    crate::services::logger::log(
                        "success",
                        "ai",
                        format!("email {email_id} translated to {}", result.target_language),
                    );
                    if let Err(e) = app_for_task.emit(
                        "email-translated",
                        EmailTranslatedEvent {
                            request_id: request_id_for_task,
                            email_id,
                            target_language: result.target_language,
                            text: result.text,
                            truncated: result.truncated,
                        },
                    ) {
                        crate::services::logger::log("error", "ai", format!("failed to emit email-translated: {e}"));
                    }
                }
                Err(err) => {
                    let message = err.to_string();
                    crate::services::logger::log(
                        "error",
                        "ai",
                        format!("translation failed for {email_id}: {message}"),
                    );
                    emit_translation_failed(&app_for_task, &request_id_for_task, &email_id, &message);
                }
            }
        })
        .await;

    Ok(request_id)
}

/// Translate compose-draft text into a target language (free text or ISO
/// code). Result arrives on `compose-translated` / `translation-failed`
/// (with an empty `emailId`, like `generate_new_draft`).
#[tauri::command]
pub async fn translate_compose_text(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    target_language: String,
) -> Result<String, AppError> {
    ensure_translation_enabled(&state)?;
    // Validate up front so obviously-bad input fails the command itself
    // instead of a delayed event (the queue path re-validates anyway).
    services::translation::sanitize_target_language(&target_language)?;

    let request_id = uuid::Uuid::new_v4().to_string();
    let db = state.db.clone();
    let app_for_task = app.clone();
    let request_id_for_task = request_id.clone();

    state
        .ai_queue
        .submit_named("translate-compose", async move {
            crate::services::logger::log("info", "ai", "translating compose draft…");
            match services::translation::translate_text(&db, &text, &target_language).await {
                Ok(result) => {
                    crate::services::logger::log(
                        "success",
                        "ai",
                        format!("compose draft translated to {}", result.target_language),
                    );
                    if let Err(e) = app_for_task.emit(
                        "compose-translated",
                        ComposeTranslatedEvent {
                            request_id: request_id_for_task,
                            target_language: result.target_language,
                            text: result.text,
                            truncated: result.truncated,
                        },
                    ) {
                        crate::services::logger::log("error", "ai", format!("failed to emit compose-translated: {e}"));
                    }
                }
                Err(err) => {
                    let message = err.to_string();
                    crate::services::logger::log("error", "ai", format!("compose translation failed: {message}"));
                    emit_translation_failed(&app_for_task, &request_id_for_task, "", &message);
                }
            }
        })
        .await;

    Ok(request_id)
}
