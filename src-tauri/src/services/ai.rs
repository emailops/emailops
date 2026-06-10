use std::sync::Arc;

/// Default idle window before a loaded local model is evicted from RAM.
/// Overridable via the `chat.keep_alive_seconds` preference.
const DEFAULT_KEEP_ALIVE_SECS: u32 = 30 * 60;

use crate::ai::ollama::OllamaClient;
use crate::ai::openrouter::OpenRouterClient;
use crate::ai::provider::{AIProvider, CompletionOptions, CompletionResult, ModelInfo};
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{AiConfig, AiLogEvent, AiUsageSummary};

const KEYRING_SERVICE: &str = "emailops";
const OPENROUTER_KEY_ID: &str = "openrouter_api_key";
const OPENROUTER_DEV_KEY_PREF: &str = "openrouter_api_key_dev";

pub struct AiService {
    provider: Arc<dyn AIProvider>,
    db: Arc<Database>,
}

// ── Cached llama.cpp runtime ─────────────────────────────────────────────────
//
// Historically `load_provider` constructed a fresh `LlamaCppRuntime` on every
// call, which meant the GGUF (2–4 GB for chat models) was reloaded from disk
// at the start of every chat turn — a 3–6 s tax on the first token of every
// message. The cache below keys a single `Arc<LlamaCppRuntime>` by the exact
// (chat_path, embed_path) tuple so that as long as the user doesn't swap
// models the runtime — and therefore the loaded weights — is reused.
//
// When the user changes models (chat or embedding) the cached entry is
// replaced and the previous runtime is dropped; its `spawn_eviction_task`
// weak-ref exits cleanly on the next poll.
#[cfg(feature = "llamacpp")]
struct CachedLlamaCppRuntime {
    chat_path: Option<std::path::PathBuf>,
    embed_path: Option<std::path::PathBuf>,
    runtime: Arc<crate::ai::llama_cpp::runtime::LlamaCppRuntime>,
}

#[cfg(feature = "llamacpp")]
static LLAMACPP_RUNTIME_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<CachedLlamaCppRuntime>>> =
    std::sync::OnceLock::new();

#[cfg(feature = "llamacpp")]
fn get_or_create_llamacpp_runtime(
    chat_path: Option<std::path::PathBuf>,
    embed_path: Option<std::path::PathBuf>,
    keep_alive_secs: u32,
) -> Arc<crate::ai::llama_cpp::runtime::LlamaCppRuntime> {
    use crate::ai::llama_cpp::runtime::LlamaCppRuntime;

    let cache = LLAMACPP_RUNTIME_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    // Reuse when the paths match exactly. A `None` path on either side means
    // the user hasn't configured that model yet; we still treat matching
    // `None`s as a cache hit so repeated calls before the user picks a
    // model don't thrash.
    if let Some(existing) = guard.as_ref() {
        if existing.chat_path == chat_path && existing.embed_path == embed_path {
            existing.runtime.set_keep_alive_secs(keep_alive_secs);
            return Arc::clone(&existing.runtime);
        }
    }

    let runtime = LlamaCppRuntime::new(chat_path.clone(), embed_path.clone());
    runtime.set_keep_alive_secs(keep_alive_secs);
    *guard = Some(CachedLlamaCppRuntime {
        chat_path,
        embed_path,
        runtime: Arc::clone(&runtime),
    });
    runtime
}

/// Read the `chat.keep_alive_seconds` preference (default 30 min). Values
/// below 60 s are clamped up to avoid accidentally disabling cache reuse
/// with a mis-typed preference.
pub fn load_keep_alive_secs(db: &Database) -> u32 {
    let raw = db
        .get_preference("chat.keep_alive_seconds")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(DEFAULT_KEEP_ALIVE_SECS);
    if raw == 0 {
        0 // explicit 0 → disable eviction (pin forever)
    } else {
        raw.max(60)
    }
}

