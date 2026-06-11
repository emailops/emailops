use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    Ollama,
    OpenRouter,
    LlamaCpp,
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::Ollama => write!(f, "ollama"),
            ProviderType::OpenRouter => write!(f, "openrouter"),
            ProviderType::LlamaCpp => write!(f, "llamacpp"),
        }
    }
}

// ── Shared message types ─────────────────────────────────────────────────────

/// A single message in an AI conversation, provider-neutral.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<AiToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiToolCall {
    pub function: AiToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiToolCallFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Result from a streaming chat completion.
#[derive(Debug, Clone)]
pub struct ChatStreamResult {
    pub content: String,
    pub eval_count: Option<u32>,
    pub prompt_eval_count: Option<u32>,
    /// Prompt prefill wall-clock time. Only reported by the embedded
    /// llama.cpp backend; HTTP providers leave it `None`.
    pub prefill_ms: Option<i64>,
    /// Prompt tokens served from a reused KV-cache prefix (0 until prefix
    /// caching lands; `None` when the backend can't know).
    pub cached_prompt_tokens: Option<u32>,
}

/// Result from a streaming chat completion that may also carry tool calls.
///
/// `message` is the fully-accumulated assistant turn: `content` holds the prose
/// that was streamed to the user (empty when the turn resolved to a tool call),
/// and `tool_calls` holds any structured calls the caller must resolve. Token
/// counts mirror [`ChatStreamResult`].
#[derive(Debug, Clone)]
pub struct ToolStreamResult {
    pub message: AiMessage,
    pub eval_count: Option<u32>,
    pub prompt_eval_count: Option<u32>,
    /// See [`ChatStreamResult::prefill_ms`].
    pub prefill_ms: Option<i64>,
    /// See [`ChatStreamResult::cached_prompt_tokens`].
    pub cached_prompt_tokens: Option<u32>,
}

/// Capability flags for a backend. Higher-level code can use these to
/// gracefully degrade when a backend doesn't support a feature.
#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    /// Supports structured tool-calling (function calling).
    pub tools: bool,
    /// Supports token streaming.
    pub streaming: bool,
    /// Supports embedding generation.
    pub embeddings: bool,
}

impl Default for BackendCapabilities {
    fn default() -> Self {
        Self {
            tools: true,
            streaming: true,
            embeddings: true,
        }
    }
}

// ── Completion types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct CompletionOptions {
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub think: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionResult {
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub model: String,
}

#[derive(Debug, Clone, Default)]
pub struct EmbeddingResult {
    pub embedding: Vec<f32>,
    pub tokens: u32,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub pricing: ModelPricing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricing {
    pub prompt: f64,
    pub completion: f64,
    pub request: f64,
}

// ── Trait ────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait AIProvider: Send + Sync {
    fn provider_type(&self) -> ProviderType;
    fn model_name(&self) -> &str;
    fn embedding_model_name(&self) -> &str;

    async fn is_available(&self) -> bool;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
    /// List models suitable for embedding generation.
    async fn list_embedding_models(&self) -> Result<Vec<ModelInfo>>;

    /// Non-streaming single-turn completion (prompt → text).
    async fn complete(&self, prompt: &str, options: CompletionOptions) -> Result<CompletionResult>;

    /// Generate a single embedding vector.
    async fn embed(&self, text: &str) -> Result<EmbeddingResult>;
    /// Generate embeddings for a batch of texts (may parallelize internally).
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<EmbeddingResult>>;

    /// Non-streaming multi-turn chat with tool definitions. Returns the
    /// assistant message (which may contain tool_calls the caller should resolve).
    async fn chat_with_tools(&self, messages: &[AiMessage], tools: &[serde_json::Value]) -> Result<AiMessage>;

