use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Serialize)]
struct OpenRouterChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    /// Structured output: the reply must follow a JSON Schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
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

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
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
struct UsageInfo {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    /// Credits charged for the request, reported in the body on every response.
    cost: Option<f64>,
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
        }
    }

    /// Route only to providers with a zero-data-retention policy.
    pub fn with_zero_data_retention(mut self, enabled: bool) -> Self {
        self.zero_data_retention = enabled;
        self
    }

    fn chat_request(&self, prompt: &str, options: &CompletionOptions) -> OpenRouterChatRequest {
        OpenRouterChatRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            stream: false,
            max_tokens: options.max_tokens,
            temperature: options.temperature,
            response_format: response_format(options.json_shape.as_ref()),
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

    async fn list_models_from_api(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/models", OPENROUTER_BASE_URL);
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
        let url = format!("{}/embeddings/models", OPENROUTER_BASE_URL);
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
        let url = format!("{}/models", OPENROUTER_BASE_URL);
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
        let url = format!("{}/chat/completions", OPENROUTER_BASE_URL);

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
        let url = format!("{}/embeddings", OPENROUTER_BASE_URL);
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

    async fn chat_with_tools(&self, _messages: &[AiMessage], _tools: &[serde_json::Value]) -> Result<AiMessage> {
        Err(AppError::AiError(
            "Tool-calling is not supported for OpenRouter backend".to_string(),
        ))
    }

    async fn chat_stream(
        &self,
        _messages: Vec<AiMessage>,
        _on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ChatStreamResult> {
        Err(AppError::AiError(
            "Streaming is not supported for OpenRouter backend".to_string(),
        ))
    }

    async fn chat_stream_with_tools(
        &self,
        _messages: Vec<AiMessage>,
        _tools: Vec<serde_json::Value>,
        _on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ToolStreamResult> {
        // OpenRouter is wired as an embeddings/judge backend only here; chat and
        // tool-calling are intentionally unsupported (see `chat_with_tools`).
        Err(AppError::AiError(
            "Streaming tool-calls are not supported for OpenRouter backend".to_string(),
        ))
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            tools: false,
            streaming: false,
            embeddings: true,
        }
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
