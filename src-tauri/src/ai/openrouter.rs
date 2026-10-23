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
use crate::ai::thinking_filter::{strip_reasoning, ThinkingGate};
use crate::db::embeddings::EMAIL_EMBEDDING_DIM;
use crate::models::error::{AppError, Result};

const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Which server this client talks to. Both speak the OpenAI Chat Completions
/// API; OpenRouter adds a routing/data-policy object and attribution headers
/// that a plain OpenAI-compatible server (LM Studio, vLLM, llama-server,
/// LiteLLM, a local proxy…) does not know and may reject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// openrouter.ai, with its data policy and `vendor/model` ids.
    OpenRouter,
    /// A user-configured OpenAI-compatible server. Model ids are whatever the
    /// server lists; the API key is optional (many local servers need none).
    OpenAiCompatible,
}
const APP_NAME: &str = "emailops";
const APP_URL: &str = "https://github.com/emailops";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const GENERATION_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a streamed reply may stay silent — before its first byte or
/// between two chunks — before it is given up on. OpenRouter sends keep-alive
/// comments while a model is busy, so a healthy stream is never this quiet.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long an OpenAI-compatible server may take to send the first token of a
/// streamed reply. A local server sends nothing while it prefills a long
/// prompt, which on a laptop can take minutes; the idle window applies after.
const COMPATIBLE_FIRST_TOKEN_TIMEOUT: Duration = Duration::from_secs(300);
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
    /// OpenRouter only; omitted for a plain OpenAI-compatible server.
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderPreferences>,
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

/// Longest server error message shown to the user, in characters.
const ERROR_SUMMARY_CHARS: usize = 200;
/// Longest error body written to the log, in bytes.
const ERROR_LOG_BYTES: usize = 4_096;

/// The part of a failed response's body that may be shown to the user: the
/// `error.message` / `message` / `detail` string of a JSON body, or a plain-text
/// body, whitespace collapsed and cut to [`ERROR_SUMMARY_CHARS`]. `None` for an
/// HTML page or anything else that is not a short message: a server can return
/// stack traces and internal paths there (DASA 1.8.1).
fn summarize_error_body(body: &str) -> Option<String> {
    let text = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(json) => [
            json.pointer("/error/message"),
            json.get("error"),
            json.get("message"),
            json.get("detail"),
        ]
        .into_iter()
        .flatten()
        .find_map(|v| v.as_str().map(str::to_string))?,
        Err(_) if body.trim_start().starts_with('<') => return None,
        Err(_) => body.to_string(),
    };
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= ERROR_SUMMARY_CHARS {
        return Some(collapsed);
    }
    Some(format!(
        "{}…",
        collapsed.chars().take(ERROR_SUMMARY_CHARS).collect::<String>()
    ))
}

/// The error for a failed request: a data-policy rejection names the blocked
/// model so the user knows to choose another; anything else carries `context`
/// (which callers end with the `(HTTP n)` status) and a trimmed summary of the
/// body. The full body
/// goes to the log only.
fn request_error(status: u16, body: &str, model: &str, context: &str) -> AppError {
    if status == 404 && body.contains(DATA_POLICY_REJECTION) {
        return AppError::AiDataPolicy {
            model: model.to_string(),
        };
    }
    let mut logged = body.to_string();
    if logged.len() > ERROR_LOG_BYTES {
        let cut = (0..=ERROR_LOG_BYTES)
            .rev()
            .find(|i| logged.is_char_boundary(*i))
            .unwrap_or(0);
        logged.truncate(cut);
    }
    crate::services::logger::log("debug", "ai", format!("{context}: {logged}"));
    match summarize_error_body(body) {
        Some(summary) => AppError::AiError(format!("{context}: {summary}")),
        None => AppError::AiError(context.to_string()),
    }
}

/// Whether `model` has the shape of an OpenRouter model id (`vendor/model`).
/// The chat-model preference is shared by every provider, so after a provider
/// switch it can still hold an in-app or Ollama id, which OpenRouter can only
/// answer with an opaque "not a valid model" body.
pub fn is_openrouter_model_id(model: &str) -> bool {
    !model.contains(char::is_whitespace)
        && model
            .split_once('/')
            .is_some_and(|(vendor, name)| !vendor.is_empty() && !name.is_empty())
}

/// The error for a chat turn the server refused before streaming anything.
/// Says what the status means, so a rate limit or an outage does not reach
/// the user as a bare JSON body. `server` names it (see `server_name`), so a
/// local server's failure is not blamed on OpenRouter.
fn stream_request_error(status: u16, body: &str, model: &str, server: &str) -> AppError {
    let what = match status {
        401 | 403 => format!("{server} refused the request: check the API key"),
        402 => format!("{server} refused the request: the account is out of credits"),
        404 => format!("{server} does not know the model \"{model}\" — choose another one in Settings → AI"),
        429 => format!("{server} rate limit reached — wait a moment and try again"),
        500..=599 => format!("{server} or the model's provider is unavailable — try again, or choose another model"),
        _ => format!("{server} chat error"),
    };
    request_error(status, body, model, &format!("{what} (HTTP {status})"))
}

fn stream_stalled(idle: Duration, server: &str) -> AppError {
    AppError::AiError(format!(
        "{server} stopped responding ({}s without data)",
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
    #[serde(default)]
    name: Option<String>,
    /// OpenRouter prices every model; a plain OpenAI-compatible server
    /// (`/v1/models` with only `id`) does not.
    #[serde(default)]
    pricing: serde_json::Value,
    /// Maximum context length of the model, in tokens.
    #[serde(default)]
    context_length: Option<u32>,
    /// vLLM's name for the context length.
    #[serde(default)]
    max_model_len: Option<u32>,
    #[serde(default)]
    top_provider: Option<OpenRouterTopProvider>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterTopProvider {
    /// Context length of the endpoint OpenRouter routes to first; can be
    /// smaller than the model's own.
    #[serde(default)]
    context_length: Option<u32>,
}

/// Model windows already read from the catalogue, by (base URL, model id).
fn known_windows() -> &'static std::sync::Mutex<std::collections::HashMap<(String, String), u32>> {
    static WINDOWS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(String, String), u32>>> =
        std::sync::OnceLock::new();
    WINDOWS.get_or_init(Default::default)
}

/// The context window of `model_id` according to the model catalogue: the
/// smaller of the model's own length and its top provider's, so a prompt sized
/// to it fits wherever the request lands. A routing suffix (`:nitro`,
/// `:floor`) is not a catalogue id and falls back to the base model; a listed
/// variant (`:free`) is looked up as is. Pure.
fn model_context_length(models: &[OpenRouterModelInfo], model_id: &str) -> Option<u32> {
    let base_id = model_id.split(':').next().unwrap_or(model_id);
    let model = models
        .iter()
        .find(|m| m.id == model_id)
        .or_else(|| models.iter().find(|m| m.id == base_id))?;
    let top = model.top_provider.as_ref().and_then(|p| p.context_length);
    [model.context_length, model.max_model_len, top]
        .into_iter()
        .flatten()
        .filter(|n| *n > 0)
        .min()
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
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
    /// OpenRouter only; omitted for a plain OpenAI-compatible server.
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderPreferences>,
}

/// How a validated embedding model is made to return vectors the email index
/// can hold ([`EMAIL_EMBEDDING_DIM`] floats).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingDimensions {
    /// The model honours the `dimensions` parameter: send it on every request.
    Requested,
    /// The model returns the right size on its own: send no `dimensions`.
    Native,
}

impl EmbeddingDimensions {
    /// The value stored in preferences for this mode.
    pub fn as_pref(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Native => "native",
        }
    }

    fn from_pref(raw: &str) -> Option<Self> {
        match raw {
            "requested" => Some(Self::Requested),
            "native" => Some(Self::Native),
            _ => None,
        }
    }

    fn request_value(self) -> Option<usize> {
        match self {
            Self::Requested => Some(EMAIL_EMBEDDING_DIM),
            Self::Native => None,
        }
    }
}

