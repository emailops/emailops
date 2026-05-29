use crate::models::error::AppError;

pub fn model_not_loaded() -> AppError {
    AppError::AiError("No llama.cpp model loaded. Download and select a model in AI Settings.".to_string())
}

pub fn model_load_failed(path: &str, reason: &str) -> AppError {
    AppError::AiError(format!(
        "Failed to load model '{}': {}. Try re-downloading it.",
        path, reason
    ))
}
