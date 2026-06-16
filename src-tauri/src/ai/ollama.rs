// Ollama client for local AI inference

use std::time::Duration;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Deserializer, Serialize};

use crate::ai::provider::{
    AIProvider, AiMessage, AiToolCall, AiToolCallFunction, BackendCapabilities, ChatStreamResult as AiChatStreamResult,
    CompletionOptions, CompletionResult, EmbeddingResult, ModelInfo, ModelPricing, ProviderType, ToolStreamResult,
};
use crate::models::error::{AppError, Result};

const OLLAMA_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "gemma4:e2b";
const EMBEDDING_MODEL: &str = "nomic-embed-text";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const EMBEDDING_TIMEOUT: Duration = Duration::from_secs(30);
const GENERATION_TIMEOUT: Duration = Duration::from_secs(60);

/// Default `keep_alive` window sent to Ollama. Ollama defaults to 5 minutes,
/// which on a constrained M1 with 16 GB RAM means the model is frequently
/// evicted between chat turns — a cold reload of a 2-4 GB model adds 3-6 s
/// of latency. 30 m covers a typical session without pinning RAM forever.
/// Overridable via the `chat.keep_alive_seconds` preference.
const DEFAULT_KEEP_ALIVE: &str = "30m";

#[derive(Debug, Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaSamplingOptions>,
    /// `keep_alive` controls how long Ollama keeps the model resident in RAM
    /// after the request. Accepted formats: "30m", "1h", "-1" (forever), "0"
    /// (unload immediately). We default to DEFAULT_KEEP_ALIVE so subsequent
    /// chat turns hit a warm model instead of cold-loading on every request.
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_alive: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    #[serde(default)]
    response: String,
    /// Thinking models (gemma4, deepseek-r1, qwq) emit their scratchpad here.
    /// The `response` field may be empty when the model only produces a think block.
    #[serde(default)]
    thinking: String,
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaSamplingOptions>,
    /// See `OllamaRequest::keep_alive`.
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_alive: Option<String>,
}

/// Sampling hyperparameters forwarded to Ollama's /api/chat under the
/// `options` key. Values here are deliberately low-temperature because the
/// chat surface is RAG-over-email — the user wants faithful retrieval, not
/// creative continuations. Tuning rationale:
///   - temperature 0.2: keeps the model grounded in retrieved text and
///     suppresses plausible-sounding-but-invented details (invoice amounts,
///     dates). Raising it briefly re-introduces phantom citations.
///   - top_p 0.9: trims the long tail without being so strict that
///     multi-lingual (EN/ES) phrasing gets flattened.
///   - top_k 40: Ollama's default upper bound; included so we do not
///     accidentally inherit a looser per-model default.
#[derive(Debug, Clone, Serialize)]
pub struct OllamaSamplingOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    /// Context window size (tokens). Ollama's default is 4096 — far below
    /// what we need for 8 RAG sources + system prompt + history. Unset here
    /// means "use Ollama's default", which for our chat pipeline will
    /// silently truncate half the input. See `grounded()` for the value
    /// we actually ship.
    #[serde(skip_serializing_if = "Option::is_none", rename = "num_ctx")]
    pub num_ctx: Option<i32>,
    /// Max tokens to generate. `-1` = model decides (until EOS or ctx).
    /// Set defensively so the final answer isn't cut off mid-markdown-table.
    #[serde(skip_serializing_if = "Option::is_none", rename = "num_predict")]
    pub num_predict: Option<i32>,
}

