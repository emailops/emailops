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
}

#[derive(Debug, Deserialize)]
struct ChatMessageContent {
    content: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct UsageInfo {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
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

        let request = OpenRouterChatRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            stream: false,
            max_tokens: options.max_tokens,
            temperature: options.temperature,
        };

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
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!("OpenRouter error: {}", error_text)));
        }

        let cost_from_headers = extract_cost_from_response(&response);

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

        let cost_usd = cost_from_headers;

        Ok(CompletionResult {
            text,
            prompt_tokens,
            completion_tokens,
            cost_usd,
            model: self.model.clone(),
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
        let request = OpenRouterEmbeddingRequest {
            model: self.embedding_model.clone(),
            input: text.to_string(),
            encoding_format: "float".to_string(),
        };

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
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::AiError(format!("OpenRouter embedding error: {}", error_text)));
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

fn extract_cost_from_response(response: &reqwest::Response) -> f64 {
    if let Some(cost_str) = response.headers().get("X-OpenRouter-Total-Cost") {
        if let Ok(cost_str) = cost_str.to_str() {
            if let Ok(cost) = cost_str.parse::<f64>() {
                return cost;
            }
        }
    }
    0.0
}