/// Whether `configured` (the embedding-model preference, shared by every
/// provider) may be used on OpenRouter: only when it is the model that passed
/// the probe, and then with the mode the probe found. A local model id left
/// over from another provider, an empty choice or a model changed since the
/// probe all answer `None`, and no embedding request is sent.
pub fn validated_embedding(
    configured: &str,
    validated_model: Option<&str>,
    mode: Option<&str>,
) -> Option<EmbeddingDimensions> {
    if configured.is_empty() || validated_model != Some(configured) {
        return None;
    }
    mode.and_then(EmbeddingDimensions::from_pref)
}

/// The fixed, neutral text the probe embeds — never mail content.
const EMBEDDING_PROBE_TEXT: &str = "EmailOps embedding check";

/// What one probe request came back with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeAttempt {
    /// A vector of this many floats.
    Vector(usize),
    /// The request was refused (HTTP 4xx).
    Rejected,
}

/// What the probe does next, or concludes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbePlan {
    Compatible(EmbeddingDimensions),
    RetryWithoutDimensions,
    /// The model's own vectors have this many floats, not the index's.
    WrongDimension(usize),
    /// OpenRouter refused the model with and without `dimensions`.
    Rejected,
}

/// Decide whether an embedding model fits the index from the attempt that
/// asked for [`EMAIL_EMBEDDING_DIM`] via `dimensions` and, once made, the
/// attempt without it. The catalogue publishes no vector size, so asking is
/// the only way to know.
pub fn plan_embedding_probe(with_dimensions: ProbeAttempt, without_dimensions: Option<ProbeAttempt>) -> ProbePlan {
    if with_dimensions == ProbeAttempt::Vector(EMAIL_EMBEDDING_DIM) {
        return ProbePlan::Compatible(EmbeddingDimensions::Requested);
    }
    match without_dimensions {
        None => ProbePlan::RetryWithoutDimensions,
        Some(ProbeAttempt::Vector(EMAIL_EMBEDDING_DIM)) => ProbePlan::Compatible(EmbeddingDimensions::Native),
        Some(ProbeAttempt::Vector(len)) => ProbePlan::WrongDimension(len),
        Some(ProbeAttempt::Rejected) => ProbePlan::Rejected,
    }
}

/// A probe that found the model usable, with what the probe itself cost.
#[derive(Debug)]
pub struct EmbeddingProbe {
    pub dimensions: EmbeddingDimensions,
    pub tokens: u32,
    pub cost_usd: f64,
}

/// A failed embedding request. `rejected` is true for an HTTP 4xx: OpenRouter
/// answered and said no, as opposed to an outage or a broken connection.
struct EmbeddingFailure {
    rejected: bool,
    error: AppError,
}

impl EmbeddingFailure {
    fn other(error: AppError) -> Self {
        Self { rejected: false, error }
    }
}