impl OllamaSamplingOptions {
    /// Default sampling for EmailOps chat: low-temperature, grounded, with
    /// an explicit context window big enough for our RAG pipeline.
    ///
    /// `num_ctx = 8192` is a deliberate trade-off for local M1-class hardware:
    /// prompt-eval time scales roughly linearly with ctx on CPU/Metal, so
    /// 16k → 8k ≈ halves time-to-first-token for typical RAG prompts. 8k
    /// still covers system prompt (~2k) + a trimmed source set + short
    /// history. Callers that need a bigger window can construct the options
    /// manually; the chat pipeline pairs this with the smaller `TOP_K_SOURCES`
    /// and `MAX_SOURCE_BODY_CHARS` so the payload fits comfortably.
    ///
    /// `num_predict = -1` lets the model generate until EOS rather than
    /// being cut off mid-table at some tight default.
    pub fn grounded() -> Self {
        Self {
            temperature: Some(0.2),
            top_p: Some(0.9),
            top_k: Some(40),
            num_ctx: Some(8192),
            num_predict: Some(-1),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OllamaToolCall>>,
    /// Thinking scratchpad — only present for thinking models.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaToolCall {
    pub function: OllamaToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaToolCallFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaChatMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaChatStreamChunk {
    #[serde(default)]
    message: Option<OllamaChatMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    eval_count: Option<u32>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
}

/// Result returned by [`OllamaClient::chat_stream`] — the accumulated text
/// plus any stats the final chunk provided.
#[derive(Debug, Clone)]
pub struct ChatStreamResult {
    pub content: String,
    pub eval_count: Option<u32>,
    pub prompt_eval_count: Option<u32>,
    /// Ollama's HTTP API does not expose prefill latency — always `None`.
    pub prefill_ms: Option<i64>,
    /// Ollama does not report KV-cache reuse — always `None`.
    pub cached_prompt_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    prompt: String,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    embedding: Vec<f32>,
}

/// Batched embedding request for Ollama's newer `/api/embed` endpoint,
/// which accepts `input: []string` and returns one vector per input in a
/// single forward pass. Much faster than the one-at-a-time `/api/embeddings`
/// loop for large reindex jobs.
#[derive(Debug, Serialize)]
struct BatchEmbeddingRequest<'a> {
    model: String,
    input: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_alive: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BatchEmbeddingResponse {
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ParsedSearchQuery {
    pub keywords: Vec<String>,
    pub from_filter: Option<String>,
    pub to_filter: Option<String>,
    pub subject_filter: Option<String>,
    pub has_attachment: Option<bool>,
    pub is_unread: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_after_timestamp")]
    pub after_timestamp: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_before_timestamp")]
    pub before_timestamp: Option<i64>,
    #[serde(default)]
    pub tag_filters: Vec<String>,
}

pub struct OllamaClient {
    client: Client,
    base_url: String,
    model: String,
    embedding_model: String,
    /// Applied to every generate/chat request. Defaults to DEFAULT_KEEP_ALIVE
    /// but the caller (AiService) may override it from the
    /// `chat.keep_alive_seconds` user preference.
    keep_alive: String,
}

impl OllamaClient {
    pub fn new(model: Option<&str>) -> Self {
        Self::new_with_models(model, None)
    }

    pub fn new_with_models(model: Option<&str>, embedding_model: Option<&str>) -> Self {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            base_url: OLLAMA_BASE_URL.to_string(),
            model: model.unwrap_or(DEFAULT_MODEL).to_string(),
            embedding_model: embedding_model.unwrap_or(EMBEDDING_MODEL).to_string(),
            keep_alive: DEFAULT_KEEP_ALIVE.to_string(),
        }
    }

    /// Override the `keep_alive` window applied to every Ollama request.
    /// Accepts any duration string Ollama recognises ("30m", "1h", "0" to
    /// evict immediately, "-1" to pin forever).
    pub fn with_keep_alive(mut self, keep_alive: impl Into<String>) -> Self {
        self.keep_alive = keep_alive.into();
        self
    }

    pub fn embedding_model_name(&self) -> &str {
        &self.embedding_model
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }

    pub async fn list_model_names(&self) -> Result<Vec<String>> {
        let models = self.list_models().await?;
        Ok(models.into_iter().map(|m| m.id).collect())
    }

    pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/api/embeddings", self.base_url);

        let request = EmbeddingRequest {
            model: self.embedding_model.clone(),
            prompt: text.to_string(),
        };

        let response = self
            .client
            .post(&url)
            .timeout(EMBEDDING_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::AiError("Embedding generation timed out (30s). Is the model loaded?".to_string())
                } else {
                    AppError::AiError(format!("Failed to connect to Ollama for embeddings: {}", e))
                }
            })?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!("Ollama embedding error: {}", error_text)));
        }

        let result: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse embedding response: {}", e)))?;

        Ok(result.embedding)
    }

    /// Generate embeddings for many texts in a single HTTP round-trip via
    /// Ollama's `/api/embed` endpoint. Falls back to a one-at-a-time loop
    /// against the legacy `/api/embeddings` endpoint if the batch endpoint
    /// returns 404 (older Ollama builds) or a mismatched count.
    pub async fn generate_embeddings_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/api/embed", self.base_url);
        let request = BatchEmbeddingRequest {
            model: self.embedding_model.clone(),
            input: texts,
            keep_alive: Some(self.keep_alive.clone()),
        };

        // Embedding passes scale with total input length; give the batch call
        // proportionally more headroom than the single-text timeout.
        let timeout = Duration::from_secs(30 + 2 * texts.len() as u64);

        let response = match self.client.post(&url).timeout(timeout).json(&request).send().await {
            Ok(r) => r,
            Err(e) => {
                if e.is_timeout() {
                    return Err(AppError::AiError(format!(
                        "Batch embedding timed out after {}s",
                        timeout.as_secs()
                    )));
                }
                return Err(AppError::AiError(format!(
                    "Failed to connect to Ollama for batch embeddings: {}",
                    e
                )));
            }
        };

        // Older Ollama builds (<0.1.32) don't expose /api/embed — fall back
        // to the per-item endpoint transparently so users on stale versions
        // keep working.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return self.generate_embeddings_batch_fallback(texts).await;
        }

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!(
                "Ollama batch embedding error ({}): {}",
                status, error_text
            )));
        }

        let result: BatchEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse batch embedding response: {}", e)))?;

        if result.embeddings.len() != texts.len() {
            // Shape mismatch — refuse to guess which text maps to which vector
            // and fall back. Fallback is slow but correct.
            crate::services::logger::log(
                "debug",
                "ai",
                format!(
                    "ollama /api/embed returned {} vectors for {} inputs; falling back to per-item",
                    result.embeddings.len(),
                    texts.len()
                ),
            );
            return self.generate_embeddings_batch_fallback(texts).await;
        }

        Ok(result.embeddings)
    }

    /// Legacy path: one HTTP call per text against `/api/embeddings`.
    /// Preserved for compatibility with Ollama builds that lack `/api/embed`.
    async fn generate_embeddings_batch_fallback(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            let embedding = self.generate_embedding(text).await?;
            embeddings.push(embedding);
        }
        Ok(embeddings)
    }

    pub async fn generate(&self, prompt: &str) -> Result<String> {
        self.generate_with_options(prompt, None).await
    }

    async fn generate_with_options(&self, prompt: &str, sampling: Option<OllamaSamplingOptions>) -> Result<String> {
        let url = format!("{}/api/generate", self.base_url);

        let request = OllamaRequest {
            model: self.model.clone(),
            prompt: prompt.to_string(),
            stream: false,
            options: sampling,
            keep_alive: Some(self.keep_alive.clone()),
        };

        let response = self
            .client
            .post(&url)
            .timeout(GENERATION_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::AiError(format!(
                        "AI generation timed out ({}s). Try a smaller model.",
                        GENERATION_TIMEOUT.as_secs()
                    ))
                } else {
                    AppError::AiError(format!("Failed to connect to Ollama: {}", e))
                }
            })?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!("Ollama error: {}", error_text)));
        }

        let result: OllamaResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse Ollama response: {}", e)))?;

        // Thinking models (gemma4, deepseek-r1, qwq) put their scratchpad in
        // `thinking` and leave `response` empty. Fall back to thinking content
        // so callers can still extract JSON from the model's reasoning.
        if result.response.is_empty() && !result.thinking.is_empty() {
            Ok(result.thinking)
        } else {
            Ok(result.response)
        }
    }

    pub async fn chat(&self, prompt: &str, think: Option<bool>) -> Result<String> {
        self.chat_with_sampling(prompt, think, OllamaSamplingOptions::grounded())
            .await
    }

    async fn chat_with_sampling(
        &self,
        prompt: &str,
        think: Option<bool>,
        sampling: OllamaSamplingOptions,
    ) -> Result<String> {
        let url = format!("{}/api/chat", self.base_url);

        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: vec![OllamaChatMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
                tool_calls: None,
                thinking: String::new(),
            }],
            stream: false,
            think,
            tools: None,
            options: Some(sampling),
            keep_alive: Some(self.keep_alive.clone()),
        };

        let response = self
            .client
            .post(&url)
            .timeout(GENERATION_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::AiError(format!(
                        "AI generation timed out ({}s). Try a smaller model.",
                        GENERATION_TIMEOUT.as_secs()
                    ))
                } else {
                    AppError::AiError(format!("Failed to connect to Ollama: {}", e))
                }
            })?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!("Ollama error: {}", error_text)));
        }

        let result: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse Ollama chat response: {}", e)))?;

        let content = result.message.content;
        if content.is_empty() && !result.message.thinking.is_empty() {
            Ok(result.message.thinking)
        } else {
            Ok(content)
        }
    }

    /// Non-streaming chat with tool definitions (Ollama wire types).
    /// Called internally by the `AIProvider::chat_with_tools` impl.
    async fn chat_with_tools_internal(
        &self,
        messages: &[OllamaChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<OllamaChatMessage> {
        let url = format!("{}/api/chat", self.base_url);

        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: messages.to_vec(),
            stream: false,
            think: None,
            tools: if tools.is_empty() { None } else { Some(tools.to_vec()) },
            options: Some(OllamaSamplingOptions::grounded()),
            keep_alive: Some(self.keep_alive.clone()),
        };

        let response = self
            .client
            .post(&url)
            .timeout(GENERATION_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::AiError(format!(
                        "Tool-call round timed out ({}s). Try a smaller model.",
                        GENERATION_TIMEOUT.as_secs()
                    ))
                } else {
                    AppError::AiError(format!("Failed to connect to Ollama: {}", e))
                }
            })?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!("Ollama tool-call error: {}", error_text)));
        }

        let result: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse Ollama tool-call response: {}", e)))?;

        Ok(result.message)
    }

    /// Streaming chat (Ollama wire types). Called internally by the
    /// `AIProvider::chat_stream` impl.
    async fn chat_stream_internal(
        &self,
        messages: Vec<(String, String)>,
        mut on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ChatStreamResult> {
        let url = format!("{}/api/chat", self.base_url);
        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: messages
                .into_iter()
                .map(|(role, content)| OllamaChatMessage {
                    role,
                    content,
                    tool_calls: None,
                    thinking: String::new(),
                })
                .collect(),
            stream: true,
            think: None,
            tools: None,
            options: Some(OllamaSamplingOptions::grounded()),
            keep_alive: Some(self.keep_alive.clone()),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to connect to Ollama for chat stream: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!(
                "Ollama chat stream error ({}): {}",
                status, error_text
            )));
        }

        let mut accumulated = String::new();
        let mut stream = response.bytes_stream();
        // Chunks can split mid-line; buffer partial lines until we see '\n'.
        let mut buf: Vec<u8> = Vec::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| AppError::AiError(format!("Ollama chat stream read error: {}", e)))?;
            buf.extend_from_slice(&bytes);

            while let Some(idx) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=idx).collect();
                let line_str = std::str::from_utf8(&line[..line.len() - 1]).unwrap_or("").trim();
                if line_str.is_empty() {
                    continue;
                }
                let parsed: OllamaChatStreamChunk = match serde_json::from_str(line_str) {
                    Ok(c) => c,
                    Err(e) => {
                        // A malformed line is unusual but not necessarily fatal — log
                        // and keep going so a single bad chunk doesn't kill the stream.
                        crate::services::logger::log(
                            "debug",
                            "ai",
                            format!("ollama.chat_stream skipping malformed chunk: {} (err: {})", line_str, e),
                        );
                        continue;
                    }
                };
                if let Some(msg) = &parsed.message {
                    if !msg.content.is_empty() {
                        accumulated.push_str(&msg.content);
                        if !on_token(msg.content.clone()) {
                            return Ok(ChatStreamResult {
                                content: accumulated,
                                eval_count: None,
                                prompt_eval_count: None,
                                prefill_ms: None,
                                cached_prompt_tokens: None,
                            });
                        }
                    }
                }
                if parsed.done {
                    return Ok(ChatStreamResult {
                        content: accumulated,
                        eval_count: parsed.eval_count,
                        prompt_eval_count: parsed.prompt_eval_count,
                        prefill_ms: None,
                        cached_prompt_tokens: None,
                    });
                }
            }
        }

        // Stream ended without a done marker — return what we have.
        Ok(ChatStreamResult {
            content: accumulated,
            eval_count: None,
            prompt_eval_count: None,
            prefill_ms: None,
            cached_prompt_tokens: None,
        })
    }

    /// Streaming chat WITH tool definitions (Ollama wire types). Streams
    /// assistant prose via `on_token` while accumulating any `tool_calls` that
    /// arrive in the stream. Ollama emits `tool_calls` structurally separate
    /// from `content`, so no prose/tool-call gating is needed — a tool-call turn
    /// simply carries empty content. Returns the accumulated assistant message
    /// plus token stats from the final chunk.
    async fn chat_stream_with_tools_internal(
        &self,
        messages: &[OllamaChatMessage],
        tools: &[serde_json::Value],
        mut on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<(OllamaChatMessage, Option<u32>, Option<u32>)> {
        let url = format!("{}/api/chat", self.base_url);
        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: messages.to_vec(),
            stream: true,
            think: None,
            tools: if tools.is_empty() { None } else { Some(tools.to_vec()) },
            options: Some(OllamaSamplingOptions::grounded()),
            keep_alive: Some(self.keep_alive.clone()),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to connect to Ollama for tool stream: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!(
                "Ollama tool stream error ({}): {}",
                status, error_text
            )));
        }

        let mut accumulated = String::new();
        let mut tool_calls: Vec<OllamaToolCall> = Vec::new();
        let mut eval_count: Option<u32> = None;
        let mut prompt_eval_count: Option<u32> = None;
        let mut stream = response.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();

        let finalize = |content: String, calls: Vec<OllamaToolCall>| OllamaChatMessage {
            role: "assistant".to_string(),
            content,
            tool_calls: if calls.is_empty() { None } else { Some(calls) },
            thinking: String::new(),
        };

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| AppError::AiError(format!("Ollama tool stream read error: {}", e)))?;
            buf.extend_from_slice(&bytes);

            while let Some(idx) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=idx).collect();
                let line_str = std::str::from_utf8(&line[..line.len() - 1]).unwrap_or("").trim();
                if line_str.is_empty() {
                    continue;
                }
                let parsed: OllamaChatStreamChunk = match serde_json::from_str(line_str) {
                    Ok(c) => c,
                    Err(e) => {
                        crate::services::logger::log(
                            "debug",
                            "ai",
                            format!(
                                "ollama.chat_stream_with_tools skipping malformed chunk: {} (err: {})",
                                line_str, e
                            ),
                        );
                        continue;
                    }
                };
                if let Some(msg) = parsed.message {
                    if let Some(tc) = msg.tool_calls {
                        tool_calls.extend(tc);
                    }
                    if !msg.content.is_empty() {
                        accumulated.push_str(&msg.content);
                        if !on_token(msg.content.clone()) {
                            // Cancelled by caller — return what we have.
                            return Ok((finalize(accumulated, tool_calls), eval_count, prompt_eval_count));
                        }
                    }
                }
                if parsed.eval_count.is_some() {
                    eval_count = parsed.eval_count;
                }
                if parsed.prompt_eval_count.is_some() {
                    prompt_eval_count = parsed.prompt_eval_count;
                }
                if parsed.done {
                    return Ok((finalize(accumulated, tool_calls), eval_count, prompt_eval_count));
                }
            }
        }

        // Stream ended without a done marker — return what we have.
        Ok((finalize(accumulated, tool_calls), eval_count, prompt_eval_count))
    }

    pub async fn parse_search_query(&self, query: &str) -> Result<ParsedSearchQuery> {
        if let Some(parsed) = self.parse_simple_query(query) {
            return Ok(parsed);
        }

        let today = Utc::now().date_naive().format("%Y-%m-%d").to_string();

        let prompt = format!(
            r#"You are a search query parser for an email application. Parse the following natural language search query and extract structured search parameters.

Today is {}.

Query: "{}"

Respond with ONLY a JSON object (no markdown, no explanation) with these fields:
- keywords: array of search keywords (words to search in email body/subject)
- from_filter: email address or name to filter by sender (null if not specified)
- to_filter: email address or name to filter by recipient (null if not specified)  
- subject_filter: specific subject text to search for (null if not specified)
- is_unread: boolean if user wants unread emails only (null if not specified)
- after_timestamp: "YYYY-MM-DD" if the query implies a start date, otherwise null
- before_timestamp: "YYYY-MM-DD" if the query implies an end date, otherwise null

Examples:
Query: "emails from John about the project proposal"
{{"keywords": ["project", "proposal"], "from_filter": "John", "to_filter": null, "subject_filter": null, "is_unread": null, "after_timestamp": null, "before_timestamp": null}}

Query: "unread messages about invoices"
{{"keywords": ["invoices"], "from_filter": null, "to_filter": null, "subject_filter": null, "is_unread": true, "after_timestamp": null, "before_timestamp": null}}

Query: "correos de chema sobre facturas"
{{"keywords": ["facturas"], "from_filter": "chema", "to_filter": null, "subject_filter": null, "is_unread": null, "after_timestamp": null, "before_timestamp": null}}

Query: "mensajes sin leer de google de esta semana"
{{"keywords": [], "from_filter": "google", "to_filter": null, "subject_filter": null, "is_unread": true, "after_timestamp": "2026-04-01", "before_timestamp": null}}

Query: "correos sobre presupuesto después de 2025-01-01"
{{"keywords": ["presupuesto"], "from_filter": null, "to_filter": null, "subject_filter": null, "is_unread": null, "after_timestamp": "2025-01-01", "before_timestamp": null}}

Now parse the query and respond with JSON only:"#,
            today, query
        );

        let response = self.generate(&prompt).await?;
        let json_str = extract_json(&response);

        serde_json::from_str(&json_str).map_err(|e| AppError::AiError(format!("Failed to parse search query: {}", e)))
    }

    fn parse_simple_query(&self, query: &str) -> Option<ParsedSearchQuery> {
        parse_search_query_patterns(query)
    }

    pub async fn triage_email(&self, subject: &str, snippet: &str) -> Result<String> {
        let prompt = format!(
            r#"You are an email triage assistant. Classify the following email into one of these categories:
- action_needed: Requires a response or action from the user
- fyi: Informational, no action needed but good to know
- low_priority: Can be ignored or dealt with later

Email Subject: {}
Email Preview: {}

Respond with ONLY one of: action_needed, fyi, low_priority"#,
            subject, snippet
        );

        let response = self.generate(&prompt).await?;
        let trimmed = response.trim().to_lowercase();

        match trimmed.as_str() {
            "action_needed" | "fyi" | "low_priority" => Ok(trimmed),
            _ => {
                if trimmed.contains("action_needed") {
                    Ok("action_needed".to_string())
                } else if trimmed.contains("low_priority") {
                    Ok("low_priority".to_string())
                } else {
                    Ok("fyi".to_string())
                }
            }
        }
    }

    pub async fn summarize_thread(&self, emails: &[String]) -> Result<String> {
        let thread_content = emails.join("\n---\n");

        let prompt = format!(
            r#"Summarize the following email thread in 2-3 sentences. Focus on the key points and any action items.

Email Thread:
{}

Summary:"#,
            thread_content
        );

        self.generate(&prompt).await
    }

    pub async fn generate_draft(&self, context: &str, instructions: Option<&str>) -> Result<String> {
        let instruction_text = instructions.unwrap_or("Write a professional reply");

        let prompt = format!(
            r#"You are an email assistant. Write a reply to the following email.

Original Email:
{}

Instructions: {}

Write a professional email reply:"#,
            context, instruction_text
        );

        self.generate(&prompt).await
    }
}

