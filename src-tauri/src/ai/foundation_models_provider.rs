//! Apple's on-device model as an [`AIProvider`].
//!
//! Deliberately a **narrow** backend. `docs/DECISIONS.md` ("iOS targets iOS 26")
//! settles what it is for: classification, tag and priority extraction, junk
//! detection, translation and per-email summaries — short, structured work with
//! a bounded prompt. It is explicitly *not* for chat over retrieved threads,
//! because the model's context is a fixed ~4096-token budget shared between
//! input and output, which cannot hold a thread plus its retrieved neighbours.
//!
//! Three of the ten trait methods are therefore refused outright rather than
//! faked:
//!
//! * **embeddings** — the framework has no embedding API at all. The provider
//!   declares `embeddings: false` and `ai::embedding_route` sends the work to
//!   the bundled local embedder instead, which is what keeps retrieval on-device
//!   on every iOS tier.
//! * **tools** — iOS 26's framework has no structured tool-calling this code can
//!   drive; the chat tool loop stays on a backend that does.
//! * **streaming** — the trait's default falls back to the blocking path.

use async_trait::async_trait;

use crate::ai::foundation_models::{apple_intelligence_status, generate_blocking, AfmError};
use crate::ai::provider::{
    AIProvider, AiMessage, BackendCapabilities, ChatStreamResult, CompletionOptions, CompletionResult, EmbeddingResult,
    ModelInfo, ModelPricing, ProviderType,
};
use crate::models::error::{AppError, Result};

/// Name reported to the UI and stored in the `ai_provider` preference.
pub const PROVIDER_ID: &str = "foundation-models";

/// What the model calls itself in usage records and the model picker.
const MODEL_NAME: &str = "Apple Intelligence";

/// Ceiling on what we will even attempt to send.
///
/// The framework's window is ~4096 tokens shared between prompt and response.
/// Rather than let a long email fail deep inside the framework, oversized
/// prompts are refused here with a message the caller can act on. Four
/// characters per token is the usual English rule of thumb and deliberately
/// pessimistic — being wrong in this direction costs a fallback, being wrong
/// the other way costs a mid-classification failure.
const MAX_PROMPT_CHARS: usize = 3_000 * 4;

pub struct FoundationModelsProvider;

impl FoundationModelsProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FoundationModelsProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a prompt is short enough to be worth sending.
///
/// Pure so the boundary is table-tested without a device.
pub fn prompt_fits(prompt_chars: usize) -> bool {
    prompt_chars <= MAX_PROMPT_CHARS
}

/// Turn an FFI failure into the app's error type, keeping the distinction the
/// user can act on.
fn map_afm_error(error: AfmError, detail: String) -> AppError {
    let message = match error {
        AfmError::Unavailable => format!("Apple Intelligence is unavailable: {detail}"),
        AfmError::GuardrailViolation => format!("Apple Intelligence refused this request: {detail}"),
        AfmError::ContextTooLong => "This text is too long for Apple's on-device model".to_string(),
        AfmError::Failed => format!("Apple Intelligence failed: {detail}"),
    };
    AppError::AiError(message)
}