/// Shown instead of sending a request when no embedding model has passed the
/// probe.
const EMBEDDING_NOT_SET_UP: &str =
    "No OpenRouter embedding model is set up, so semantic search is off — choose one in Settings → AI";

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
    /// `Some` only when `embedding_model` passed the probe; without it no
    /// embedding request is sent.
    embedding_dimensions: Option<EmbeddingDimensions>,
    zero_data_retention: bool,
    base_url: String,
    endpoint: Endpoint,
    stream_idle_timeout: Duration,
    /// How long a streamed reply may take to start; `None` = the idle window.
    first_token_timeout: Option<Duration>,
    /// The context window the user set for this model; `None` = ask the server.
    context_window: Option<u32>,
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
            embedding_dimensions: None,
            zero_data_retention: false,
            base_url: OPENROUTER_BASE_URL.to_string(),
            endpoint: Endpoint::OpenRouter,
            stream_idle_timeout: STREAM_IDLE_TIMEOUT,
            first_token_timeout: None,
            context_window: None,
        }
    }

    /// A client for a user-configured OpenAI-compatible server at `base_url`
    /// (e.g. `http://localhost:1234/v1`). `api_key` may be empty. No
    /// OpenRouter routing object or attribution headers are sent.
    ///
    /// Redirects are never followed: `base_url` passed
    /// `normalize_ai_base_url`, and a redirect could send the mail in the
    /// request on to a host (or a plain-http URL) that check never saw.
    pub fn openai_compatible(base_url: &str, api_key: String, model: String, embedding_model: String) -> Self {
        let mut client = Self::new(api_key, model, embedding_model);
        client.client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|e| {
                crate::services::logger::log(
                    "warn",
                    "ai",
                    format!("could not build the OpenAI-compatible HTTP client ({e}); requests will fail"),
                );
                Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .unwrap_or_default()
            });
        client.base_url = base_url.trim().trim_end_matches('/').to_string();
        client.endpoint = Endpoint::OpenAiCompatible;
        client.first_token_timeout = Some(COMPATIBLE_FIRST_TOKEN_TIMEOUT);
        client
    }

    /// Send requests to `base_url` (a mock server) instead of openrouter.ai.
    #[cfg(test)]
    pub(crate) fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// The server this client talks to.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Name used in errors and logs, so a local server's failure is not
    /// blamed on OpenRouter.
    fn server_name(&self) -> &'static str {
        match self.endpoint {
            Endpoint::OpenRouter => "OpenRouter",
            Endpoint::OpenAiCompatible => "The OpenAI-compatible server",
        }
    }

    /// OpenRouter's data-policy object; `None` for other servers.
    fn provider_preferences(&self) -> Option<ProviderPreferences> {
        (self.endpoint == Endpoint::OpenRouter).then(|| ProviderPreferences::new(self.zero_data_retention))
    }

    /// Attach authentication (when there is a key) and, for OpenRouter only,
    /// its attribution headers.
    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let request = if self.api_key.trim().is_empty() {
            request
        } else {
            request.header("Authorization", format!("Bearer {}", self.api_key))
        };
        match self.endpoint {
            Endpoint::OpenRouter => request
                .header("HTTP-Referer", APP_URL)
                .header("X-OpenRouter-Title", APP_NAME),
            Endpoint::OpenAiCompatible => request,
        }
    }

    /// Use `window` as the model's context window instead of asking the server.
    pub fn with_context_window(mut self, window: Option<u32>) -> Self {
        self.context_window = window.filter(|n| *n > 0);
        self
    }

    /// Route only to providers with a zero-data-retention policy.
    pub fn with_zero_data_retention(mut self, enabled: bool) -> Self {
        self.zero_data_retention = enabled;
        self
    }

    /// Allow embedding with the configured model, in the mode the probe
    /// validated it for (see [`validated_embedding`]).
    pub fn with_embedding_dimensions(mut self, dimensions: Option<EmbeddingDimensions>) -> Self {
        self.embedding_dimensions = dimensions;
        self
    }

    /// Refuse a chat model OpenRouter cannot know (see
    /// [`is_openrouter_model_id`]) with what to do about it, before a request
    /// carrying mail content is sent.
    fn ensure_chat_model(&self) -> Result<()> {
        if self.endpoint == Endpoint::OpenAiCompatible {
            // Any id the server knows; an empty one is the only certain mistake.
            if self.model.trim().is_empty() {
                return Err(AppError::AiError(
                    "No chat model selected — choose one in Settings → AI → OpenAI-compatible".to_string(),
                ));
            }
            return Ok(());
        }
        if is_openrouter_model_id(&self.model) {
            return Ok(());
        }
        Err(AppError::AiError(format!(
            "\"{}\" is not an OpenRouter chat model — enter one (vendor/model) in Settings → AI → OpenRouter",
            self.model
        )))
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
            provider: self.provider_preferences(),
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
            provider: self.provider_preferences(),
        }
    }

    fn embedding_request(&self, text: &str, dimensions: Option<usize>) -> OpenRouterEmbeddingRequest {
        OpenRouterEmbeddingRequest {
            model: self.embedding_model.clone(),
            input: text.to_string(),
            encoding_format: "float".to_string(),
            dimensions,
            provider: self.provider_preferences(),
        }
    }

    /// One `POST /embeddings` for `text`, asking for `dimensions` floats when
    /// given.
    async fn request_embedding(
        &self,
        text: &str,
        dimensions: Option<usize>,
    ) -> std::result::Result<EmbeddingResult, EmbeddingFailure> {
        let url = format!("{}/embeddings", self.base_url);
        let request = self.embedding_request(text, dimensions);

        let response = self
            .authorized(self.client.post(&url))
            .header("Content-Type", "application/json")
            .timeout(GENERATION_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                EmbeddingFailure::other(if e.is_timeout() {
                    AppError::AiError(format!(
                        "{} embedding timed out ({}s)",
                        self.server_name(),
                        GENERATION_TIMEOUT.as_secs()
                    ))
                } else {
                    AppError::AiError(format!("Failed to connect to {} embeddings: {}", self.server_name(), e))
                })
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(EmbeddingFailure {
                rejected: status.is_client_error(),
                error: request_error(
                    status.as_u16(),
                    &error_text,
                    &self.embedding_model,
                    &format!("{} embedding error (HTTP {})", self.server_name(), status.as_u16()),
                ),
            });
        }

        let body: OpenRouterEmbeddingResponse = response.json().await.map_err(|e| {
            EmbeddingFailure::other(AppError::AiError(format!(
                "Failed to parse {} embedding response: {}",
                self.server_name(),
                e
            )))
        })?;

        let embedding = body.data.first().map(|item| item.embedding.clone()).ok_or_else(|| {
            EmbeddingFailure::other(AppError::AiError(format!(
                "{} returned no embedding vector",
                self.server_name()
            )))
        })?;

        Ok(EmbeddingResult {
            embedding,
            tokens: body.usage.as_ref().and_then(|usage| usage.prompt_tokens).unwrap_or(0),
            cost_usd: body.usage.as_ref().and_then(|usage| usage.cost).unwrap_or(0.0),
        })
    }

    /// Find out whether the configured embedding model can fill the email
    /// index, by embedding a fixed neutral string: first asking for
    /// [`EMAIL_EMBEDDING_DIM`] floats via `dimensions`, then — if that is
    /// refused or ignored — once more without it (see
    /// [`plan_embedding_probe`]). An outage is returned as the error it is,
    /// not as a verdict on the model.
    pub async fn probe_embedding(&self) -> Result<EmbeddingProbe> {
        let mut tokens = 0;
        let mut cost_usd = 0.0;
        let mut with_dimensions = None;
        let mut last_rejection = None;
        loop {
            let dimensions = with_dimensions.is_none().then_some(EMAIL_EMBEDDING_DIM);
            let attempt = match self.request_embedding(EMBEDDING_PROBE_TEXT, dimensions).await {
                Ok(result) => {
                    tokens += result.tokens;
                    cost_usd += result.cost_usd;
                    ProbeAttempt::Vector(result.embedding.len())
                }
                Err(failure) if failure.rejected => {
                    last_rejection = Some(failure.error);
                    ProbeAttempt::Rejected
                }
                Err(failure) => return Err(failure.error),
            };
            let plan = match with_dimensions {
                None => plan_embedding_probe(attempt, None),
                Some(first) => plan_embedding_probe(first, Some(attempt)),
            };
            match plan {
                ProbePlan::Compatible(dimensions) => {
                    return Ok(EmbeddingProbe {
                        dimensions,
                        tokens,
                        cost_usd,
                    })
                }
                ProbePlan::RetryWithoutDimensions => with_dimensions = Some(attempt),
                ProbePlan::WrongDimension(len) => {
                    return Err(AppError::InvalidInput(format!(
                        "The embedding model {} returns {len}-dimension vectors; the email index needs \
                         {EMAIL_EMBEDDING_DIM}. Choose a {EMAIL_EMBEDDING_DIM}-dimension embedding model.",
                        self.embedding_model
                    )))
                }
                ProbePlan::Rejected => {
                    return Err(last_rejection.unwrap_or_else(|| {
                        AppError::AiError(format!(
                            "OpenRouter rejected the embedding model {}",
                            self.embedding_model
                        ))
                    }))
                }
            }
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
        self.ensure_chat_model()?;
        let idle = self.stream_idle_timeout;
        let first_token = self.first_token_timeout.unwrap_or(idle);
        let send = self
            .authorized(self.client.post(format!("{}/chat/completions", self.base_url)))
            .header("Content-Type", "application/json")
            .json(&self.stream_request(messages, tools))
            .send();
        let response = tokio::time::timeout(first_token, send)
            .await
            .map_err(|_| stream_stalled(first_token, self.server_name()))?
            .map_err(|e| AppError::AiError(format!("Failed to connect to {}: {e}", self.server_name())))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(stream_request_error(
                status,
                &error_text,
                &self.model,
                self.server_name(),
            ));
        }

        let mut stream = response.bytes_stream();
        let mut lines = SseLines::default();
        let mut reply = StreamAccumulator::default();
        // Many local models write their reasoning inline (`<think>…</think>`):
        // it is held back from the user and dropped from the final message.
        let mut thinking = ThinkingGate::new();
        let mut started = false;
        loop {
            let wait = if started { idle } else { first_token };
            let next = tokio::time::timeout(wait, stream.next())
                .await
                .map_err(|_| stream_stalled(wait, self.server_name()))?;
            let (batch, ended) = match next {
                Some(chunk) => {
                    let bytes = chunk
                        .map_err(|e| AppError::AiError(format!("{} stream read error: {e}", self.server_name())))?;
                    started = true;
                    (lines.push(&bytes), false)
                }
                None => (lines.finish().into_iter().collect(), true),
            };
            for line in batch {
                match parse_sse_line(&line)? {
                    None => {}
                    Some(SseEvent::Done) => return reply.finish().map(without_reasoning),
                    Some(SseEvent::Chunk(chunk)) => {
                        if let Some(prose) = reply.apply(chunk) {
                            let visible = thinking.push(&prose);
                            if !visible.is_empty() && !on_token(visible) {
                                return Ok(without_reasoning(reply.into_partial()));
                            }
                        }
                    }
                }
            }
            if ended {
                return if reply.finished() {
                    reply.finish().map(without_reasoning)
                } else {
                    Err(AppError::AiError(format!(
                        "{} ended the reply before it was complete — the model's provider may have dropped the connection",
                        self.server_name()
                    )))
                };
            }
        }
    }

    async fn list_models_from_api(&self) -> Result<Vec<ModelInfo>> {
        let models = self
            .fetch_model_catalogue()
            .await?
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

    async fn fetch_model_catalogue(&self) -> Result<Vec<OpenRouterModelInfo>> {
        let url = format!("{}/models", self.base_url);
        let response = self
            .authorized(self.client.get(&url))
            .timeout(CONNECT_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to fetch {} models: {}", self.server_name(), e)))?;

        if !response.status().is_success() {
            return Err(AppError::AiError(format!(
                "Failed to list {} models",
                self.server_name()
            )));
        }

        let body: OpenRouterModelsResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse {} models: {}", self.server_name(), e)))?;

        Ok(body.data)
    }

    pub async fn list_embedding_models_from_api(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/embeddings/models", self.base_url);
        let response = self
            .authorized(self.client.get(&url))
            .timeout(CONNECT_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to fetch OpenRouter embedding models: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(request_error(
                status,
                &error_text,
                &self.embedding_model,
                &format!("Failed to list OpenRouter embedding models (HTTP {status})"),
            ));
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
        match self.endpoint {
            Endpoint::OpenRouter => ProviderType::OpenRouter,
            Endpoint::OpenAiCompatible => ProviderType::OpenAiCompatible,
        }
    }

    /// The selected model's window from the catalogue. Asked for on demand
    /// and remembered for the life of the process, so the chat — which sizes
    /// every turn to the window — costs one catalogue request per model, not
    /// one per turn. An unreadable catalogue leaves it unknown (and is asked
    /// again next time); the caller sizes to its safe default.
    async fn resolve_context_window(&self) -> Option<u32> {
        if self.context_window.is_some() {
            return self.context_window;
        }
        let key = (self.base_url.clone(), self.model.clone());
        if let Some(window) = known_windows()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
        {
            return Some(*window);
        }
        match self.fetch_model_catalogue().await {
            Ok(models) => {
                let window = model_context_length(&models, &self.model)?;
                known_windows()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(key, window);
                Some(window)
            }
            Err(e) => {
                crate::services::logger::log(
                    "warn",
                    "ai",
                    format!(
                        "{}: could not read the context window of {}: {e}",
                        self.server_name(),
                        self.model
                    ),
                );
                None
            }
        }
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    async fn is_available(&self) -> bool {
        let url = format!("{}/models", self.base_url);
        self.authorized(self.client.get(&url))
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
        self.ensure_chat_model()?;
        let url = format!("{}/chat/completions", self.base_url);

        let request = self.chat_request(prompt, &options);

        let response = self
            .authorized(self.client.post(&url))
            .header("Content-Type", "application/json")
            .timeout(GENERATION_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::AiError(format!(
                        "{} generation timed out ({}s)",
                        self.server_name(),
                        GENERATION_TIMEOUT.as_secs()
                    ))
                } else {
                    AppError::AiError(format!("Failed to connect to {}: {}", self.server_name(), e))
                }
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(request_error(
                status,
                &error_text,
                &self.model,
                &format!("{} error (HTTP {status})", self.server_name()),
            ));
        }

        let result: OpenRouterChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::AiError(format!("Failed to parse {} response: {}", self.server_name(), e)))?;

        let text = result
            .choices
            .first()
            .map(|c| strip_reasoning(&openrouter_content_to_text(&c.message.content)))
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

    fn embedding_configured(&self) -> bool {
        self.embedding_dimensions.is_some()
    }

    /// False, without asking the network, unless the embedding model passed
    /// the probe.
    async fn is_embedding_available(&self) -> bool {
        self.embedding_configured() && self.is_available().await
    }

    async fn list_embedding_models(&self) -> Result<Vec<ModelInfo>> {
        self.list_embedding_models_from_api().await
    }

    async fn embed(&self, text: &str) -> Result<EmbeddingResult> {
        let Some(dimensions) = self.embedding_dimensions else {
            return Err(AppError::AiError(EMBEDDING_NOT_SET_UP.to_string()));
        };
        self.request_embedding(text, dimensions.request_value())
            .await
            .map_err(|failure| failure.error)
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

    async fn chat_with_tools_metered(
        &self,
        messages: &[AiMessage],
        tools: &[serde_json::Value],
    ) -> Result<ToolStreamResult> {
        self.chat_stream_with_tools(messages.to_vec(), tools.to_vec(), Box::new(|_| true))
            .await
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
/// `outcome` with any inline reasoning span removed from its prose.
fn without_reasoning(mut outcome: StreamOutcome) -> StreamOutcome {
    if !outcome.content.is_empty() {
        outcome.content = strip_reasoning(&outcome.content);
    }
    outcome
}

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
        let body = serde_json::to_value(client(false).embedding_request("hi", None)).unwrap();
        assert_eq!(body["provider"], serde_json::json!({ "data_collection": "deny" }));
        let body = serde_json::to_value(client(true).embedding_request("hi", None)).unwrap();
        assert_eq!(
            body["provider"],
            serde_json::json!({ "data_collection": "deny", "zdr": true })
        );
    }

    fn compatible() -> OpenRouterClient {
        OpenRouterClient::openai_compatible(
            "http://127.0.0.1:8317/v1/",
            String::new(),
            "local-model".into(),
            String::new(),
        )
    }

    #[test]
    fn an_openai_compatible_server_gets_no_openrouter_routing_object() {
        // A plain OpenAI-compatible server does not know `provider` and may
        // reject the request over it.
        let body = serde_json::to_value(compatible().chat_request("hi", &CompletionOptions::default())).unwrap();
        assert!(body.get("provider").is_none(), "{body}");
        let body = serde_json::to_value(compatible().embedding_request("hi", None)).unwrap();
        assert!(body.get("provider").is_none(), "{body}");
        let body = serde_json::to_value(compatible().stream_request(&[], &[])).unwrap();
        assert!(body.get("provider").is_none(), "{body}");
    }

    #[test]
    fn an_openai_compatible_server_takes_any_model_id_but_not_none() {
        assert!(
            compatible().ensure_chat_model().is_ok(),
            "ids are whatever the server lists"
        );
        let none =
            OpenRouterClient::openai_compatible("http://localhost:1/v1", String::new(), "  ".into(), String::new());
        assert!(none.ensure_chat_model().is_err());
        // OpenRouter keeps its vendor/model rule.
        assert!(client(false).ensure_chat_model().is_ok());
        let bad = OpenRouterClient::new("k".into(), "llama3".into(), String::new());
        assert!(bad.ensure_chat_model().is_err());
    }

    #[test]
    fn an_openai_compatible_client_reports_its_own_type_and_trims_the_url() {
        let c = compatible();
        assert_eq!(c.provider_type(), ProviderType::OpenAiCompatible);
        assert_eq!(c.endpoint(), &Endpoint::OpenAiCompatible);
        assert_eq!(c.base_url, "http://127.0.0.1:8317/v1");
        assert_eq!(client(false).provider_type(), ProviderType::OpenRouter);
    }

    #[test]
    fn stream_errors_name_the_server_they_came_from() {
        let msg = |e: AppError| match e {
            AppError::AiError(m) => m,
            other => panic!("{other:?}"),
        };
        let local = msg(stream_request_error(401, "{}", "m", "The OpenAI-compatible server"));
        assert!(
            local.starts_with("The OpenAI-compatible server refused the request: check the API key"),
            "{local}"
        );
        assert!(!local.contains("OpenRouter"));
        let unknown = msg(stream_request_error(404, "{}", "gpt-x", "The OpenAI-compatible server"));
        assert!(unknown.contains("does not know the model \"gpt-x\""), "{unknown}");
        // OpenRouter's data-policy 404 keeps its own meaning.
        let policy = r#"{"error":{"message":"No endpoints found matching your data policy"}}"#;
        assert!(matches!(
            stream_request_error(404, policy, "vendor/model", "OpenRouter"),
            AppError::AiDataPolicy { .. }
        ));
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
            AppError::AiError(msg) if msg == "OpenRouter error: Provider returned error"
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

    /// What an OpenAI-compatible server received: only the parts that differ
    /// from OpenRouter (headers, body) are inspected.
    async fn received(server: &MockServer) -> (reqwest::header::HeaderMap, serde_json::Value) {
        let requests = server.received_requests().await.expect("recording enabled");
        let last = requests.last().expect("a request was sent");
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in last.headers.iter() {
            if let (Ok(n), Ok(v)) = (
                reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
                reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
            ) {
                headers.insert(n, v);
            }
        }
        (headers, serde_json::from_slice(&last.body).unwrap_or_default())
    }

    #[tokio::test]
    async fn an_openai_compatible_server_streams_a_reply_without_openrouter_headers() {
        let server = server_replying(sse(&[
            r#"{"choices":[{"delta":{"content":"Bon"}}]}"#,
            r#"{"choices":[{"delta":{"content":"jour"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]))
        .await;
        let c = OpenRouterClient::openai_compatible(&server.uri(), String::new(), "claude-haiku".into(), String::new());
        let (tokens, on_token) = recording(|_| true);
        let result = c.chat_stream(vec![user("Salut")], on_token).await.expect("streamed");
        assert_eq!(result.content, "Bonjour");
        assert_eq!(seen(&tokens).concat(), "Bonjour");

        let (headers, body) = received(&server).await;
        assert!(
            headers.get("authorization").is_none(),
            "no key, no Authorization header"
        );
        assert!(headers.get("x-openrouter-title").is_none());
        assert!(headers.get("http-referer").is_none());
        assert_eq!(body["model"], "claude-haiku");
        assert!(body.get("provider").is_none());
    }

    #[tokio::test]
    async fn an_openai_compatible_server_gets_the_key_when_one_is_set() {
        let server = server_replying(sse(&[
            r#"{"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]))
        .await;
        let c = OpenRouterClient::openai_compatible(&server.uri(), "secret".into(), "m".into(), String::new());
        let (_tokens, on_token) = recording(|_| true);
        c.chat_stream(vec![user("x")], on_token).await.expect("streamed");
        let (headers, _) = received(&server).await;
        assert_eq!(headers.get("authorization").unwrap(), "Bearer secret");
    }

    #[tokio::test]
    async fn openrouter_still_sends_its_headers_and_routing_object() {
        let server = server_replying(sse(&[
            r#"{"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]))
        .await;
        let (_tokens, on_token) = recording(|_| true);
        client(&server)
            .chat_stream(vec![user("x")], on_token)
            .await
            .expect("streamed");
        let (headers, body) = received(&server).await;
        assert_eq!(headers.get("authorization").unwrap(), "Bearer key");
        assert!(headers.get("x-openrouter-title").is_some());
        assert_eq!(body["provider"], serde_json::json!({ "data_collection": "deny" }));
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

    #[test]
    fn only_a_vendor_slash_model_id_is_an_openrouter_model() {
        for id in ["vendor/model", "vendor/model:free", "vendor/family/model-1.5"] {
            assert!(is_openrouter_model_id(id), "{id}");
        }
        for id in [
            "",
            "  ",
            "local-model-q4_k_m",
            "local-model:latest",
            "/model",
            "vendor/",
            "vendor/ model",
        ] {
            assert!(!is_openrouter_model_id(id), "{id:?}");
        }
    }

    #[tokio::test]
    async fn a_local_model_id_is_refused_before_anything_is_sent() {
        let server = MockServer::start().await;
        let client =
            OpenRouterClient::new("key".into(), "local-model-q4_k_m".into(), String::new()).with_base_url(server.uri());

        let streamed = client.chat_stream(vec![user("Hi")], Box::new(|_| true)).await;
        let completed = client.complete("Hi", CompletionOptions::default()).await;

        for result in [streamed.map(|_| ()), completed.map(|_| ())] {
            assert!(
                matches!(&result, Err(AppError::AiError(msg))
                    if msg.contains("local-model-q4_k_m") && msg.contains("Settings")),
                "{result:?}"
            );
        }
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "no request may leave the machine"
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

#[cfg(test)]
mod context_window_tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn models(json: serde_json::Value) -> Vec<OpenRouterModelInfo> {
        serde_json::from_value::<OpenRouterModelsResponse>(json)
            .expect("models json")
            .data
    }

    fn catalogue() -> serde_json::Value {
        serde_json::json!({ "data": [
            { "id": "vendor/big", "name": "Big", "pricing": {}, "context_length": 200000,
              "top_provider": { "context_length": 128000 } },
            { "id": "vendor/plain", "name": "Plain", "pricing": {}, "context_length": 32000 },
            { "id": "vendor/top-only", "name": "Top", "pricing": {}, "context_length": null,
              "top_provider": { "context_length": 64000 } },
            { "id": "vendor/free-one:free", "name": "Free", "pricing": {}, "context_length": 8000 },
            { "id": "vendor/free-one", "name": "Paid", "pricing": {}, "context_length": 100000 },
            { "id": "vendor/silent", "name": "Silent", "pricing": {} }
        ]})
    }

    #[test]
    fn the_window_is_the_smaller_of_the_model_and_its_top_provider() {
        let models = models(catalogue());
        assert_eq!(model_context_length(&models, "vendor/big"), Some(128_000));
        assert_eq!(model_context_length(&models, "vendor/plain"), Some(32_000));
        assert_eq!(model_context_length(&models, "vendor/top-only"), Some(64_000));
    }

    #[test]
    fn a_model_the_catalogue_says_nothing_about_has_no_window() {
        let models = models(catalogue());
        assert_eq!(model_context_length(&models, "vendor/silent"), None);
        assert_eq!(model_context_length(&models, "vendor/unknown"), None);
    }

    #[test]
    fn a_routing_suffix_falls_back_to_the_base_model_but_a_listed_variant_wins() {
        let models = models(catalogue());
        assert_eq!(model_context_length(&models, "vendor/plain:nitro"), Some(32_000));
        assert_eq!(model_context_length(&models, "vendor/free-one:free"), Some(8_000));
    }

    #[tokio::test]
    async fn the_client_reads_its_models_window_from_the_catalogue() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(catalogue()))
            .expect(1)
            .mount(&server)
            .await;
        let client =
            OpenRouterClient::new("key".into(), "vendor/big".into(), "vendor/embed".into()).with_base_url(server.uri());
        assert_eq!(client.context_window(), None, "not known before it is asked for");
        assert_eq!(client.resolve_context_window().await, Some(128_000));
    }

    #[tokio::test]
    async fn an_openai_compatible_server_lists_models_that_carry_only_an_id() {
        // CLIProxyAPI, LM Studio, vLLM…: `/v1/models` has no name or pricing.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer 123456"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    { "id": "claude-haiku", "object": "model", "owned_by": "anthropic" },
                    { "id": "claude-sonnet", "object": "model" }
                ]
            })))
            .mount(&server)
            .await;
        let client = OpenRouterClient::openai_compatible(
            &format!("{}/v1", server.uri()),
            "123456".into(),
            String::new(),
            String::new(),
        );
        let ids: Vec<String> = client.list_models().await.unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["claude-haiku", "claude-sonnet"]);
    }

    #[tokio::test]
    async fn a_window_already_read_is_not_asked_for_again() {
        // The chat sizes every turn to the window, and builds a new client
        // per turn: the catalogue is fetched once per model, not once per turn.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(catalogue()))
            .expect(1)
            .mount(&server)
            .await;
        for _ in 0..2 {
            let client = OpenRouterClient::new("key".into(), "vendor/plain".into(), "vendor/embed".into())
                .with_base_url(server.uri());
            assert_eq!(client.resolve_context_window().await, Some(32_000));
        }
    }

    #[tokio::test]
    async fn a_catalogue_that_cannot_be_read_leaves_the_window_unknown() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client =
            OpenRouterClient::new("key".into(), "vendor/big".into(), "vendor/embed".into()).with_base_url(server.uri());
        assert_eq!(client.resolve_context_window().await, None);
    }
}

#[cfg(test)]
mod embedding_tests {
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const DIM: usize = EMAIL_EMBEDDING_DIM;

    fn client(server: &MockServer) -> OpenRouterClient {
        OpenRouterClient::new("key".into(), "vendor/model".into(), "vendor/embed".into()).with_base_url(server.uri())
    }

    fn vector_reply(len: usize) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "embedding": vec![0.5_f32; len] }],
            "usage": { "prompt_tokens": 4, "total_tokens": 4, "cost": 0.000_002 }
        }))
    }

    async fn embedding_bodies(server: &MockServer) -> Vec<serde_json::Value> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect()
    }

    #[test]
    fn the_probe_plan_follows_the_two_attempts() {
        use EmbeddingDimensions::{Native, Requested};
        use ProbeAttempt::{Rejected, Vector};
        let cases = [
            (Vector(DIM), None, ProbePlan::Compatible(Requested)),
            (Vector(1536), None, ProbePlan::RetryWithoutDimensions),
            (Rejected, None, ProbePlan::RetryWithoutDimensions),
            (Rejected, Some(Vector(DIM)), ProbePlan::Compatible(Native)),
            (Vector(1536), Some(Vector(DIM)), ProbePlan::Compatible(Native)),
            (Rejected, Some(Vector(1024)), ProbePlan::WrongDimension(1024)),
            (Vector(256), Some(Vector(1536)), ProbePlan::WrongDimension(1536)),
            (Rejected, Some(Rejected), ProbePlan::Rejected),
            (Vector(1536), Some(Rejected), ProbePlan::Rejected),
        ];
        for (with_dimensions, without, expected) in cases {
            assert_eq!(
                plan_embedding_probe(with_dimensions, without),
                expected,
                "{with_dimensions:?} then {without:?}"
            );
        }
    }

    #[test]
    fn only_the_model_that_passed_the_probe_is_usable() {
        use EmbeddingDimensions::{Native, Requested};
        let cases = [
            ("vendor/embed", Some("vendor/embed"), Some("requested"), Some(Requested)),
            ("vendor/embed", Some("vendor/embed"), Some("native"), Some(Native)),
            // A local GGUF id left over from another provider, never probed.
            ("local-embed-q4_k_m", None, None, None),
            ("vendor/other", Some("vendor/embed"), Some("requested"), None),
            ("", Some(""), Some("native"), None),
            ("vendor/embed", Some("vendor/embed"), None, None),
            ("vendor/embed", Some("vendor/embed"), Some("garbage"), None),
        ];
        for (configured, validated, mode, expected) in cases {
            assert_eq!(
                validated_embedding(configured, validated, mode),
                expected,
                "{configured:?} / {validated:?} / {mode:?}"
            );
        }
    }

    #[test]
    fn the_request_carries_dimensions_only_when_asked() {
        let client = OpenRouterClient::new("key".into(), "vendor/model".into(), "vendor/embed".into());
        let body = serde_json::to_value(client.embedding_request("hi", Some(DIM))).unwrap();
        assert_eq!(body["dimensions"], DIM);
        let body = serde_json::to_value(client.embedding_request("hi", None)).unwrap();
        assert!(body.get("dimensions").is_none());
    }

    #[tokio::test]
    async fn a_model_that_honours_dimensions_is_compatible_via_the_parameter() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .and(body_partial_json(json!({ "dimensions": DIM })))
            .respond_with(vector_reply(DIM))
            .mount(&server)
            .await;

        let probe = client(&server).probe_embedding().await.unwrap();

        assert_eq!(probe.dimensions, EmbeddingDimensions::Requested);
        assert_eq!(probe.tokens, 4);
        assert!((probe.cost_usd - 0.000_002).abs() < 1e-12);
        let bodies = embedding_bodies(&server).await;
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["model"], "vendor/embed");
        assert_eq!(bodies[0]["input"], EMBEDDING_PROBE_TEXT);
    }

    #[tokio::test]
    async fn a_model_that_rejects_dimensions_but_is_natively_the_right_size_is_compatible() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .and(body_partial_json(json!({ "dimensions": DIM })))
            .respond_with(
                ResponseTemplate::new(400).set_body_string(r#"{"error":{"message":"dimensions not supported"}}"#),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(vector_reply(DIM))
            .mount(&server)
            .await;

        let probe = client(&server).probe_embedding().await.unwrap();

        assert_eq!(probe.dimensions, EmbeddingDimensions::Native);
        let bodies = embedding_bodies(&server).await;
        assert_eq!(bodies.len(), 2);
        assert!(bodies[1].get("dimensions").is_none());
    }

    #[tokio::test]
    async fn a_model_of_another_size_is_refused_with_the_size_it_returned() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(vector_reply(1536))
            .mount(&server)
            .await;

        let err = client(&server).probe_embedding().await.unwrap_err();

        match err {
            AppError::InvalidInput(msg) => {
                assert!(
                    msg.contains("1536") && msg.contains("768") && msg.contains("vendor/embed"),
                    "{msg}"
                );
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
        assert_eq!(embedding_bodies(&server).await.len(), 2, "one retry, no more");
    }

    #[tokio::test]
    async fn an_unknown_model_is_refused_with_the_providers_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"error":{"message":"No such model"}}"#))
            .mount(&server)
            .await;

        let err = client(&server).probe_embedding().await.unwrap_err();

        assert!(
            matches!(&err, AppError::AiError(msg) if msg.contains("No such model")),
            "{err:?}"
        );
        assert_eq!(embedding_bodies(&server).await.len(), 2);
    }

    #[tokio::test]
    async fn a_model_no_zero_retention_provider_serves_is_a_data_policy_refusal() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .and(body_partial_json(json!({ "provider": { "zdr": true } })))
            .respond_with(ResponseTemplate::new(404).set_body_string(
                r#"{"error":{"message":"No endpoints found matching your data policy (Zero data retention)","code":404}}"#,
            ))
            .mount(&server)
            .await;

        let err = client(&server)
            .with_zero_data_retention(true)
            .probe_embedding()
            .await
            .unwrap_err();

        assert!(
            matches!(&err, AppError::AiDataPolicy { model } if model == "vendor/embed"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn an_outage_is_an_error_not_a_verdict_and_is_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
            .mount(&server)
            .await;

        let err = client(&server).probe_embedding().await.unwrap_err();

        assert!(matches!(err, AppError::AiError(_)), "{err:?}");
        assert_eq!(embedding_bodies(&server).await.len(), 1);
    }

    #[tokio::test]
    async fn without_a_validated_model_nothing_is_sent() {
        let server = MockServer::start().await;
        let client = client(&server);

        assert!(!client.embedding_configured());
        assert!(!client.is_embedding_available().await);
        let err = client.embed("mail text").await.unwrap_err();
        assert!(
            matches!(&err, AppError::AiError(msg) if msg.contains("Settings")),
            "{err:?}"
        );
        assert!(client.embed_batch(&["a".to_string()]).await.is_err());

        assert!(
            embedding_bodies(&server).await.is_empty(),
            "no request may leave the machine"
        );
    }

    #[tokio::test]
    async fn a_validated_model_embeds_with_or_without_dimensions_and_reports_its_cost() {
        for (mode, sends) in [
            (EmbeddingDimensions::Requested, true),
            (EmbeddingDimensions::Native, false),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/embeddings"))
                .respond_with(vector_reply(DIM))
                .mount(&server)
                .await;
            let client = client(&server).with_embedding_dimensions(Some(mode));
            assert!(client.embedding_configured());

            let result = client.embed("mail text").await.unwrap();

            assert_eq!(result.embedding.len(), DIM);
            assert_eq!(result.tokens, 4);
            assert!((result.cost_usd - 0.000_002).abs() < 1e-12);
            let bodies = embedding_bodies(&server).await;
            assert_eq!(bodies[0].get("dimensions").is_some(), sends, "{mode:?}");
            assert_eq!(bodies[0]["provider"]["data_collection"], "deny");
        }
    }
}