#[async_trait]
impl AIProvider for OllamaClient {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Ollama
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    async fn is_available(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        self.client.get(&url).timeout(CONNECT_TIMEOUT).send().await.is_ok()
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/api/tags", self.base_url);
        let response = self
            .client
            .get(&url)
            .timeout(CONNECT_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to connect to Ollama: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::AiError("Failed to list models".to_string()));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse models response: {}", e)))?;

        let models = body["models"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let name = m["name"].as_str()?.to_string();
                        Some(ModelInfo {
                            id: name.clone(),
                            name: name.clone(),
                            pricing: ModelPricing {
                                prompt: 0.0,
                                completion: 0.0,
                                request: 0.0,
                            },
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(models)
    }

    async fn complete(&self, prompt: &str, options: CompletionOptions) -> Result<CompletionResult> {
        let text = if options.think.is_some() {
            // Some(true) = enable thinking, Some(false) = disable thinking.
            // Both route through /api/chat which is the only endpoint that
            // supports the `think` parameter for thinking models.
            let mut sampling = OllamaSamplingOptions::grounded();
            if let Some(temp) = options.temperature {
                sampling.temperature = Some(temp as f32);
            }
            if let Some(max_tokens) = options.max_tokens {
                sampling.num_predict = Some(max_tokens as i32);
            }
            self.chat_with_sampling(prompt, options.think, sampling).await?
        } else {
            let sampling = if options.temperature.is_some() || options.max_tokens.is_some() {
                let mut s = OllamaSamplingOptions::grounded();
                if let Some(temp) = options.temperature {
                    s.temperature = Some(temp as f32);
                }
                if let Some(max_tokens) = options.max_tokens {
                    s.num_predict = Some(max_tokens as i32);
                }
                Some(s)
            } else {
                None
            };
            self.generate_with_options(prompt, sampling).await?
        };
        Ok(CompletionResult {
            text,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: 0.0,
            model: self.model.clone(),
        })
    }

    fn embedding_model_name(&self) -> &str {
        &self.embedding_model
    }

    async fn list_embedding_models(&self) -> Result<Vec<ModelInfo>> {
        // Return all models; callers filter by name for embed-specific ones.
        self.list_models().await
    }

    async fn embed(&self, text: &str) -> Result<EmbeddingResult> {
        let embedding = self.generate_embedding(text).await?;
        Ok(EmbeddingResult {
            embedding,
            tokens: 0,
            cost_usd: 0.0,
        })
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<EmbeddingResult>> {
        // Batched path: single POST to /api/embed returns all vectors at once.
        // On older Ollama builds this transparently falls back to per-item.
        let vectors = self.generate_embeddings_batch(texts).await?;
        Ok(vectors
            .into_iter()
            .map(|embedding| EmbeddingResult {
                embedding,
                tokens: 0,
                cost_usd: 0.0,
            })
            .collect())
    }

    async fn chat_with_tools(&self, messages: &[AiMessage], tools: &[serde_json::Value]) -> Result<AiMessage> {
        let ollama_msgs = ai_messages_to_ollama(messages);
        let result = self.chat_with_tools_internal(&ollama_msgs, tools).await?;
        Ok(ollama_message_to_ai(result))
    }

    async fn chat_stream(
        &self,
        messages: Vec<AiMessage>,
        on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<AiChatStreamResult> {
        let pairs: Vec<(String, String)> = messages.into_iter().map(|m| (m.role, m.content)).collect();
        let result = self.chat_stream_internal(pairs, on_token).await?;
        Ok(AiChatStreamResult {
            content: result.content,
            eval_count: result.eval_count,
            prompt_eval_count: result.prompt_eval_count,
            prefill_ms: None,
            cached_prompt_tokens: None,
            prefix_plan: None,
            sys_cached_before: None,
            sys_cached_after: None,
            system_prefix_tokens: None,
            stable_tokens: None,
            dropped_front_tokens: None,
        })
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<AiMessage>,
        tools: Vec<serde_json::Value>,
        on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ToolStreamResult> {
        let ollama_msgs = ai_messages_to_ollama(&messages);
        let (result, eval_count, prompt_eval_count) = self
            .chat_stream_with_tools_internal(&ollama_msgs, &tools, on_token)
            .await?;
        // `ollama_message_to_ai` clears content when tool_calls are present, so a
        // tool-call turn never carries leaked prose in history.
        let message = ollama_message_to_ai(result);
        Ok(ToolStreamResult {
            message,
            eval_count,
            prompt_eval_count,
            prefill_ms: None,
            cached_prompt_tokens: None,
            prefix_plan: None,
            sys_cached_before: None,
            sys_cached_after: None,
            system_prefix_tokens: None,
            stable_tokens: None,
            dropped_front_tokens: None,
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            tools: true,
            streaming: true,
            embeddings: true,
        }
    }

    /// Fire a 1-token completion to force Ollama to load the configured model
    /// into RAM. Combined with the `keep_alive` field this keeps subsequent
    /// turns warm for the lifetime of the keep_alive window.
    async fn warmup(&self) -> Result<()> {
        let url = format!("{}/api/generate", self.base_url);
        let request = OllamaRequest {
            model: self.model.clone(),
            prompt: "hi".to_string(),
            stream: false,
            options: Some(OllamaSamplingOptions {
                temperature: Some(0.0),
                top_p: None,
                top_k: None,
                num_ctx: Some(256),
                num_predict: Some(1),
            }),
            keep_alive: Some(self.keep_alive.clone()),
        };
        // Generous timeout: the first load of a multi-GB GGUF on a cold
        // system can take tens of seconds. We don't care about the output.
        let _ = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(120))
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::AiError(format!("Ollama warmup failed: {}", e)))?;
        Ok(())
    }
}

/// Convert provider-neutral `AiMessage`s into Ollama wire messages, preserving
/// any `tool_calls` so multi-round tool histories round-trip correctly.
fn ai_messages_to_ollama(messages: &[AiMessage]) -> Vec<OllamaChatMessage> {
    messages
        .iter()
        .map(|m| OllamaChatMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            tool_calls: m.tool_calls.as_ref().map(|tc_list| {
                tc_list
                    .iter()
                    .map(|tc| OllamaToolCall {
                        function: OllamaToolCallFunction {
                            name: tc.function.name.clone(),
                            arguments: tc.function.arguments.clone(),
                        },
                    })
                    .collect()
            }),
            thinking: String::new(),
        })
        .collect()
}

/// Convert an Ollama assistant message back to provider-neutral form. When tool
/// calls are present the content is dropped — consistent with the other
/// backends, a tool-call turn dispatches calls rather than surfacing prose.
fn ollama_message_to_ai(msg: OllamaChatMessage) -> AiMessage {
    let tool_calls: Option<Vec<AiToolCall>> = msg.tool_calls.map(|tc_list| {
        tc_list
            .into_iter()
            .map(|tc| AiToolCall {
                function: AiToolCallFunction {
                    name: tc.function.name,
                    arguments: tc.function.arguments,
                },
            })
            .collect()
    });
    let has_tool_calls = tool_calls.as_ref().is_some_and(|v| !v.is_empty());
    AiMessage {
        role: msg.role,
        content: if has_tool_calls { String::new() } else { msg.content },
        tool_calls,
    }
}

fn extract_json(text: &str) -> String {
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return text[start..=end].to_string();
        }
    }
    text.to_string()
}

pub fn parse_search_query_patterns(query: &str) -> Option<ParsedSearchQuery> {
    let query = query.trim();
    let query_lower = query.to_lowercase();
    let mut parsed = ParsedSearchQuery::default();
    let mut has_filter = false;

    if let Some(value) = extract_prefixed_value(query, &query_lower, &["from:", "de:"]) {
        parsed.from_filter = Some(value);
        has_filter = true;
    }

    if let Some(value) = extract_prefixed_value(query, &query_lower, &["to:", "para:"]) {
        parsed.to_filter = Some(value);
        has_filter = true;
    }

    if let Some(value) = extract_prefixed_value(query, &query_lower, &["subject:", "asunto:"]) {
        parsed.subject_filter = Some(value);
        has_filter = true;
    }

    if parsed.from_filter.is_none() {
        if let Some(value) = extract_phrase_filter(
            query,
            &query_lower,
            &[
                "emails from ",
                "messages from ",
                "mails from ",
                "correos de ",
                "mails de ",
                "emails de",
                "mensajes de ",
                "sin leer de ",
                "unread from ",
            ],
        ) {
            parsed.from_filter = Some(value);
            has_filter = true;
        }
    }

    if query_lower.contains("is:unread")
        || query_lower.contains("unread")
        || query_lower.contains("sin leer")
        || query_lower.contains("no leidos")
        || query_lower.contains("no leídos")
    {
        parsed.is_unread = Some(true);
        has_filter = true;
    }

    if let Some(value) = extract_prefixed_value(query, &query_lower, &["after:", "despues:", "después:"]) {
        if let Some(ts) = parse_date_to_timestamp(&value) {
            parsed.after_timestamp = Some(ts);
            has_filter = true;
        }
    }

    if let Some(value) = extract_prefixed_value(query, &query_lower, &["before:", "antes:"]) {
        if let Some(ts) = parse_date_to_timestamp(&value) {
            parsed.before_timestamp = Some(end_of_day_timestamp(ts));
            has_filter = true;
        }
    }

    if query_lower.contains("today") || query_lower.contains("hoy") {
        parsed.after_timestamp = parsed.after_timestamp.or_else(start_of_today_timestamp);
        has_filter = true;
    }

    if query_lower.contains("this week") || query_lower.contains("esta semana") {
        parsed.after_timestamp = parsed.after_timestamp.or_else(start_of_week_timestamp);
        has_filter = true;
    }

    if query_lower.contains("this month") || query_lower.contains("este mes") {
        parsed.after_timestamp = parsed.after_timestamp.or_else(start_of_month_timestamp);
        has_filter = true;
    }

    // Extract tag: filters (e.g., tag:billing, tag:urgent)
    {
        let mut rest = query_lower.as_str();
        while let Some(idx) = rest.find("tag:") {
            let after = &rest[idx + 4..];
            let value = after.split_whitespace().next().unwrap_or(after).trim();
            if !value.is_empty() {
                parsed.tag_filters.push(value.to_string());
                has_filter = true;
            }
            rest = &rest[idx + 4..];
        }
    }

    if has_filter {
        parsed.keywords = extract_residual_keywords(query);
        let from_filter = parsed.from_filter.clone();
        let to_filter = parsed.to_filter.clone();
        let subject_filter = parsed.subject_filter.clone();
        prune_filter_terms(
            &mut parsed.keywords,
            from_filter.as_deref(),
            to_filter.as_deref(),
            subject_filter.as_deref(),
        );
        return Some(parsed);
    }

    None
}

fn extract_prefixed_value(query: &str, query_lower: &str, prefixes: &[&str]) -> Option<String> {
    for prefix in prefixes {
        if let Some(idx) = query_lower.find(prefix) {
            let rest = &query[idx + prefix.len()..].trim_start();
            let value = rest.split_whitespace().next().unwrap_or(rest).trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn extract_phrase_filter(query: &str, query_lower: &str, patterns: &[&str]) -> Option<String> {
    const SEPARATORS: [&str; 11] = [
        " about ",
        " sobre ",
        " after ",
        " despues ",
        " después ",
        " before ",
        " antes ",
        " this week",
        " this month",
        " esta semana",
        " este mes",
    ];

    for pattern in patterns {
        if let Some(idx) = query_lower.find(pattern) {
            let start = idx + pattern.len();
            let rest_lower = &query_lower[start..];
            let end = SEPARATORS
                .iter()
                .filter_map(|separator| rest_lower.find(separator))
                .min()
                .unwrap_or(rest_lower.len());
            let value = query[start..start + end].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn extract_residual_keywords(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .filter_map(|word| {
            let normalized = normalize_token(word);
            if normalized.is_empty()
                || normalized.contains(':')
                || matches!(
                    normalized.as_str(),
                    "from"
                        | "de"
                        | "to"
                        | "para"
                        | "subject"
                        | "asunto"
                        | "is"
                        | "unread"
                        | "sin"
                        | "leer"
                        | "no"
                        | "leidos"
                        | "despues"
                        | "despues:"
                        | "after"
                        | "before"
                        | "antes"
                        | "today"
                        | "hoy"
                        | "this"
                        | "week"
                        | "month"
                        | "emails"
                        | "messages"
                        | "mails"
                        | "correos"
                        | "mensajes"
                        | "esta"
                        | "este"
                        | "semana"
                        | "mes"
                        | "sobre"
                        | "about"
                )
                || normalized.ends_with(':')
            {
                None
            } else {
                Some(normalized)
            }
        })
        .collect()
}

fn prune_filter_terms(
    keywords: &mut Vec<String>,
    from_filter: Option<&str>,
    to_filter: Option<&str>,
    subject_filter: Option<&str>,
) {
    let mut blocked = Vec::new();

    if let Some(from) = from_filter {
        blocked.extend(from.split_whitespace().map(normalize_token));
    }
    if let Some(to) = to_filter {
        blocked.extend(to.split_whitespace().map(normalize_token));
    }
    if let Some(subject) = subject_filter {
        blocked.extend(subject.split_whitespace().map(normalize_token));
    }

    keywords.retain(|keyword| !blocked.iter().any(|blocked_term| blocked_term == keyword));
}

fn normalize_token(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != ':' && c != '-' && c != '_')
        .to_lowercase()
}

/// Parse a user-typed date for `before:`/`after:` search operators.
///
/// Accepts the three formats users actually type: `YYYY-MM-DD`,
/// `DD-MM-YYYY`, and `DD/MM/YYYY` (and their `/`-separated / `-`-separated
/// equivalents). The format is inferred from the size of the first part —
/// four digits is a year, one-or-two digits is a day-of-month.
fn parse_date_to_timestamp(date_str: &str) -> Option<i64> {
    let parts: Vec<&str> = date_str.split(['-', '/']).collect();
    if parts.len() != 3 {
        return None;
    }

    let (year, month, day) = if parts[0].len() == 4 {
        // YYYY-MM-DD
        let year: i32 = parts[0].parse().ok()?;
        let month: u32 = parts[1].parse().ok()?;
        let day: u32 = parts[2].parse().ok()?;
        (year, month, day)
    } else if parts[2].len() == 4 {
        // DD-MM-YYYY
        let day: u32 = parts[0].parse().ok()?;
        let month: u32 = parts[1].parse().ok()?;
        let year: i32 = parts[2].parse().ok()?;
        (year, month, day)
    } else {
        return None;
    };

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let datetime = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0)?);
    Some(datetime.and_utc().timestamp())
}

fn start_of_today_timestamp() -> Option<i64> {
    Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc().timestamp())
}

fn start_of_week_timestamp() -> Option<i64> {
    let now = Utc::now();
    let days_since_monday = now.weekday().num_days_from_monday();
    let monday = now.date_naive() - chrono::Duration::days(days_since_monday as i64);
    monday.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp())
}

fn start_of_month_timestamp() -> Option<i64> {
    let now = Utc::now();
    NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp())
}

fn end_of_day_timestamp(timestamp: i64) -> i64 {
    timestamp + 86_399
}

fn deserialize_optional_after_timestamp<'de, D>(deserializer: D) -> std::result::Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional_timestamp(deserializer, false)
}

fn deserialize_optional_before_timestamp<'de, D>(deserializer: D) -> std::result::Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional_timestamp(deserializer, true)
}

