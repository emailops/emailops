use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

use crate::models::error::AppError;
use crate::models::AiUsageSummary;
use crate::services;
use crate::services::ai::AiService;
use crate::services::embeddings::EmbeddingsConfig;
use crate::AppState;

fn emit_log(_app: &AppHandle, level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

#[tauri::command]
pub async fn get_ai_config(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let config = services::ai::AiService::get_config(&state.db)?;
    let has_api_key = services::ai::AiService::has_openrouter_api_key(&state.db)?;
    let validated_embedding_model = AiService::validated_openrouter_embedding_model(&state.db)?;
    let remembered: serde_json::Map<String, serde_json::Value> = AiService::remembered_models(&state.db, &config)?
        .into_iter()
        .map(|(provider, models)| {
            (
                provider.to_string(),
                serde_json::json!({ "model": models.model, "embeddingModel": models.embedding_model }),
            )
        })
        .collect();

    Ok(serde_json::json!({
        "provider": config.provider,
        "model": config.model,
        "embeddingModel": config.embedding_model,
        "openRouterValidatedEmbeddingModel": validated_embedding_model,
        "remembered": remembered,
        "monthlyBudgetUsd": config.monthly_budget_usd,
        "periodStart": config.period_start,
        "hasApiKey": has_api_key,
        "thinkingEnabled": config.thinking_enabled,
        "zeroDataRetention": config.zero_data_retention,
    }))
}

#[tauri::command]
pub async fn set_ai_config(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: String,
    model: String,
    embedding_model: Option<String>,
    api_key: Option<String>,
    monthly_budget_usd: f64,
    thinking_enabled: Option<bool>,
    zero_data_retention: Option<bool>,
) -> Result<(), AppError> {
    // If no API key is being written, keychain is not touched — safe to call directly.
    let result = if api_key.is_none() {
        services::ai::AiService::save_config(
            &state.db,
            &provider,
            &model,
            embedding_model.as_deref(),
            None,
            monthly_budget_usd,
            thinking_enabled,
            zero_data_retention,
        )
    } else {
        // Keychain writes can block on a macOS permission prompt; use a dedicated
        // blocking thread so the async runtime is never stalled.
        let db = state.db.clone();
        let task = tauri::async_runtime::spawn_blocking(move || {
            services::ai::AiService::save_config(
                &db,
                &provider,
                &model,
                embedding_model.as_deref(),
                api_key.as_deref(),
                monthly_budget_usd,
                thinking_enabled,
                zero_data_retention,
            )
        });

        match tokio::time::timeout(Duration::from_secs(8), task).await {
            Ok(Ok(r)) => r,
            Ok(Err(join_error)) => Err(AppError::AiError(format!("AI config save task failed: {}", join_error))),
            Err(_) => Err(AppError::AiError(
                "Saving AI config timed out. Check for a macOS Keychain permission prompt.".to_string(),
            )),
        }
    };

    // Notify the rest of the app (LogPanel selectors, AI Settings, etc.) so
    // they re-read the new provider/model immediately. Background tasks always
    // resolve the provider on execution, so they pick up changes too.
    if result.is_ok() {
        let _ = app.emit("ai-config-updated", serde_json::Value::Null);
    }

    result
}

#[tauri::command]
pub async fn get_ai_usage(state: State<'_, AppState>) -> Result<AiUsageSummary, AppError> {
    // Straight to the database: reading a spend counter must not build a
    // provider. Going through `AiService::new` made this fail whenever the
    // master AI switch was off — exactly when a user who had hit their budget
    // would come looking — and on a llama.cpp setup it loaded the model into
    // RAM to read an integer.
    AiService::usage_summary(&state.db)
}

#[tauri::command]
pub async fn reset_ai_usage(state: State<'_, AppState>) -> Result<(), AppError> {
    AiService::reset_usage_period(&state.db)
}

#[tauri::command]
pub async fn list_ai_models(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, AppError> {
    let service = AiService::new(state.db.clone())?;
    let models = service.list_models().await?;
    Ok(models
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "pricing": {
                    "prompt": m.pricing.prompt,
                    "completion": m.pricing.completion,
                    "request": m.pricing.request,
                }
            })
        })
        .collect())
}