#[cfg(test)]
mod compatible_server_tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn sse(events: &[&str]) -> String {
        events.iter().map(|event| format!("data: {event}\n\n")).collect()
    }

    fn compatible(server: &MockServer) -> OpenRouterClient {
        OpenRouterClient::openai_compatible(&server.uri(), String::new(), "local-model".into(), "local-embed".into())
    }

    fn user(content: &str) -> AiMessage {
        AiMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_calls: None,
        }
    }

    type Tokens = Arc<Mutex<Vec<String>>>;

    fn recording() -> (Tokens, Box<dyn FnMut(String) -> bool + Send>) {
        let tokens: Tokens = Arc::default();
        let sink = tokens.clone();
        let callback = Box::new(move |token: String| {
            sink.lock().unwrap_or_else(PoisonError::into_inner).push(token);
            true
        });
        (tokens, callback)
    }

    async fn chat_replying(template: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(template)
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed() {
        let elsewhere = chat_replying(ResponseTemplate::new(200).set_body_raw(
            sse(&[
                r#"{"choices":[{"delta":{"content":"leaked"},"finish_reason":"stop"}]}"#,
                "[DONE]",
            ]),
            "text/event-stream",
        ))
        .await;
        let server = chat_replying(
            ResponseTemplate::new(307).insert_header("Location", format!("{}/chat/completions", elsewhere.uri())),
        )
        .await;

        let result = compatible(&server)
            .chat_stream(vec![user("mail")], Box::new(|_| true))
            .await;

        assert!(result.is_err(), "{result:?}");
        let followed = elsewhere.received_requests().await.expect("recording").len();
        assert_eq!(followed, 0, "the redirect target must never receive the mail");
    }

    #[tokio::test]
    async fn an_html_error_page_never_reaches_the_message() {
        let server = chat_replying(ResponseTemplate::new(500).set_body_raw(
            "<html><body>Traceback (most recent call last): /srv/app/secret.py</body></html>",
            "text/html",
        ))
        .await;

        let err = compatible(&server)
            .chat_stream(vec![user("x")], Box::new(|_| true))
            .await
            .expect_err("500");
        let msg = err.to_string();

        assert!(msg.contains("HTTP 500"), "{msg}");
        assert!(!msg.contains("<html"), "{msg}");
        assert!(!msg.contains("Traceback"), "{msg}");
    }

    #[tokio::test]
    async fn a_json_error_keeps_only_its_trimmed_message() {
        let long = "x".repeat(2_000);
        let server = chat_replying(ResponseTemplate::new(400).set_body_json(json!({
            "error": { "message": format!("model is not loaded {long}"), "type": "invalid_request_error" }
        })))
        .await;

        let err = compatible(&server)
            .complete("x", CompletionOptions::default())
            .await
            .expect_err("400");
        let msg = err.to_string();

        assert!(msg.contains("model is not loaded"), "{msg}");
        assert!(!msg.contains("invalid_request_error"), "{msg}");
        assert!(msg.len() < 400, "trimmed, got {} chars", msg.len());
    }

    #[tokio::test]
    async fn inline_reasoning_is_dropped_from_the_stream() {
        let server = chat_replying(ResponseTemplate::new(200).set_body_raw(
            sse(&[
                r#"{"choices":[{"delta":{"content":"<thi"}}]}"#,
                r#"{"choices":[{"delta":{"content":"nk>planning the answer</think>"}}]}"#,
                r#"{"choices":[{"delta":{"content":"Two emails."},"finish_reason":"stop"}]}"#,
                "[DONE]",
            ]),
            "text/event-stream",
        ))
        .await;
        let (tokens, on_token) = recording();

        let result = compatible(&server)
            .chat_stream(vec![user("x")], on_token)
            .await
            .expect("streamed");

        assert_eq!(result.content, "Two emails.");
        let shown = tokens.lock().unwrap_or_else(PoisonError::into_inner).concat();
        assert_eq!(shown, "Two emails.");
    }

    #[tokio::test]
    async fn inline_reasoning_is_dropped_from_a_completion() {
        let server = chat_replying(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "<think>hmm</think>OK" }, "finish_reason": "stop" }]
        })))
        .await;

        let result = compatible(&server)
            .complete("x", CompletionOptions::default())
            .await
            .expect("completed");

        assert_eq!(result.text, "OK");
    }

    #[tokio::test]
    async fn the_first_token_may_take_longer_than_the_idle_window() {
        // A local server prefilling a long prompt is silent until the first
        // token; the idle window only applies once the reply has started.
        let server = chat_replying(
            ResponseTemplate::new(200)
                .set_body_raw(
                    sse(&[
                        r#"{"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
                        "[DONE]",
                    ]),
                    "text/event-stream",
                )
                .set_delay(Duration::from_millis(300)),
        )
        .await;
        let client = OpenRouterClient {
            stream_idle_timeout: Duration::from_millis(50),
            ..compatible(&server)
        };

        let result = client.chat_stream(vec![user("x")], Box::new(|_| true)).await;

        assert_eq!(result.expect("prefill is not a stall").content, "ok");
    }

    #[tokio::test]
    async fn the_context_window_comes_from_max_model_len() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": "local-model", "max_model_len": 32768 }]
            })))
            .mount(&server)
            .await;

        assert_eq!(compatible(&server).resolve_context_window().await, Some(32768));
    }

    #[tokio::test]
    async fn a_configured_context_window_wins_over_the_server() {
        let server = MockServer::start().await;
        let client = compatible(&server).with_context_window(Some(16_384));

        assert_eq!(client.resolve_context_window().await, Some(16_384));
        assert!(server.received_requests().await.expect("recording").is_empty());
    }

    #[tokio::test]
    async fn tool_calls_work_against_a_compatible_server() {
        let server = chat_replying(ResponseTemplate::new(200).set_body_raw(
            sse(&[
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"search_emails","arguments":"{\"query\":"}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"invoice\"}"}}]}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
                "[DONE]",
            ]),
            "text/event-stream",
        ))
        .await;
        let tool = json!({"type": "function", "function": {"name": "search_emails", "parameters": {"type": "object"}}});

        let result = compatible(&server)
            .chat_stream_with_tools(vec![user("Find the invoice")], vec![tool.clone()], Box::new(|_| true))
            .await
            .expect("tool round");

        let calls = result.message.tool_calls.expect("a tool call");
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(calls[0].function.arguments, json!({"query": "invoice"}));
        let requests = server.received_requests().await.expect("recording");
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("json");
        assert_eq!(body["tools"], json!([tool]));
        assert!(body.get("provider").is_none());
    }

    #[tokio::test]
    async fn the_embedding_probe_accepts_an_index_sized_vector() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "embedding": vec![0.1_f32; EMAIL_EMBEDDING_DIM] }]
            })))
            .mount(&server)
            .await;

        let probe = compatible(&server).probe_embedding().await.expect("compatible");

        assert_eq!(probe.dimensions, EmbeddingDimensions::Requested);
    }

    #[tokio::test]
    async fn the_embedding_probe_refuses_a_model_without_vectors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
            .mount(&server)
            .await;

        assert!(compatible(&server).probe_embedding().await.is_err());
    }
}