fn deserialize_optional_timestamp<'de, D>(
    deserializer: D,
    end_of_day: bool,
) -> std::result::Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(number)) => Ok(number.as_i64()),
        Some(serde_json::Value::String(text)) => {
            let parsed = parse_date_to_timestamp(&text).map(|timestamp| {
                if end_of_day {
                    end_of_day_timestamp(timestamp)
                } else {
                    timestamp
                }
            });
            Ok(parsed)
        }
        Some(other) => Err(serde::de::Error::custom(format!(
            "unsupported timestamp value: {}",
            other
        ))),
    }
}

#[cfg(test)]
mod date_parser_tests {
    use super::parse_date_to_timestamp;

    #[test]
    fn parses_iso_format() {
        assert!(parse_date_to_timestamp("2025-01-01").is_some());
    }

    #[test]
    fn parses_dmy_dash_format() {
        // 01-01-2025 → 2025-01-01
        assert_eq!(
            parse_date_to_timestamp("01-01-2025"),
            parse_date_to_timestamp("2025-01-01"),
        );
    }

    #[test]
    fn parses_dmy_slash_format() {
        assert_eq!(
            parse_date_to_timestamp("01/01/2025"),
            parse_date_to_timestamp("2025-01-01"),
        );
    }

    #[test]
    fn parses_iso_slash_format() {
        assert_eq!(
            parse_date_to_timestamp("2025/01/01"),
            parse_date_to_timestamp("2025-01-01"),
        );
    }

    #[test]
    fn rejects_invalid_month() {
        assert!(parse_date_to_timestamp("2025-13-01").is_none());
    }

    #[test]
    fn rejects_junk() {
        assert!(parse_date_to_timestamp("yesterday").is_none());
        assert!(parse_date_to_timestamp("2025-01").is_none());
    }
}
