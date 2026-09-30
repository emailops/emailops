use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::ai::openrouter_stream::{
    parse_sse_line, wire_messages, SseEvent, SseLines, StreamAccumulator, StreamOutcome, WireMessage,
};
use crate::ai::provider::{
    AIProvider, AiMessage, BackendCapabilities, ChatStreamResult, CompletionOptions, CompletionResult, EmbeddingResult,
    ModelInfo, ModelPricing, ProviderType, ToolStreamResult,
};
use crate::models::error::{AppError, Result};

const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const APP_NAME: &str = "emailops";
const APP_URL: &str = "https://github.com/emailops";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const GENERATION_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a streamed reply may stay silent — before its first byte or
/// between two chunks — before it is given up on. OpenRouter sends keep-alive
/// comments while a model is busy, so a healthy stream is never this quiet.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Sampling temperature for chat turns: low, to keep answers grounded in the
/// retrieved mail (the same value the Ollama chat path uses).
const CHAT_TEMPERATURE: f64 = 0.2;

#[derive(Debug, Serialize)]
struct OpenRouterChatRequest {
    model: String,
    messages: Vec<WireMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    /// Structured output: the reply must follow a JSON Schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    /// Tool definitions, in the `{"type": "function", "function": {…}}` form
    /// the chat tool registry already produces.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    provider: ProviderPreferences,
}

/// OpenRouter routing constraints sent with every request that carries mail
/// content. `data_collection: "deny"` is fixed: Google's Workspace user-data
/// policy forbids letting Gmail data train a model, and OpenRouter's default
/// ("allow") would route to providers that store and train on prompts. Zero
/// data retention is stricter and costs models, so it is the user's choice.
#[derive(Debug, Serialize)]
struct ProviderPreferences {
    data_collection: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    zdr: bool,
}

impl ProviderPreferences {
    fn new(zero_data_retention: bool) -> Self {
        Self {
            data_collection: "deny",
            zdr: zero_data_retention,
        }
    }
}

/// OpenRouter's answer when no provider for the model meets the request's
/// data policy.
const DATA_POLICY_REJECTION: &str = "No endpoints found matching your data policy";

/// The error for a failed OpenRouter request: a data-policy rejection names the
/// blocked model so the user knows to choose another; anything else keeps the
/// raw body for the log.
fn request_error(status: u16, body: &str, model: &str, context: &str) -> AppError {
    if status == 404 && body.contains(DATA_POLICY_REJECTION) {
        return AppError::AiDataPolicy {
            model: model.to_string(),
        };
    }
    AppError::AiError(format!("{context}: {body}"))
}

/// The error for a chat turn OpenRouter refused before streaming anything.
/// Says what the status means, so a rate limit or an outage does not reach
/// the user as a bare JSON body.
fn stream_request_error(status: u16, body: &str, model: &str) -> AppError {
    let what = match status {
        402 => "OpenRouter refused the request: the account is out of credits",
        429 => "OpenRouter rate limit reached — wait a moment and try again",
        500..=599 => "OpenRouter or the model's provider is unavailable — try again, or choose another model",
        _ => "OpenRouter chat error",
    };
    request_error(status, body, model, &format!("{what} (HTTP {status})"))
}

fn stream_stalled(idle: Duration) -> AppError {
    AppError::AiError(format!(
        "OpenRouter stopped responding ({}s without data)",
        idle.as_secs_f32()
    ))
}

/// OpenRouter's structured-output request for `shape`, strict so the model
/// may not add or drop fields.
fn response_format(shape: Option<&crate::ai::json_shape::JsonShape>) -> Option<serde_json::Value> {
    shape.map(|shape| {
        serde_json::json!({
            "type": "json_schema",
            "json_schema": { "name": "reply", "strict": true, "schema": shape.to_json_schema() },
        })
    })
}

#[derive(Debug, Deserialize)]
struct OpenRouterChatResponse {
    choices: Vec<ChatChoice>,
    usage: Option<UsageInfo>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageContent,
    /// `"length"` when the reply stopped at `max_tokens`.
    #[serde(default)]
    finish_reason: Option<String>,
}

/// Whether the reply stopped at `max_tokens`.
fn first_choice_truncated(response: &OpenRouterChatResponse) -> bool {
    response.choices.first().and_then(|c| c.finish_reason.as_deref()) == Some("length")
}