    /// Streaming chat. `on_token` is called for each text chunk (owned String);
    /// returning `false` cancels the stream. Returns the full accumulated content.
    async fn chat_stream(
        &self,
        messages: Vec<AiMessage>,
        on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ChatStreamResult>;

    /// Streaming multi-turn chat WITH tool definitions. Streams assistant
    /// *prose* via `on_token` (returning `false` cancels) while still returning
    /// any structured `tool_calls` in the final `ToolStreamResult.message`.
    ///
    /// This unifies `chat_with_tools` (tools, no streaming) and `chat_stream`
    /// (streaming, no tools): the tool round can now stream synthesized prose to
    /// the user without losing the ability to dispatch tool calls.
    ///
    /// When a turn resolves to tool calls, NO prose tokens are emitted — a
    /// model's tool-call syntax / planning must never leak into the user-visible
    /// stream. Providers that expose tool_calls structurally (Ollama,
    /// OpenRouter) stream content live and accumulate tool_calls separately;
    /// llama.cpp parses tool-call syntax out of the token stream and so buffers
    /// the leading tokens until it can tell prose from a tool call.
    ///
    /// The default impl falls back to the blocking `chat_with_tools` and emits
    /// the resulting prose as a single chunk — correct, just not incremental.
    async fn chat_stream_with_tools(
        &self,
        messages: Vec<AiMessage>,
        tools: Vec<serde_json::Value>,
        mut on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ToolStreamResult> {
        let msg = self.chat_with_tools(&messages, &tools).await?;
        let has_tool_calls = msg.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty());
        if !has_tool_calls && !msg.content.is_empty() {
            let _ = on_token(msg.content.clone());
        }
        Ok(ToolStreamResult {
            message: msg,
            eval_count: None,
            prompt_eval_count: None,
            prefill_ms: None,
            cached_prompt_tokens: None,
        })
    }

    /// Capability flags. Used to decide whether to use tool-calling vs RAG-only,
    /// streaming vs buffered, etc.
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::default()
    }

    /// Best-effort pre-warm: fire a tiny request so the model's weights are
    /// resident and any lazy allocations happen before the user's first turn.
    ///
    /// Backends that incur a heavy model-load cost (local llama.cpp, fresh
    /// Ollama processes) override this to actually run a minimal inference.
    /// Remote backends (OpenRouter) may leave the default no-op impl since
    /// they have nothing to warm up. Errors are intentionally swallowed —
    /// warmup failing must never block app startup.
    async fn warmup(&self) -> Result<()> {
        Ok(())
    }
}

// ── Fake AI provider for tests ───────────────────────────────────────────────
//
// Lives in the production crate (not `#[cfg(test)]`) so integration tests and
// eval harnesses can use it without enabling cargo test mode. Tests that
// exercise AI-driven services should depend on `FakeAiProvider` instead of
// stubbing reqwest / Ollama HTTP.

use std::sync::{PoisonError, RwLock};

/// Deterministic in-memory `AIProvider` for tests. By default returns a fixed
/// canned response for every completion call; tests can pre-load specific
/// responses via [`push_completion`] / [`push_chat_response`].
///
/// Embeddings are derived from a SHA-256 of the input so they are stable
/// across runs but distinguish inputs.
pub struct FakeAiProvider {
    model: String,
    embedding_model: String,
    available: RwLock<bool>,
    /// FIFO of canned completion responses. When empty, falls back to
    /// `default_completion`.
    completions: RwLock<std::collections::VecDeque<CompletionResult>>,
    default_completion: RwLock<CompletionResult>,
    /// FIFO of canned chat responses. When empty, falls back to an empty
    /// assistant message.
    chats: RwLock<std::collections::VecDeque<AiMessage>>,
    /// Calls recorded for later assertion.
    completion_calls: RwLock<Vec<String>>,
    chat_calls: RwLock<Vec<Vec<AiMessage>>>,
    embed_calls: RwLock<Vec<String>>,
}