/// Format `keep_alive_secs` for Ollama's `keep_alive` field. Ollama accepts
/// "30m", "1h", "-1" (forever), "0" (unload immediately).
fn format_ollama_keep_alive(secs: u32) -> String {
    if secs == 0 {
        "-1".to_string() // 0 in our pref = pin forever; matches llama.cpp path
    } else if secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

impl AiService {
    fn env_openrouter_api_key() -> Option<String> {
        std::env::var("OPENROUTER_API_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn use_dev_ai_keys() -> bool {
        cfg!(debug_assertions)
    }

    pub fn has_openrouter_api_key(db: &Database) -> Result<bool> {
        if Self::env_openrouter_api_key().is_some() {
            return Ok(true);
        }
        if Self::use_dev_ai_keys() {
            return Ok(db.get_preference(OPENROUTER_DEV_KEY_PREF)?.is_some());
        }
        Ok(db.get_preference("openrouter_api_key_id")?.is_some())
    }

    pub fn load_openrouter_api_key(db: &Database) -> Result<String> {
        if let Some(key) = Self::env_openrouter_api_key() {
            return Ok(key);
        }
        if Self::use_dev_ai_keys() {
            return db
                .get_preference(OPENROUTER_DEV_KEY_PREF)?
                .ok_or_else(|| AppError::AiError("OpenRouter API key not configured".to_string()));
        }

        let api_key_id = db
            .get_preference("openrouter_api_key_id")?
            .ok_or_else(|| AppError::AiError("OpenRouter API key not configured".to_string()))?;
        super::keychain::current()
            .get_password(KEYRING_SERVICE, &api_key_id)?
            .ok_or_else(|| AppError::AiError("OpenRouter API key not configured".to_string()))
    }

    pub fn store_openrouter_api_key(db: &Database, key: &str) -> Result<()> {
        if Self::env_openrouter_api_key().is_some() {
            return Ok(());
        }
        if Self::use_dev_ai_keys() {
            db.set_preference(OPENROUTER_DEV_KEY_PREF, key)?;
            db.set_preference("openrouter_api_key_id", OPENROUTER_KEY_ID)?;
            return Ok(());
        }

        super::keychain::current().set_password(KEYRING_SERVICE, OPENROUTER_KEY_ID, key)?;
        db.set_preference("openrouter_api_key_id", OPENROUTER_KEY_ID)?;
        Ok(())
    }

    pub fn new(db: Arc<Database>) -> Result<Self> {
        let provider = Self::load_provider(&db)?;
        Ok(Self { provider, db })
    }

    /// Build an `AiService` around an already-constructed provider. Used by
    /// eval harnesses that want to exercise the extraction pipeline with a
    /// specific embedded model without mutating the user's prefs.
    pub fn with_provider(db: Arc<Database>, provider: Arc<dyn AIProvider>) -> Self {
        Self { provider, db }
    }

    pub fn provider(&self) -> &dyn AIProvider {
        self.provider.as_ref()
    }

    pub async fn reload_provider(&mut self) -> Result<()> {
        self.provider = Self::load_provider(&self.db)?;
        Ok(())
    }

    /// Build a provider with a custom provider name and model (e.g., for classification).
    pub fn build_provider(db: &Database, provider_name: &str, model: &str) -> Result<Arc<dyn AIProvider>> {
        let keep_alive_secs = load_keep_alive_secs(db);
        let ollama_keep_alive = format_ollama_keep_alive(keep_alive_secs);
        match provider_name {
            "ollama" => Ok(Arc::new(
                OllamaClient::new_with_models(Some(model), None).with_keep_alive(ollama_keep_alive),
            )),
            "openrouter" => {
                let key = Self::load_openrouter_api_key(db)?;
                Ok(Arc::new(OpenRouterClient::new(
                    key,
                    model.to_string(),
                    "nomic-embed-text".to_string(),
                )))
            }
            #[cfg(feature = "llamacpp")]
            "llamacpp" => {
                use crate::ai::llama_cpp::LlamaCppBackend;
                let embedding_model = db
                    .get_preference("ai_embedding_model")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let (chat_path, embed_path) = llamacpp_model_paths(db, model, &embedding_model);
                // Reuse the cached runtime when (chat_path, embed_path) match
                // an earlier load — avoids reloading the multi-GB GGUF every
                // turn. See CachedLlamaCppRuntime above.
                let runtime = get_or_create_llamacpp_runtime(chat_path, embed_path, keep_alive_secs);
                Ok(Arc::new(LlamaCppBackend::new(
                    runtime,
                    model.to_string(),
                    embedding_model,
                )))
            }
            _ => Ok(Arc::new(
                OllamaClient::new_with_models(Some(model), None).with_keep_alive(ollama_keep_alive),
            )),
        }
    }

    pub fn load_provider(db: &Database) -> Result<Arc<dyn AIProvider>> {
        let config = Self::get_config(db)?;
        let keep_alive_secs = load_keep_alive_secs(db);
        let ollama_keep_alive = format_ollama_keep_alive(keep_alive_secs);

        match config.provider.as_str() {
            "ollama" => Ok(Arc::new(
                OllamaClient::new_with_models(Some(&config.model), Some(&config.embedding_model))
                    .with_keep_alive(ollama_keep_alive),
            )),
            "openrouter" => {
                let key = Self::load_openrouter_api_key(db)?;
                Ok(Arc::new(OpenRouterClient::new(
                    key,
                    config.model,
                    config.embedding_model,
                )))
            }
            #[cfg(feature = "llamacpp")]
            "llamacpp" => {
                use crate::ai::llama_cpp::LlamaCppBackend;
                let (chat_path, embed_path) = llamacpp_model_paths(db, &config.model, &config.embedding_model);
                let runtime = get_or_create_llamacpp_runtime(chat_path, embed_path, keep_alive_secs);
                Ok(Arc::new(LlamaCppBackend::new(
                    runtime,
                    config.model,
                    config.embedding_model,
                )))
            }
            _ => Ok(Arc::new(
                OllamaClient::new_with_models(Some(&config.model), Some(&config.embedding_model))
                    .with_keep_alive(ollama_keep_alive),
            )),
        }
    }

    /// Fire a tiny request to force the local model into RAM. Intended to be
    /// called once at app startup so the first chat turn doesn't pay the full
    /// cold-load cost. Never returns an error — failures are logged but don't
    /// block the caller.
    pub async fn warmup_from_db(db: &Database) {
        fn log(level: &str, message: &str) {
            crate::services::logger::log(level, "ai", message);
        }

        let provider = match Self::load_provider(db) {
            Ok(p) => p,
            Err(e) => {
                log("warn", &format!("AI warmup skipped: provider unavailable ({e})"));
                return;
            }
        };

        let model = provider.model_name().to_string();
        log("info", &format!("Warming up AI model ({})…", model));
        let started = std::time::Instant::now();
        match provider.warmup().await {
            Ok(()) => log(
                "success",
                &format!("AI model warmed up ({}) in {}ms", model, started.elapsed().as_millis()),
            ),
            Err(e) => log("warn", &format!("AI warmup failed ({}): {}", model, e)),
        }
    }

    pub fn get_config(db: &Database) -> Result<AiConfig> {
        // Default to the embedded llama.cpp runtime so fresh installs don't
        // require a separate Ollama process. The recommended chat / embedding
        // model IDs match `ai/model_catalog.rs` so the model manager can
        // resolve them from the curated catalog.
        let provider = db
            .get_preference("ai_provider")?
            .unwrap_or_else(|| "llamacpp".to_string());
        let model = db
            .get_preference("ai_model")?
            .unwrap_or_else(|| "qwen3.5-4b-q4_k_m".to_string());
        let embedding_model = db
            .get_preference("ai_embedding_model")?
            .unwrap_or_else(|| "nomic-embed-text-v1.5-q4_k_m".to_string());
        let api_key_id = db.get_preference("openrouter_api_key_id")?;
        let budget_str = db
            .get_preference("ai_monthly_budget")?
            .unwrap_or_else(|| "0.0".to_string());
        let budget = budget_str.parse::<f64>().unwrap_or_else(|e| {
            crate::services::logger::log(
                "debug",
                "ai",
                format!("malformed ai_monthly_budget pref ({budget_str:?}): {e}; defaulting to 0.0"),
            );
            0.0
        });
        let period_start_str = db.get_preference("ai_period_start")?.unwrap_or_else(|| "0".to_string());
        let period_start = period_start_str.parse::<i64>().unwrap_or_else(|e| {
            crate::services::logger::log(
                "debug",
                "ai",
                format!("malformed ai_period_start pref ({period_start_str:?}): {e}; defaulting to 0"),
            );
            0
        });
        let thinking_enabled = db
            .get_preference("ai_thinking_enabled")?
            .map(|v| v == "true")
            .unwrap_or(false);

        Ok(AiConfig {
            provider,
            model,
            embedding_model,
            api_key_id,
            monthly_budget_usd: budget,
            period_start,
            thinking_enabled,
        })
    }

    pub fn save_config(
        db: &Database,
        provider: &str,
        model: &str,
        embedding_model: Option<&str>,
        api_key: Option<&str>,
        monthly_budget_usd: f64,
        thinking_enabled: Option<bool>,
    ) -> Result<()> {
        db.set_preference("ai_provider", provider)?;
        db.set_preference("ai_model", model)?;
        if let Some(embed_model) = embedding_model {
            db.set_preference("ai_embedding_model", embed_model)?;
        }
        db.set_preference("ai_monthly_budget", &monthly_budget_usd.to_string())?;
        if let Some(thinking) = thinking_enabled {
            db.set_preference("ai_thinking_enabled", if thinking { "true" } else { "false" })?;
        }

        let now = chrono::Utc::now().timestamp();
        db.set_preference("ai_period_start", &now.to_string())?;

        if let Some(key) = api_key {
            // Route the secret to the right backing store based on the
            // provider being saved. Previously this unconditionally wrote
            // to the OpenRouter slot regardless of provider.
            match provider {
                "openrouter" => Self::store_openrouter_api_key(db, key)?,
                _ => {
                    // No-op: ollama / llamacpp don't take a key. Silently
                    // ignore so a stray key doesn't get stored under the
                    // wrong provider.
                }
            }
        }

        Ok(())
    }

    fn check_budget(&self, additional_cost: f64) -> Result<()> {
        let config = Self::get_config(&self.db)?;
        if config.monthly_budget_usd <= 0.0 {
            return Ok(());
        }

        let spent = self.get_usage_since(config.period_start)?;
        let total = spent.total_cost_usd + additional_cost;

        if total > config.monthly_budget_usd {
            Err(AppError::BudgetExceeded(format!(
                "AI budget exceeded: ${:.4} spent + ${:.4} would exceed ${:.2} budget",
                spent.total_cost_usd, additional_cost, config.monthly_budget_usd
            )))
        } else {
            Ok(())
        }
    }

    pub fn get_usage_since(&self, period_start: i64) -> Result<AiUsageSummary> {
        let conn = self.db.connection();
        let mut stmt = conn.prepare(
            "SELECT COALESCE(SUM(cost_usd), 0.0), COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0), COUNT(*)
             FROM ai_usage WHERE timestamp >= ?1"
        )?;
        let mut row = stmt.query(rusqlite::params![period_start])?;
        if let Some(row) = row.next()? {
            Ok(AiUsageSummary {
                total_cost_usd: row.get(0)?,
                total_prompt_tokens: row.get(1)?,
                total_completion_tokens: row.get(2)?,
                total_calls: row.get(3)?,
                period_start,
                budget_usd: Self::get_config(&self.db)?.monthly_budget_usd,
            })
        } else {
            Ok(AiUsageSummary {
                total_cost_usd: 0.0,
                total_prompt_tokens: 0,
                total_completion_tokens: 0,
                total_calls: 0,
                period_start,
                budget_usd: Self::get_config(&self.db)?.monthly_budget_usd,
            })
        }
    }

    pub fn get_current_usage(&self) -> Result<AiUsageSummary> {
        let config = Self::get_config(&self.db)?;
        self.get_usage_since(config.period_start)
    }

    pub fn reset_usage(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.db.set_preference("ai_period_start", &now.to_string())?;
        Ok(())
    }

    fn record_usage(&self, result: &CompletionResult, operation: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.db.connection();
        conn.execute(
            "INSERT INTO ai_usage (provider, model, operation, prompt_tokens, completion_tokens, cost_usd, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                self.provider.provider_type().to_string(),
                result.model,
                operation,
                result.prompt_tokens,
                result.completion_tokens,
                result.cost_usd,
                now,
            ],
        )?;

        self.emit_ai_log(&AiLogEvent {
            provider: self.provider.provider_type().to_string(),
            model: result.model.clone(),
            operation: operation.to_string(),
            prompt_tokens: result.prompt_tokens,
            completion_tokens: result.completion_tokens,
            cost_usd: result.cost_usd,
            status: "ok".to_string(),
            timestamp: now,
        });

        Ok(())
    }

    fn emit_ai_log(&self, event: &AiLogEvent) {
        crate::services::events::emit("ai_log", event);
    }

    pub async fn complete(&self, prompt: &str, operation: &str, options: Option<CompletionOptions>) -> Result<String> {
        let mut opts = options.unwrap_or_default();
        // Apply thinking preference from config if not explicitly set
        if opts.think.is_none() {
            let config = Self::get_config(&self.db)?;
            if !config.thinking_enabled {
                opts.think = Some(false);
            }
        }
        let t = std::time::Instant::now();
        let result = self.provider.complete(prompt, opts).await?;
        let latency_ms = t.elapsed().as_millis() as u64;
        self.check_budget(result.cost_usd)?;
        self.record_usage(&result, operation)?;
        crate::ai::tracing::driver().record_generation(crate::ai::tracing::GenerationParams {
            trace_name: operation,
            name: operation,
            model: &result.model,
            input: prompt,
            output: &result.text,
            prompt_tokens: result.prompt_tokens,
            completion_tokens: result.completion_tokens,
            latency_ms,
            error: None,
        });
        Ok(result.text)
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let result = self.provider.embed(text).await?;
        self.check_budget(result.cost_usd)?;
        Ok(result.embedding)
    }

    pub async fn is_available(&self) -> bool {
        self.provider.is_available().await
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.provider.list_models().await
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Reject AI provider base URLs that aren't plain `http://` or `https://`.
///
/// Anything else (`file:`, `javascript:`, `data:`, `gopher:`, custom schemes …)
/// would either point the AI HTTP client at the local filesystem or open up
/// SSRF-style pivots through another protocol handler. We don't try to be
/// clever about private/loopback IPs here because the supported vllm/Ollama
/// deployment is *meant* to run on `127.0.0.1` / `localhost`; the rule we
/// actually want to enforce is "must be an HTTP(S) URL with a host".
pub fn validate_ai_base_url(raw: &str) -> Result<()> {
    let parsed = url::Url::parse(raw).map_err(|e| {
        AppError::AiError(format!(
            "Invalid AI base URL '{raw}': {e}. Expected an http(s) URL such as http://localhost:8080."
        ))
    })?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(AppError::AiError(format!(
                "AI base URL '{raw}' uses unsupported scheme '{other}'. Only http and https are allowed."
            )));
        }
    }

    if parsed.host_str().is_none_or(|h| h.is_empty()) {
        return Err(AppError::AiError(format!("AI base URL '{raw}' has no host component.")));
    }

    Ok(())
}

/// Compute `(chat_model_path, embed_model_path)` for the llamacpp backend.
///
/// Reads the `app_data_dir` preference that is written at startup so that
/// `load_provider` / `build_provider` — which only have `&Database` — can
/// resolve the on-disk GGUF paths without an `AppState` reference.
#[cfg(feature = "llamacpp")]
fn llamacpp_model_paths(
    db: &Database,
    chat_model_id: &str,
    embed_model_id: &str,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    use crate::ai::{model_catalog::ModelKind, model_manager};

    let Some(app_data_dir) = db
        .get_preference("app_data_dir")
        .ok()
        .flatten()
        .map(std::path::PathBuf::from)
    else {
        return (None, None);
    };

    let chat_path = if !chat_model_id.is_empty() {
        Some(model_manager::model_path(&app_data_dir, ModelKind::Chat, chat_model_id))
    } else {
        None
    };
    let embed_path = if !embed_model_id.is_empty() {
        Some(model_manager::model_path(
            &app_data_dir,
            ModelKind::Embedding,
            embed_model_id,
        ))
    } else {
        None
    };

    (chat_path, embed_path)
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::db::Database;

    /// Serializes tests that mutate the process-global `OPENROUTER_API_KEY` so
    /// they don't stomp on each other under `cargo test`'s parallel runner.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// RAII guard that removes `OPENROUTER_API_KEY` for the test body and
    /// restores the prior value on drop. The Makefile exports the developer's
    /// `.env.local` key into every recipe environment, so without this the
    /// "missing key" assertion would never fire under `make check`.
    struct ClearedOpenRouterKey(Option<String>);

    impl ClearedOpenRouterKey {
        fn new() -> Self {
            let prev = std::env::var("OPENROUTER_API_KEY").ok();
            std::env::remove_var("OPENROUTER_API_KEY");
            Self(prev)
        }
    }

    impl Drop for ClearedOpenRouterKey {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("OPENROUTER_API_KEY", value),
                None => std::env::remove_var("OPENROUTER_API_KEY"),
            }
        }
    }

    /// When the user has configured OpenRouter as the AI provider but not yet
    /// entered an API key, `load_provider` must return an error rather than
    /// constructing a provider with an empty key. This is the production failure
    /// mode that surfaces as "Lens run failed to start: …" in the output panel.
    #[test]
    fn load_provider_openrouter_without_key_returns_error() {
        let _g = env_lock();
        let _no_env_key = ClearedOpenRouterKey::new();
        let db = Database::new_for_testing().expect("test db");
        db.set_preference("ai_provider", "openrouter").unwrap();
        // No key stored — load_provider must fail with a descriptive message.
        let result = AiService::load_provider(&db);
        assert!(result.is_err(), "openrouter without key must fail");
        let msg = result.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            msg.to_lowercase().contains("api key") || msg.to_lowercase().contains("not configured"),
            "error must describe the missing key; got: {msg}"
        );
    }

    /// Fresh installs must default to a tool-capable chat model that exists in
    /// the catalog. Gemma 4 was the old default but lacks reliable tool-calling
    /// and was retired, so the default moved to Qwen 3.5 4B.
    #[test]
    fn default_chat_model_is_qwen_3_5_4b_and_tool_capable() {
        let db = Database::new_for_testing().expect("test db");
        let cfg = AiService::get_config(&db).expect("get_config");
        assert_eq!(cfg.model, "qwen3.5-4b-q4_k_m");
        let entry = crate::ai::model_catalog::find(&cfg.model).expect("default chat model must be in catalog");
        assert_eq!(entry.kind, crate::ai::model_catalog::ModelKind::Chat);
        assert!(entry.supports_tools, "default chat model must support tools");
    }
}

#[cfg(test)]
mod url_validation_tests {
    use super::validate_ai_base_url;

    #[test]
    fn accepts_localhost_and_https_hosts() {
        assert!(validate_ai_base_url("http://localhost:8080").is_ok());
        assert!(validate_ai_base_url("http://127.0.0.1:11434").is_ok());
        assert!(validate_ai_base_url("https://api.example.com/v1").is_ok());
    }

    #[test]
    fn rejects_dangerous_schemes() {
        assert!(validate_ai_base_url("file:///etc/passwd").is_err());
        assert!(validate_ai_base_url("javascript:alert(1)").is_err());
        assert!(validate_ai_base_url("data:text/html,evil").is_err());
        assert!(validate_ai_base_url("gopher://example.com").is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(validate_ai_base_url("not a url").is_err());
        assert!(validate_ai_base_url("http://").is_err());
    }
}