#[derive(Debug, Deserialize)]
struct ChatMessageContent {
    content: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct UsageInfo {
    pub(super) prompt_tokens: Option<u32>,
    pub(super) completion_tokens: Option<u32>,
    /// Credits charged for the request, reported in the body on every response.
    pub(super) cost: Option<f64>,
}

/// What OpenRouter charged for a completion, from the body's `usage.cost`
/// (OpenRouter sends no cost header). Missing usage counts as free.
fn completion_cost(response: &OpenRouterChatResponse) -> f64 {
    response.usage.as_ref().and_then(|u| u.cost).unwrap_or(0.0)
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModelInfo>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterEmbeddingsResponse {
    data: Vec<OpenRouterEmbeddingModelInfo>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelInfo {
    id: String,
    name: Option<String>,
    pricing: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenRouterEmbeddingModelInfo {
    id: String,
    name: Option<String>,
    pricing: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct OpenRouterEmbeddingRequest {
    model: String,
    input: String,
    encoding_format: String,
    provider: ProviderPreferences,
}

#[derive(Debug, Deserialize)]
struct OpenRouterEmbeddingResponse {
    data: Vec<EmbeddingDataItem>,
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingDataItem {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingUsage {
    prompt_tokens: Option<u32>,
    cost: Option<f64>,
}

pub struct OpenRouterClient {
    client: Client,
    api_key: String,
    model: String,
    embedding_model: String,
    zero_data_retention: bool,
    base_url: String,
    stream_idle_timeout: Duration,
}

impl OpenRouterClient {
    pub fn new(api_key: String, model: String, embedding_model: String) -> Self {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            api_key,
            model,
            embedding_model,
            zero_data_retention: false,
            base_url: OPENROUTER_BASE_URL.to_string(),
            stream_idle_timeout: STREAM_IDLE_TIMEOUT,
        }
    }

    /// Send requests to `base_url` (a mock server) instead of openrouter.ai.
    #[cfg(test)]
    pub(crate) fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Route only to providers with a zero-data-retention policy.
    pub fn with_zero_data_retention(mut self, enabled: bool) -> Self {
        self.zero_data_retention = enabled;
        self
    }

    fn chat_request(&self, prompt: &str, options: &CompletionOptions) -> OpenRouterChatRequest {
        OpenRouterChatRequest {
            model: self.model.clone(),
            messages: vec![WireMessage::text("user", prompt)],
            stream: false,
            max_tokens: options.max_tokens,
            temperature: options.temperature,
            response_format: response_format(options.json_shape.as_ref()),
            tools: None,
            provider: ProviderPreferences::new(self.zero_data_retention),
        }
    }

    /// The request for a streamed chat turn. Carries the same provider data
    /// policy as every other request that holds mail content.
    fn stream_request(&self, messages: &[AiMessage], tools: &[serde_json::Value]) -> OpenRouterChatRequest {
        OpenRouterChatRequest {
            model: self.model.clone(),
            messages: wire_messages(messages),
            stream: true,
            max_tokens: None,
            temperature: Some(CHAT_TEMPERATURE),
            response_format: None,
            tools: (!tools.is_empty()).then(|| tools.to_vec()),
            provider: ProviderPreferences::new(self.zero_data_retention),
        }
    }

    fn embedding_request(&self, text: &str) -> OpenRouterEmbeddingRequest {
        OpenRouterEmbeddingRequest {
            model: self.embedding_model.clone(),
            input: text.to_string(),
            encoding_format: "float".to_string(),
            provider: ProviderPreferences::new(self.zero_data_retention),
        }
    }

    /// Run one streamed chat completion. Prose reaches `on_token` as it
    /// arrives; tool calls and usage come back in the outcome. `on_token`
    /// returning `false` stops reading and drops the connection, which is how
    /// OpenRouter is told to stop generating.
    ///
    /// Never retried: by the time a failure shows, part of the reply may
    /// already be on screen.
    async fn stream_chat(
        &self,
        messages: &[AiMessage],
        tools: &[serde_json::Value],
        mut on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<StreamOutcome> {
        let idle = self.stream_idle_timeout;
        let send = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", APP_URL)
            .header("X-OpenRouter-Title", APP_NAME)
            .header("Content-Type", "application/json")
            .json(&self.stream_request(messages, tools))
            .send();
        let response = tokio::time::timeout(idle, send)
            .await
            .map_err(|_| stream_stalled(idle))?
            .map_err(|e| AppError::AiError(format!("Failed to connect to OpenRouter: {e}")))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(stream_request_error(status, &error_text, &self.model));
        }

        let mut stream = response.bytes_stream();
        let mut lines = SseLines::default();
        let mut reply = StreamAccumulator::default();
        loop {
            let next = tokio::time::timeout(idle, stream.next())
                .await
                .map_err(|_| stream_stalled(idle))?;
            let (batch, ended) = match next {
                Some(chunk) => {
                    let bytes = chunk.map_err(|e| AppError::AiError(format!("OpenRouter stream read error: {e}")))?;
                    (lines.push(&bytes), false)
                }
                None => (lines.finish().into_iter().collect(), true),
            };
            for line in batch {
                match parse_sse_line(&line)? {
                    None => {}
                    Some(SseEvent::Done) => return reply.finish(),
                    Some(SseEvent::Chunk(chunk)) => {
                        if let Some(prose) = reply.apply(chunk) {
                            if !on_token(prose) {
                                return Ok(reply.into_partial());
                            }
                        }
                    }
                }
            }
            if ended {
                return if reply.finished() {
                    reply.finish()
                } else {
                    Err(AppError::AiError(
                        "OpenRouter ended the reply before it was complete — the model's provider may have dropped the connection"
                            .to_string(),
                    ))
                };
            }
        }
    }

    async fn list_models_from_api(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/models", self.base_url);
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", APP_URL)
            .header("X-OpenRouter-Title", APP_NAME)
            .timeout(CONNECT_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to fetch OpenRouter models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::AiError("Failed to list OpenRouter models".to_string()));
        }

        let body: OpenRouterModelsResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse OpenRouter models: {}", e)))?;

        let models = body
            .data
            .into_iter()
            .map(|m| {
                let pricing = parse_openrouter_pricing(&m.pricing);
                ModelInfo {
                    id: m.id,
                    name: m.name.unwrap_or_else(|| "Unnamed model".to_string()),
                    pricing,
                }
            })
            .collect();

        Ok(models)
    }

    pub async fn list_embedding_models_from_api(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/embeddings/models", self.base_url);
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", APP_URL)
            .header("X-OpenRouter-Title", APP_NAME)
            .timeout(CONNECT_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to fetch OpenRouter embedding models: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!(
                "Failed to list OpenRouter embedding models: {}",
                error_text
            )));
        }

        let body: OpenRouterEmbeddingsResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse OpenRouter embedding models: {}", e)))?;

        Ok(body
            .data
            .into_iter()
            .map(|m| {
                let pricing = m
                    .pricing
                    .as_ref()
                    .map(parse_openrouter_pricing)
                    .unwrap_or(ModelPricing {
                        prompt: 0.0,
                        completion: 0.0,
                        request: 0.0,
                    });
                ModelInfo {
                    id: m.id,
                    name: m.name.unwrap_or_else(|| "Unnamed embedding model".to_string()),
                    pricing,
                }
            })
            .collect())
    }
}

#[async_trait]
impl AIProvider for OpenRouterClient {
    fn provider_type(&self) -> ProviderType {
        ProviderType::OpenRouter
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    async fn is_available(&self) -> bool {
        let url = format!("{}/models", self.base_url);
        self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", APP_URL)
            .header("X-OpenRouter-Title", APP_NAME)
            .timeout(CONNECT_TIMEOUT)
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.list_models_from_api().await
    }

    async fn complete(&self, prompt: &str, options: CompletionOptions) -> Result<CompletionResult> {
        let url = format!("{}/chat/completions", self.base_url);

        let request = self.chat_request(prompt, &options);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", APP_URL)
            .header("X-OpenRouter-Title", APP_NAME)
            .header("Content-Type", "application/json")
            .timeout(GENERATION_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::AiError(format!(
                        "OpenRouter generation timed out ({}s)",
                        GENERATION_TIMEOUT.as_secs()
                    ))
                } else {
                    AppError::AiError(format!("Failed to connect to OpenRouter: {}", e))
                }
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(request_error(status, &error_text, &self.model, "OpenRouter error"));
        }

        let result: OpenRouterChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse OpenRouter response: {}", e)))?;

        let text = result
            .choices
            .first()
            .map(|c| openrouter_content_to_text(&c.message.content))
            .unwrap_or_default();

        let prompt_tokens = result.usage.as_ref().and_then(|u| u.prompt_tokens).unwrap_or(0);
        let completion_tokens = result.usage.as_ref().and_then(|u| u.completion_tokens).unwrap_or(0);

        let cost_usd = completion_cost(&result);
        let truncated = first_choice_truncated(&result);

        Ok(CompletionResult {
            text,
            prompt_tokens,
            completion_tokens,
            cost_usd,
            model: self.model.clone(),
            prefill_ms: None,
            cached_prompt_tokens: None,
            aux_plan: None,
            truncated,
        })
    }

    fn embedding_model_name(&self) -> &str {
        &self.embedding_model
    }

    async fn list_embedding_models(&self) -> Result<Vec<ModelInfo>> {
        self.list_embedding_models_from_api().await
    }

    async fn embed(&self, text: &str) -> Result<EmbeddingResult> {
        let url = format!("{}/embeddings", self.base_url);
        let request = self.embedding_request(text);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", APP_URL)
            .header("X-OpenRouter-Title", APP_NAME)
            .header("Content-Type", "application/json")
            .timeout(GENERATION_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::AiError(format!(
                        "OpenRouter embedding timed out ({}s)",
                        GENERATION_TIMEOUT.as_secs()
                    ))
                } else {
                    AppError::AiError(format!("Failed to connect to OpenRouter embeddings: {}", e))
                }
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(request_error(
                status,
                &error_text,
                &self.embedding_model,
                "OpenRouter embedding error",
            ));
        }

        let body: OpenRouterEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse OpenRouter embedding response: {}", e)))?;

        let embedding = body
            .data
            .first()
            .map(|item| item.embedding.clone())
            .ok_or_else(|| AppError::AiError("OpenRouter returned no embedding vector".to_string()))?;

        Ok(EmbeddingResult {
            embedding,
            tokens: body.usage.as_ref().and_then(|usage| usage.prompt_tokens).unwrap_or(0),
            cost_usd: body.usage.as_ref().and_then(|usage| usage.cost).unwrap_or(0.0),
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
        let outcome = self.stream_chat(messages, tools, Box::new(|_| true)).await?;
        Ok(assistant_message(outcome))
    }

    async fn chat_stream(
        &self,
        messages: Vec<AiMessage>,
        on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ChatStreamResult> {
        let outcome = self.stream_chat(&messages, &[], on_token).await?;
        Ok(ChatStreamResult {
            eval_count: outcome.usage.as_ref().and_then(|u| u.completion_tokens),
            prompt_eval_count: outcome.usage.as_ref().and_then(|u| u.prompt_tokens),
            cost_usd: outcome.usage.as_ref().and_then(|u| u.cost),
            content: outcome.content,
            ..Default::default()
        })
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<AiMessage>,
        tools: Vec<serde_json::Value>,
        on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ToolStreamResult> {
        let outcome = self.stream_chat(&messages, &tools, on_token).await?;
        Ok(ToolStreamResult {
            eval_count: outcome.usage.as_ref().and_then(|u| u.completion_tokens),
            prompt_eval_count: outcome.usage.as_ref().and_then(|u| u.prompt_tokens),
            cost_usd: outcome.usage.as_ref().and_then(|u| u.cost),
            message: assistant_message(outcome),
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
}

/// The assistant turn a finished stream amounts to. When it asks for tools
/// its prose is dropped — as on the other backends, a tool-call turn
/// dispatches calls rather than surfacing text.
fn assistant_message(outcome: StreamOutcome) -> AiMessage {
    let has_tool_calls = !outcome.tool_calls.is_empty();
    AiMessage {
        role: "assistant".to_string(),
        content: if has_tool_calls { String::new() } else { outcome.content },
        tool_calls: has_tool_calls.then_some(outcome.tool_calls),
    }
}

fn parse_openrouter_pricing(pricing: &serde_json::Value) -> ModelPricing {
    let prompt = pricing.get("prompt").and_then(parse_openrouter_number).unwrap_or(0.0);
    let completion = pricing
        .get("completion")
        .and_then(parse_openrouter_number)
        .unwrap_or(0.0);
    let request = pricing.get("request").and_then(parse_openrouter_number).unwrap_or(0.0);

    ModelPricing {
        prompt,
        completion,
        request,
    }
}

fn parse_openrouter_number(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
}

fn openrouter_content_to_text(content: &serde_json::Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }

    if let Some(parts) = content.as_array() {
        let joined = parts
            .iter()
            .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return joined;
        }
    }

    String::new()
}

#[cfg(test)]
mod data_policy_tests {
    use super::*;

    fn client(zdr: bool) -> OpenRouterClient {
        OpenRouterClient::new("key".into(), "vendor/model".into(), "vendor/embed".into()).with_zero_data_retention(zdr)
    }

    #[test]
    fn every_chat_request_denies_data_collection() {
        let body = serde_json::to_value(client(false).chat_request("hi", &CompletionOptions::default())).unwrap();
        assert_eq!(body["provider"], serde_json::json!({ "data_collection": "deny" }));
    }

    #[test]
    fn a_chat_request_asks_for_zero_retention_when_enabled() {
        let body = serde_json::to_value(client(true).chat_request("hi", &CompletionOptions::default())).unwrap();
        assert_eq!(
            body["provider"],
            serde_json::json!({ "data_collection": "deny", "zdr": true })
        );
    }

    #[test]
    fn every_embedding_request_carries_the_same_policy() {
        let body = serde_json::to_value(client(false).embedding_request("hi")).unwrap();
        assert_eq!(body["provider"], serde_json::json!({ "data_collection": "deny" }));
        let body = serde_json::to_value(client(true).embedding_request("hi")).unwrap();
        assert_eq!(
            body["provider"],
            serde_json::json!({ "data_collection": "deny", "zdr": true })
        );
    }

    #[test]
    fn a_data_policy_404_names_the_blocked_model() {
        let body = r#"{"error":{"message":"No endpoints found matching your data policy (Free model training). Configure: https://openrouter.ai/settings/privacy","code":404}}"#;
        match request_error(404, body, "vendor/model", "OpenRouter error") {
            AppError::AiDataPolicy { model } => assert_eq!(model, "vendor/model"),
            other => panic!("expected AiDataPolicy, got {other:?}"),
        }
    }

    #[test]
    fn other_failures_stay_generic_ai_errors() {
        let body = r#"{"error":{"message":"Provider returned error","code":429}}"#;
        assert!(matches!(
            request_error(429, body, "vendor/model", "OpenRouter error"),
            AppError::AiError(msg) if msg == format!("OpenRouter error: {body}")
        ));
        let body = r#"{"error":{"message":"x cannot be used with the chat/completions endpoint","code":404}}"#;
        assert!(matches!(
            request_error(404, body, "vendor/model", "OpenRouter error"),
            AppError::AiError(_)
        ));
    }
}

#[cfg(test)]
mod stop_reason_tests {
    use super::*;

    #[test]
    fn a_json_shape_becomes_a_strict_response_format() {
        use crate::ai::json_shape::JsonShape;
        let shape = JsonShape::object(vec![("tag", JsonShape::one_of(&["match", "context"]))]);
        let format = response_format(Some(&shape)).expect("a format");
        assert_eq!(format["type"], "json_schema");
        assert_eq!(format["json_schema"]["strict"], true);
        assert_eq!(format["json_schema"]["schema"], shape.to_json_schema());
        assert!(response_format(None).is_none());
    }

    #[test]
    fn the_completion_cost_comes_from_the_body_usage() {
        let r: OpenRouterChatResponse = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"x"}}],"usage":{"prompt_tokens":194,"completion_tokens":2,"cost":0.0125}}"#,
        )
        .unwrap();
        assert_eq!(completion_cost(&r), 0.0125);
        let r: OpenRouterChatResponse = serde_json::from_str(r#"{"choices":[{"message":{"content":"x"}}]}"#).unwrap();
        assert_eq!(completion_cost(&r), 0.0);
    }

    #[test]
    fn a_choice_that_hit_max_tokens_is_truncated() {
        let r: OpenRouterChatResponse =
            serde_json::from_str(r#"{"choices":[{"message":{"content":"x"},"finish_reason":"length"}],"usage":null}"#)
                .unwrap();
        assert!(first_choice_truncated(&r));
        let r: OpenRouterChatResponse =
            serde_json::from_str(r#"{"choices":[{"message":{"content":"x"},"finish_reason":"stop"}]}"#).unwrap();
        assert!(!first_choice_truncated(&r));
        let r: OpenRouterChatResponse = serde_json::from_str(r#"{"choices":[{"message":{"content":"x"}}]}"#).unwrap();
        assert!(!first_choice_truncated(&r));
    }
}

#[cfg(test)]
mod chat_stream_tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::ai::provider::{AiToolCall, AiToolCallFunction};

    /// An SSE body: one `data:` event per entry, blank-line separated.
    fn sse(events: &[&str]) -> String {
        events.iter().map(|event| format!("data: {event}\n\n")).collect()
    }

    fn client(server: &MockServer) -> OpenRouterClient {
        OpenRouterClient::new("key".into(), "vendor/model".into(), "vendor/embed".into()).with_base_url(server.uri())
    }

    async fn server_replying(body: String) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        server
    }

    fn user(content: &str) -> AiMessage {
        AiMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_calls: None,
        }
    }

    fn search_tool() -> serde_json::Value {
        json!({"type": "function", "function": {
            "name": "search_emails",
            "description": "Search the mailbox",
            "parameters": {"type": "object", "properties": {"query": {"type": "string"}}},
        }})
    }

    type Tokens = Arc<Mutex<Vec<String>>>;

    /// A callback that records every token and keeps going while `keep_going`
    /// says so.
    fn recording(
        keep_going: impl Fn(usize) -> bool + Send + 'static,
    ) -> (Tokens, Box<dyn FnMut(String) -> bool + Send>) {
        let tokens: Tokens = Arc::default();
        let sink = tokens.clone();
        let callback = Box::new(move |token: String| {
            let mut seen = sink.lock().unwrap_or_else(PoisonError::into_inner);
            seen.push(token);
            keep_going(seen.len())
        });
        (tokens, callback)
    }

    fn seen(tokens: &Tokens) -> Vec<String> {
        tokens.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    async fn request_bodies(server: &MockServer) -> Vec<serde_json::Value> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect()
    }

    #[test]
    fn the_backend_reports_tools_and_streaming() {
        let caps = OpenRouterClient::new("key".into(), "m".into(), "e".into()).capabilities();
        assert!(caps.tools && caps.streaming && caps.embeddings);
    }

    #[tokio::test]
    async fn a_streamed_answer_arrives_token_by_token_with_its_usage() {
        let body = format!(
            ": OPENROUTER PROCESSING\n\n{}",
            sse(&[
                r#"{"choices":[{"delta":{"role":"assistant","content":"The invoice "}}]}"#,
                r#"{"choices":[{"delta":{"reasoning":"the user wants the total"}}]}"#,
                r#"{"choices":[{"delta":{"content":"is paid."}}]}"#,
                r#"{"choices":[{"delta":{"content":""},"finish_reason":"stop"}]}"#,
                r#"{"choices":[],"usage":{"prompt_tokens":812,"completion_tokens":5,"cost":0.00042}}"#,
                "[DONE]",
            ])
        );
        let server = server_replying(body).await;
        let (tokens, on_token) = recording(|_| true);

        let result = client(&server)
            .chat_stream(vec![user("Is the invoice paid?")], on_token)
            .await
            .unwrap();

        assert_eq!(seen(&tokens), vec!["The invoice ", "is paid."]);
        assert_eq!(result.content, "The invoice is paid.");
        assert_eq!(result.prompt_eval_count, Some(812));
        assert_eq!(result.eval_count, Some(5));
        assert_eq!(result.cost_usd, Some(0.00042));
    }

    /// The chat loop's path: a round that asks for a tool, the tool result
    /// appended, and a second round that answers from it.
    #[tokio::test]
    async fn a_tool_round_trip_ends_in_an_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                sse(&[
                    r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_x","type":"function","function":{"name":"search_emails","arguments":""}}]}}]}"#,
                    r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":\"invoice\"}"}}]}}]}"#,
                    r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
                    r#"{"choices":[],"usage":{"prompt_tokens":300,"completion_tokens":12,"cost":0.001}}"#,
                    "[DONE]",
                ]),
                "text/event-stream",
            ))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                sse(&[
                    r#"{"choices":[{"delta":{"content":"One invoice, paid."},"finish_reason":"stop"}]}"#,
                    r#"{"choices":[],"usage":{"prompt_tokens":340,"completion_tokens":6,"cost":0.002}}"#,
                    "[DONE]",
                ]),
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let client = client(&server);
        let mut messages = vec![user("Find the invoice")];

        let (tokens, on_token) = recording(|_| true);
        let first = client
            .chat_stream_with_tools(messages.clone(), vec![search_tool()], on_token)
            .await
            .unwrap();
        assert!(seen(&tokens).is_empty(), "a tool-call round streams no prose");
        let calls = first.message.tool_calls.clone().expect("a tool call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(calls[0].function.arguments, json!({"query": "invoice"}));
        assert_eq!(first.cost_usd, Some(0.001));

        messages.push(first.message);
        messages.push(AiMessage {
            role: "tool".to_string(),
            content: "1 email: Invoice 2041 (paid)".to_string(),
            tool_calls: None,
        });
        let (tokens, on_token) = recording(|_| true);
        let second = client
            .chat_stream_with_tools(messages, vec![search_tool()], on_token)
            .await
            .unwrap();
        assert_eq!(seen(&tokens), vec!["One invoice, paid."]);
        assert_eq!(second.message.content, "One invoice, paid.");
        assert!(second.message.tool_calls.is_none());
        assert_eq!(second.prompt_eval_count, Some(340));
        assert_eq!(second.cost_usd, Some(0.002));

        let bodies = request_bodies(&server).await;
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0]["stream"], true);
        assert_eq!(bodies[0]["tools"], json!([search_tool()]));
        let history = &bodies[1]["messages"];
        let call_id = history[1]["tool_calls"][0]["id"].as_str().expect("a call id");
        assert_eq!(history[1]["tool_calls"][0]["function"]["name"], "search_emails");
        assert_eq!(
            history[1]["tool_calls"][0]["function"]["arguments"],
            "{\"query\":\"invoice\"}"
        );
        assert_eq!(history[2]["role"], "tool");
        assert_eq!(history[2]["tool_call_id"], call_id);
    }

    #[tokio::test]
    async fn the_blocking_tool_call_returns_the_same_message() {
        let server = server_replying(sse(&[
            r#"{"choices":[{"delta":{"content":"Let me look."}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"search_emails","arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ]))
        .await;
        let message = client(&server)
            .chat_with_tools(&[user("Find it")], &[search_tool()])
            .await
            .unwrap();
        assert_eq!(message.role, "assistant");
        assert_eq!(
            message.content, "",
            "a tool-call turn carries no prose into the history"
        );
        assert_eq!(message.tool_calls.expect("a call")[0].function.name, "search_emails");
    }

    #[tokio::test]
    async fn an_error_mid_stream_fails_the_reply() {
        let server = server_replying(sse(&[
            r#"{"choices":[{"delta":{"content":"Half an"}}]}"#,
            r#"{"error":{"code":"server_error","message":"Provider disconnected unexpectedly"},"choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}]}"#,
        ]))
        .await;
        let (tokens, on_token) = recording(|_| true);
        let result = client(&server).chat_stream(vec![user("Hi")], on_token).await;
        assert!(
            matches!(&result, Err(AppError::AiError(m)) if m.contains("Provider disconnected unexpectedly")),
            "{result:?}"
        );
        assert_eq!(seen(&tokens), vec!["Half an"]);
    }

    #[tokio::test]
    async fn a_stream_that_ends_before_the_model_finished_is_an_error() {
        let server = server_replying(sse(&[r#"{"choices":[{"delta":{"content":"Half an"}}]}"#])).await;
        let result = client(&server).chat_stream(vec![user("Hi")], Box::new(|_| true)).await;
        assert!(result.is_err(), "a cut-off reply must not pass as complete");
    }

    /// Some upstream providers close the stream after the finishing chunk
    /// without the `[DONE]` sentinel.
    #[tokio::test]
    async fn a_finished_reply_without_the_done_sentinel_is_complete() {
        let server = server_replying(sse(&[
            r#"{"choices":[{"delta":{"content":"Hi"},"finish_reason":"stop"}]}"#,
        ]))
        .await;
        let result = client(&server)
            .chat_stream(vec![user("Hi")], Box::new(|_| true))
            .await
            .unwrap();
        assert_eq!(result.content, "Hi");
    }

    #[tokio::test]
    async fn returning_false_from_the_callback_stops_the_reply() {
        let server = server_replying(sse(&[
            r#"{"choices":[{"delta":{"content":"One"}}]}"#,
            r#"{"choices":[{"delta":{"content":" two"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"search_emails","arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ]))
        .await;
        let (tokens, on_token) = recording(|_| false);
        let result = client(&server)
            .chat_stream_with_tools(vec![user("Hi")], vec![search_tool()], on_token)
            .await
            .unwrap();
        assert_eq!(seen(&tokens), vec!["One"], "nothing is read after the cancel");
        assert_eq!(result.message.content, "One");
        assert!(result.message.tool_calls.is_none(), "a cancelled reply runs no tool");
    }

    #[tokio::test]
    async fn malformed_tool_arguments_fail_the_round() {
        let server = server_replying(sse(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"search_emails","arguments":"{\"query\":\"inv"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
            "[DONE]",
        ]))
        .await;
        let result = client(&server)
            .chat_stream_with_tools(vec![user("Hi")], vec![search_tool()], Box::new(|_| true))
            .await;
        assert!(
            matches!(&result, Err(AppError::AiError(m)) if m.contains("search_emails")),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn every_chat_method_sends_the_data_policy() {
        let done = sse(&[
            r#"{"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]);
        for (zdr, policy) in [
            (false, json!({"data_collection": "deny"})),
            (true, json!({"data_collection": "deny", "zdr": true})),
        ] {
            let server = server_replying(done.clone()).await;
            let client = client(&server).with_zero_data_retention(zdr);
            client.chat_stream(vec![user("Hi")], Box::new(|_| true)).await.unwrap();
            client
                .chat_stream_with_tools(vec![user("Hi")], vec![search_tool()], Box::new(|_| true))
                .await
                .unwrap();
            client.chat_with_tools(&[user("Hi")], &[search_tool()]).await.unwrap();
            let bodies = request_bodies(&server).await;
            assert_eq!(bodies.len(), 3);
            for body in bodies {
                assert_eq!(body["provider"], policy, "zdr={zdr}");
                assert_eq!(body["model"], "vendor/model");
            }
        }
    }

    #[tokio::test]
    async fn a_request_without_tools_sends_no_tools_field() {
        let server = server_replying(sse(&["[DONE]"])).await;
        client(&server)
            .chat_stream(vec![user("Hi")], Box::new(|_| true))
            .await
            .unwrap();
        assert!(request_bodies(&server).await[0].get("tools").is_none());
    }

    /// A rate limit or an upstream outage is reported once, in words; the
    /// turn is not retried behind the user's back.
    #[tokio::test]
    async fn a_refused_request_is_a_clear_error_and_is_not_retried() {
        for (status, body, expected) in [
            (
                429,
                r#"{"error":{"message":"Rate limit exceeded","code":429}}"#,
                "rate limit",
            ),
            (
                402,
                r#"{"error":{"message":"Insufficient credits","code":402}}"#,
                "credits",
            ),
            (502, r#"{"error":{"message":"Bad gateway","code":502}}"#, "unavailable"),
            (
                503,
                r#"{"error":{"message":"No provider available","code":503}}"#,
                "unavailable",
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat/completions"))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .mount(&server)
                .await;
            let result = client(&server)
                .chat_stream_with_tools(vec![user("Hi")], vec![search_tool()], Box::new(|_| true))
                .await;
            assert!(
                matches!(&result, Err(AppError::AiError(m))
                    if m.contains(expected) && m.contains(&status.to_string())),
                "{status}: {result:?}"
            );
            assert_eq!(server.received_requests().await.unwrap().len(), 1, "{status} retried");
        }
    }

    #[tokio::test]
    async fn a_model_blocked_by_the_data_policy_is_named() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(404).set_body_string(
                r#"{"error":{"message":"No endpoints found matching your data policy (Zero data retention)","code":404}}"#,
            ))
            .mount(&server)
            .await;
        let result = client(&server).chat_stream(vec![user("Hi")], Box::new(|_| true)).await;
        assert!(
            matches!(&result, Err(AppError::AiDataPolicy { model }) if model == "vendor/model"),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn a_silent_server_times_out() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse(&["[DONE]"]), "text/event-stream")
                    .set_delay(Duration::from_secs(5)),
            )
            .mount(&server)
            .await;
        let client = OpenRouterClient {
            stream_idle_timeout: Duration::from_millis(50),
            ..client(&server)
        };
        let result = client.chat_stream(vec![user("Hi")], Box::new(|_| true)).await;
        assert!(
            matches!(&result, Err(AppError::AiError(m)) if m.contains("stopped responding")),
            "{result:?}"
        );
    }

    #[test]
    fn a_tool_call_history_keeps_its_structure_on_the_wire() {
        let client = OpenRouterClient::new("key".into(), "vendor/model".into(), "vendor/embed".into());
        let request = client.stream_request(
            &[
                user("Find it"),
                AiMessage {
                    role: "assistant".to_string(),
                    content: String::new(),
                    tool_calls: Some(vec![AiToolCall {
                        function: AiToolCallFunction {
                            name: "search_emails".to_string(),
                            arguments: json!({"query": "x"}),
                        },
                    }]),
                },
            ],
            &[],
        );
        let body = serde_json::to_value(request).unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["messages"][1]["tool_calls"][0]["type"], "function");
        assert!(body.get("reasoning").is_none(), "reasoning is left to the model");
    }
}
