// Embedded llama.cpp backend — only compiled when the `llamacpp` feature is on.
//
// Architecture:
//   runtime.rs    — global LlamaBackend singleton + model load/swap
//   streaming.rs  — spawn_blocking + mpsc bridge for async streaming
//   embeddings.rs — encoder-mode inference + mean pooling
//   errors.rs     — map llama-cpp-2 errors → AppError

pub mod errors;
pub mod runtime;

use std::sync::Arc;

use async_trait::async_trait;

use crate::ai::provider::{
    AIProvider, AiMessage, BackendCapabilities, ChatStreamResult, CompletionOptions, CompletionResult, EmbeddingResult,
    ModelInfo, ModelPricing, ProviderType,
};
use crate::models::error::Result;

use self::runtime::LlamaCppRuntime;

/// Embedded llama.cpp AI backend.
///
/// Wraps a shared `LlamaCppRuntime` that owns the loaded models.
/// Multiple `LlamaCppBackend` instances can share the same runtime;
/// only one model is loaded at a time and swapped on change.
pub struct LlamaCppBackend {
    runtime: Arc<LlamaCppRuntime>,
    model_name: String,
    embedding_model_name: String,
}

impl LlamaCppBackend {
    pub fn new(runtime: Arc<LlamaCppRuntime>, model_name: String, embedding_model_name: String) -> Self {
        Self {
            runtime,
            model_name,
            embedding_model_name,
        }
    }
}

#[async_trait]
impl AIProvider for LlamaCppBackend {
    fn provider_type(&self) -> ProviderType {
        ProviderType::LlamaCpp
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn embedding_model_name(&self) -> &str {
        &self.embedding_model_name
    }

    async fn is_available(&self) -> bool {
        self.runtime.is_ready()
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        // For llama.cpp the "available models" are the local GGUFs on disk.
        // Delegated to the model manager; here we return just the currently
        // configured chat model so the settings UI can show it.
        let name = self.model_name.clone();
        if name.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![ModelInfo {
            id: name.clone(),
            name,
            pricing: ModelPricing {
                prompt: 0.0,
                completion: 0.0,
                request: 0.0,
            },
        }])
    }

    async fn list_embedding_models(&self) -> Result<Vec<ModelInfo>> {
        let name = self.embedding_model_name.clone();
        if name.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![ModelInfo {
            id: name.clone(),
            name,
            pricing: ModelPricing {
                prompt: 0.0,
                completion: 0.0,
                request: 0.0,
            },
        }])
    }

    async fn complete(&self, prompt: &str, options: CompletionOptions) -> Result<CompletionResult> {
        let text = self.runtime.generate(prompt, &options).await?;
        Ok(CompletionResult {
            text,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: 0.0,
            model: self.model_name.clone(),
        })
    }

    async fn embed(&self, text: &str) -> Result<EmbeddingResult> {
        let embedding = self.runtime.embed(text).await?;
        Ok(EmbeddingResult {
            embedding,
            tokens: 0,
            cost_usd: 0.0,
        })
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<EmbeddingResult>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }

    async fn chat_with_tools(&self, messages: &[AiMessage], tools: &[serde_json::Value]) -> Result<AiMessage> {
        self.runtime.chat_with_tools(messages, tools).await
    }

    async fn chat_stream(
        &self,
        messages: Vec<AiMessage>,
        on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ChatStreamResult> {
        self.runtime.chat_stream(messages, on_token).await
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            tools: true,
            streaming: true,
            embeddings: true,
        }
    }

    /// Run a tiny 1-token generation so the GGUF is fully loaded and any
    /// lazy Metal buffers are allocated. Skipped if no chat model has been
    /// configured yet (clean install before the user picks a model).
    async fn warmup(&self) -> Result<()> {
        if !self.runtime.is_ready() {
            return Ok(());
        }
        self.runtime.warmup_chat().await
    }
}