/// Embedding models of `provider` — the saved provider when omitted. Settings
/// passes the tab being edited, which may not be saved yet.
#[tauri::command]
pub async fn list_ai_embedding_models(
    state: State<'_, AppState>,
    provider: Option<String>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let config = services::ai::AiService::get_config(&state.db)?;

    if provider.as_deref().unwrap_or(&config.provider) == "openrouter" {
        let key = AiService::load_openrouter_api_key(&state.db)?;
        let client =
            crate::ai::openrouter::OpenRouterClient::new(key, config.model.clone(), config.embedding_model.clone());
        let models = client.list_embedding_models_from_api().await?;
        return Ok(models
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "name": m.name,
                    "pricing": {
                        "prompt": m.pricing.prompt,
                        "completion": m.pricing.completion,
                        "request": m.pricing.request,
                    }
                })
            })
            .collect());
    }

    let ollama = crate::ai::ollama::OllamaClient::new(None);
    let models = ollama
        .list_model_names()
        .await?
        .into_iter()
        .filter(|id| {
            let lower = id.to_lowercase();
            lower.contains("embed")
                || lower.contains("embedding")
                || lower.contains("nomic")
                || lower.contains("bge")
                || lower.contains("e5")
        })
        .map(|id| {
            serde_json::json!({
                "id": id,
                "name": id,
                "pricing": {
                    "prompt": 0.0,
                    "completion": 0.0,
                    "request": 0.0,
                }
            })
        })
        .collect();
    Ok(models)
}

/// Check that an OpenRouter embedding model produces vectors the email index
/// can hold, and remember it when it does. Settings calls this before saving
/// a newly chosen model; until a model has passed, no embedding request is
/// sent to OpenRouter. `api_key` and `zero_data_retention` carry values typed
/// in Settings but not saved yet.
#[tauri::command]
pub async fn validate_openrouter_embedding_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
    api_key: Option<String>,
    zero_data_retention: Option<bool>,
) -> Result<(), AppError> {
    let model = model.trim().to_string();
    if model.is_empty() {
        return Err(AppError::InvalidInput("No embedding model was given".to_string()));
    }
    emit_log(
        &app,
        "info",
        "embeddings",
        &format!("Checking embedding model {model}…"),
    );

    let config = AiService::get_config(&state.db)?;
    let key = match api_key {
        Some(key) if !key.is_empty() => key,
        _ => AiService::load_openrouter_api_key(&state.db)?,
    };
    let client = crate::ai::openrouter::OpenRouterClient::new(key, config.model, model.clone())
        .with_zero_data_retention(zero_data_retention.unwrap_or(config.zero_data_retention));

    match AiService::validate_openrouter_embedding_model(&state.db, &client).await {
        Ok(_) => {
            emit_log(
                &app,
                "success",
                "embeddings",
                &format!("Embedding model {model} fits the email index"),
            );
            Ok(())
        }
        Err(e) => {
            emit_log(
                &app,
                "error",
                "embeddings",
                &format!("Embedding model {model} cannot be used: {e}"),
            );
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn get_embeddings_config(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<EmbeddingsConfig, AppError> {
    services::embeddings::get_embeddings_config(&state.db, &account_id)
}

#[tauri::command]
pub async fn set_embeddings_config(
    state: State<'_, AppState>,
    account_id: String,
    config: EmbeddingsConfig,
) -> Result<(), AppError> {
    services::embeddings::save_embeddings_config(&state.db, &account_id, &config)
}

#[tauri::command]
pub async fn check_ai_available(state: State<'_, AppState>) -> Result<bool, AppError> {
    let service = AiService::new(state.db.clone())?;
    Ok(service.is_available().await)
}

#[tauri::command]
pub async fn test_ai_provider(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: String,
    model: String,
    api_key: Option<String>,
) -> Result<String, AppError> {
    emit_log(&app, "info", "ai", &format!("Testing {provider} ({model})..."));

    let prov: Arc<dyn crate::ai::provider::AIProvider> = if provider == "openrouter" {
        let key = match api_key {
            Some(key) if !key.is_empty() => key,
            _ => AiService::load_openrouter_api_key(&state.db)?,
        };
        let zdr = AiService::get_config(&state.db)?.zero_data_retention;
        Arc::new(
            crate::ai::openrouter::OpenRouterClient::new(key, model, "openai/text-embedding-3-small".to_string())
                .with_zero_data_retention(zdr),
        )
    } else {
        // Covers "ollama", "llamacpp", and any future provider.
        // build_provider resolves GGUF paths via DB preferences for llamacpp.
        services::ai::AiService::build_provider(&state.db, &provider, &model)?
    };

    let result = prov.complete("Reply with exactly: OK", Default::default()).await?;

    emit_log(
        &app,
        "success",
        "ai",
        &format!(
            "AI provider test successful: {}",
            result.text.chars().take(50).collect::<String>()
        ),
    );
    Ok(result.text)
}