impl FakeAiProvider {
    pub fn new() -> Self {
        Self {
            model: "fake-model".to_string(),
            embedding_model: "fake-embed-model".to_string(),
            available: RwLock::new(true),
            completions: RwLock::new(std::collections::VecDeque::new()),
            default_completion: RwLock::new(CompletionResult {
                text: String::new(),
                prompt_tokens: 0,
                completion_tokens: 0,
                cost_usd: 0.0,
                model: "fake-model".to_string(),
            }),
            chats: RwLock::new(std::collections::VecDeque::new()),
            completion_calls: RwLock::new(Vec::new()),
            chat_calls: RwLock::new(Vec::new()),
            embed_calls: RwLock::new(Vec::new()),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn set_available(&self, available: bool) {
        *self.available.write().unwrap_or_else(PoisonError::into_inner) = available;
    }

    /// Queue a canned completion. Returned in FIFO order from `complete`.
    pub fn push_completion(&self, text: impl Into<String>) {
        self.completions
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(CompletionResult {
                text: text.into(),
                prompt_tokens: 0,
                completion_tokens: 0,
                cost_usd: 0.0,
                model: self.model.clone(),
            });
    }

    /// Queue a canned chat response. Returned in FIFO order from
    /// `chat_with_tools`.
    pub fn push_chat_response(&self, content: impl Into<String>) {
        self.chats
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(AiMessage {
                role: "assistant".to_string(),
                content: content.into(),
                tool_calls: None,
            });
    }

    /// Queue a fully-formed canned chat response (e.g. one carrying
    /// `tool_calls`). Returned in FIFO order from `chat_with_tools` /
    /// `chat_stream_with_tools`.
    pub fn push_chat_message(&self, msg: AiMessage) {
        self.chats
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(msg);
    }

    /// Every prompt passed to `complete`, in call order.
    pub fn completion_calls(&self) -> Vec<String> {
        self.completion_calls
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every message list passed to `chat_with_tools`, in call order.
    pub fn chat_calls(&self) -> Vec<Vec<AiMessage>> {
        self.chat_calls.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Every text passed to `embed` (single or batch), in call order.
    pub fn embed_calls(&self) -> Vec<String> {
        self.embed_calls.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Deterministic 8-dimensional embedding derived from a SHA-256 of `text`.
    /// Same input → same vector; distinct inputs almost always produce distinct
    /// vectors, which is enough for tests that need to assert similarity ranking.
    fn deterministic_embedding(text: &str) -> Vec<f32> {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(text.as_bytes());
        let mut out = Vec::with_capacity(8);
        for chunk in digest.chunks(4).take(8) {
            // SHA-256 output is exactly 32 bytes → all 8 chunks are exactly
            // 4 bytes. `try_into()` is infallible by construction.
            #[allow(clippy::unwrap_used)]
            let bits = u32::from_le_bytes(chunk.try_into().unwrap());
            // Map u32 → [-1.0, 1.0).
            let f = (bits as f32 / u32::MAX as f32) * 2.0 - 1.0;
            out.push(f);
        }
        out
    }
}

impl Default for FakeAiProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AIProvider for FakeAiProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Ollama // arbitrary — tests typically don't gate on this
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn embedding_model_name(&self) -> &str {
        &self.embedding_model
    }

    async fn is_available(&self) -> bool {
        *self.available.read().unwrap_or_else(PoisonError::into_inner)
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: self.model.clone(),
            name: self.model.clone(),
            pricing: ModelPricing {
                prompt: 0.0,
                completion: 0.0,
                request: 0.0,
            },
        }])
    }

    async fn list_embedding_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: self.embedding_model.clone(),
            name: self.embedding_model.clone(),
            pricing: ModelPricing {
                prompt: 0.0,
                completion: 0.0,
                request: 0.0,
            },
        }])
    }

    async fn complete(&self, prompt: &str, _options: CompletionOptions) -> Result<CompletionResult> {
        self.completion_calls
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(prompt.to_string());
        let popped = self
            .completions
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        Ok(popped.unwrap_or_else(|| {
            self.default_completion
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }))
    }

    async fn embed(&self, text: &str) -> Result<EmbeddingResult> {
        self.embed_calls
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(text.to_string());
        Ok(EmbeddingResult {
            embedding: Self::deterministic_embedding(text),
            tokens: 0,
            cost_usd: 0.0,
        })
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<EmbeddingResult>> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed(t).await?);
        }
        Ok(out)
    }

    async fn chat_with_tools(&self, messages: &[AiMessage], _tools: &[serde_json::Value]) -> Result<AiMessage> {
        self.chat_calls
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(messages.to_vec());
        Ok(self
            .chats
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| AiMessage {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: None,
            }))
    }

    async fn chat_stream(
        &self,
        messages: Vec<AiMessage>,
        mut on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ChatStreamResult> {
        // Reuse chat_with_tools to grab the next canned response, then emit it
        // as a single token chunk so callers using streaming see the same text
        // as callers using non-streaming.
        let resp = self.chat_with_tools(&messages, &[]).await?;
        let _ = on_token(resp.content.clone());
        Ok(ChatStreamResult {
            content: resp.content,
            eval_count: None,
            prompt_eval_count: None,
            prefill_ms: None,
            cached_prompt_tokens: None,
        })
    }
}