#[async_trait]
impl AIProvider for FoundationModelsProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::FoundationModels
    }

    fn model_name(&self) -> &str {
        MODEL_NAME
    }

    fn embedding_model_name(&self) -> &str {
        ""
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            tools: false,
            streaming: false,
            embeddings: false,
        }
    }

    async fn is_available(&self) -> bool {
        apple_intelligence_status().is_available()
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: PROVIDER_ID.to_string(),
            name: MODEL_NAME.to_string(),
            // It runs on the user's own device: zero is a fact, not a placeholder.
            pricing: ModelPricing {
                prompt: 0.0,
                completion: 0.0,
                request: 0.0,
            },
        }])
    }

    async fn list_embedding_models(&self) -> Result<Vec<ModelInfo>> {
        // Not "none installed" — none possible. The empty list is what the
        // model picker shows, and `capabilities().embeddings` is what actually
        // routes the work elsewhere.
        Ok(Vec::new())
    }

    async fn complete(&self, prompt: &str, options: CompletionOptions) -> Result<CompletionResult> {
        if !prompt_fits(prompt.chars().count()) {
            return Err(AppError::AiError(
                "This text is too long for Apple's on-device model".to_string(),
            ));
        }

        let prompt = prompt.to_string();
        let temperature = options.temperature.unwrap_or(-1.0);
        let max_tokens = options.max_tokens.map(|t| t as i32).unwrap_or(0);

        // `generate_blocking` blocks until the model answers, so it must not run
        // on a runtime worker — a classifier batch would otherwise stall every
        // other task sharing that thread.
        let text = tokio::task::spawn_blocking(move || generate_blocking(&prompt, None, temperature, max_tokens))
            .await
            .map_err(|e| AppError::AiError(format!("Apple Intelligence task failed: {e}")))?
            .map_err(|(error, detail)| map_afm_error(error, detail))?;

        Ok(CompletionResult {
            text,
            model: MODEL_NAME.to_string(),
            // The framework reports no token counts, and inference on the
            // user's own device costs nothing. Zeroes here are facts.
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: 0.0,
        })
    }

    async fn embed(&self, _text: &str) -> Result<EmbeddingResult> {
        Err(AppError::AiError(
            "Apple's on-device model cannot create embeddings".to_string(),
        ))
    }

    async fn embed_batch(&self, _texts: &[String]) -> Result<Vec<EmbeddingResult>> {
        Err(AppError::AiError(
            "Apple's on-device model cannot create embeddings".to_string(),
        ))
    }

    async fn chat_with_tools(&self, _messages: &[AiMessage], _tools: &[serde_json::Value]) -> Result<AiMessage> {
        Err(AppError::AiError(
            "Apple's on-device model does not support tool calling".to_string(),
        ))
    }

    async fn chat_stream(
        &self,
        _messages: Vec<AiMessage>,
        _on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ChatStreamResult> {
        // Not a streaming gap to paper over: chat never routes here at all,
        // because a retrieved thread does not fit the context window.
        Err(AppError::AiError(
            "Apple's on-device model is not used for chat".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_flags_say_what_this_backend_cannot_do() {
        // These three flags are the whole point of the provider: each one routes
        // work somewhere else instead of failing at the point of use.
        let caps = FoundationModelsProvider::new().capabilities();
        assert!(!caps.embeddings, "no embedding API exists in the framework");
        assert!(!caps.tools);
        assert!(!caps.streaming);
    }

    #[test]
    fn a_short_structured_prompt_fits() {
        // A classification prompt with one email body is the design target.
        assert!(prompt_fits(0));
        assert!(prompt_fits(4_000));
        assert!(prompt_fits(MAX_PROMPT_CHARS));
    }

    #[test]
    fn a_retrieved_thread_does_not_fit() {
        // The reason chat never routes here: the window is shared between
        // prompt and response and cannot hold a thread plus its neighbours.
        assert!(!prompt_fits(MAX_PROMPT_CHARS + 1));
        assert!(!prompt_fits(200_000));
    }

    #[tokio::test]
    async fn embedding_is_refused_in_words_the_user_can_read() {
        let provider = FoundationModelsProvider::new();
        let err = provider.embed("hello").await.unwrap_err();
        assert!(err.to_string().contains("cannot create embeddings"), "got: {err}");
    }

    #[tokio::test]
    async fn an_oversized_prompt_is_refused_before_the_framework_is_asked() {
        // Fails locally rather than deep inside the framework, and without
        // needing a device: the length check runs before the FFI call.
        let provider = FoundationModelsProvider::new();
        let err = provider
            .complete(&"x".repeat(MAX_PROMPT_CHARS + 1), CompletionOptions::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("too long"), "got: {err}");
    }

    #[tokio::test]
    async fn it_reports_itself_unavailable_where_there_is_no_framework() {
        assert!(!FoundationModelsProvider::new().is_available().await);
    }
}