// Implement Clone manually so tests can hand the same backing store to multiple
// services without re-pushing canned responses. Wrap state in `Arc` for sharing.
impl Clone for FakeAiProvider {
    fn clone(&self) -> Self {
        // Tests that need shared state should instead wrap in `Arc<FakeAiProvider>`
        // and clone the Arc. A trait impl can't return Self by reference so this
        // makes a fresh, empty fake — caller error if they expected shared state.
        Self::new().with_model(self.model.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn complete_returns_canned_then_default() {
        let p = FakeAiProvider::new();
        p.push_completion("first");
        p.push_completion("second");
        let r1 = p.complete("prompt-1", CompletionOptions::default()).await.unwrap();
        let r2 = p.complete("prompt-2", CompletionOptions::default()).await.unwrap();
        let r3 = p.complete("prompt-3", CompletionOptions::default()).await.unwrap();
        assert_eq!(r1.text, "first");
        assert_eq!(r2.text, "second");
        assert_eq!(r3.text, ""); // default
        assert_eq!(p.completion_calls(), vec!["prompt-1", "prompt-2", "prompt-3"]);
    }

    #[tokio::test]
    async fn embed_is_deterministic() {
        let p = FakeAiProvider::new();
        let a = p.embed("hello").await.unwrap();
        let b = p.embed("hello").await.unwrap();
        let c = p.embed("world").await.unwrap();
        assert_eq!(a.embedding, b.embedding);
        assert_ne!(a.embedding, c.embedding);
        assert_eq!(a.embedding.len(), 8);
    }

    #[tokio::test]
    async fn chat_stream_with_tools_default_streams_prose() {
        use std::sync::{Arc, Mutex};
        let p = FakeAiProvider::new();
        p.push_chat_response("hello world");
        let tokens = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = tokens.clone();
        let msg = AiMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            tool_calls: None,
        };
        let result = p
            .chat_stream_with_tools(
                vec![msg],
                vec![],
                Box::new(move |t| {
                    sink.lock().unwrap_or_else(PoisonError::into_inner).push(t);
                    true
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.message.content, "hello world");
        assert!(result.message.tool_calls.is_none());
        assert_eq!(
            *tokens.lock().unwrap_or_else(PoisonError::into_inner),
            vec!["hello world".to_string()]
        );
    }

    #[tokio::test]
    async fn chat_stream_with_tools_default_suppresses_prose_on_tool_call() {
        use std::sync::{Arc, Mutex};
        let p = FakeAiProvider::new();
        p.push_chat_message(AiMessage {
            role: "assistant".to_string(),
            content: "internal planning that must not leak".to_string(),
            tool_calls: Some(vec![AiToolCall {
                function: AiToolCallFunction {
                    name: "search_emails".to_string(),
                    arguments: serde_json::json!({"query": "invoices"}),
                },
            }]),
        });
        let tokens = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = tokens.clone();
        let msg = AiMessage {
            role: "user".to_string(),
            content: "find invoices".to_string(),
            tool_calls: None,
        };
        let result = p
            .chat_stream_with_tools(
                vec![msg],
                vec![],
                Box::new(move |t| {
                    sink.lock().unwrap_or_else(PoisonError::into_inner).push(t);
                    true
                }),
            )
            .await
            .unwrap();
        assert!(
            tokens.lock().unwrap_or_else(PoisonError::into_inner).is_empty(),
            "tool-call turns must not stream prose"
        );
        let calls = result.message.tool_calls.expect("tool_calls preserved");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "search_emails");
    }

    #[tokio::test]
    async fn chat_records_calls() {
        let p = FakeAiProvider::new();
        p.push_chat_response("hi back");
        let msg = AiMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            tool_calls: None,
        };
        let resp = p.chat_with_tools(std::slice::from_ref(&msg), &[]).await.unwrap();
        assert_eq!(resp.content, "hi back");
        let calls = p.chat_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0].content, "hi");
    }
}
