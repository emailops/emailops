use std::sync::Arc;

/// Default idle window before a loaded local model is evicted from RAM.
/// Overridable via the `chat.keep_alive_seconds` preference.
const DEFAULT_KEEP_ALIVE_SECS: u32 = 30 * 60;

use crate::ai::afm_routing::{plan_afm_route, AfmRoute};
use crate::ai::embedding_route::{plan_embedding_route, EmbeddingRoute};
use crate::ai::foundation_models::{apple_intelligence_status, generation_registered};
use crate::ai::foundation_models_provider::{prompt_fits, FoundationModelsProvider};
use crate::ai::ollama::OllamaClient;
use crate::ai::openrouter::OpenRouterClient;
use crate::ai::provider::{AIProvider, CompletionOptions, CompletionResult, ModelInfo};
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{AiConfig, AiLogEvent, AiUsageSummary};

/// Shown when the configured provider is the embedded runtime but this build
/// was compiled without it (`--no-default-features`, e.g. the Intel-mac bundle
/// or a CI packaging artifact).
#[cfg(not(feature = "llamacpp"))]
const EMBEDDED_AI_UNAVAILABLE: &str = "This build of EmailOps does not include the embedded AI runtime. \
     Choose Ollama or OpenRouter in Settings → AI, or install a build with embedded AI.";

/// Shown when the runtime *is* compiled in but the machine cannot execute it —
/// the universal bundle's x86_64 slice running on an Intel Mac.
#[cfg(feature = "llamacpp")]
const EMBEDDED_AI_UNSUPPORTED_HOST: &str = "The embedded AI runtime requires an Apple Silicon Mac (M1 or newer). \
     Choose OpenRouter in Settings → AI to use the AI features on this Mac.";

/// Refuse the embedded runtime on hosts that cannot run it, before any model is
/// loaded. Without this the failure surfaces several seconds later as an opaque
/// `Decode Error -3: unknown` from the first prefill, on every single turn.
#[cfg(feature = "llamacpp")]
fn ensure_embedded_runtime_supported() -> Result<()> {
    if crate::ai::gpu_plan::embedded_runtime_supported(std::env::consts::OS, std::env::consts::ARCH) {
        return Ok(());
    }
    Err(AppError::AiError(EMBEDDED_AI_UNSUPPORTED_HOST.to_string()))
}

const KEYRING_SERVICE: &str = "emailops";
const OPENROUTER_KEY_ID: &str = "openrouter_api_key";
const OPENROUTER_DEV_KEY_PREF: &str = "openrouter_api_key_dev";

pub struct AiService {
    provider: Arc<dyn AIProvider>,
    /// The provider embeddings actually go to — `provider` itself when it can
    /// embed, the local embedder when it cannot, `None` when neither can.
    /// Resolved eagerly so a backend without embeddings does not discover the
    /// problem per query, in the middle of retrieval.
    embedder: Option<Arc<dyn AIProvider>>,
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

/// Release the embedded AI runtime before the process exits.
///
/// ggml asserts at `exit()` that every Metal buffer has been released, and
/// aborts the process otherwise — so quitting with a model still loaded turns
/// a clean quit into `SIGABRT`. Call this from every process that can load the
/// embedded provider (the Tauri app on `RunEvent::Exit`, and `emailops-cli`
/// before returning from `main`).
///
/// Safe to call when no model was ever loaded, when the `llamacpp` feature is
/// off (it compiles to a no-op), and more than once.
pub fn shutdown_local_ai() {
    #[cfg(feature = "llamacpp")]
    {
        // Bounded: an in-flight generation keeps decoding until it finishes,
        // and we would rather fall through to the caller's backstop than hang
        // the user's quit behind a long completion.
        const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

        let Some(cache) = LLAMACPP_RUNTIME_CACHE.get() else {
            return; // never initialised — nothing was ever loaded
        };
        let runtime = {
            let mut guard = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            // Take the entry so a late `get_or_create` cannot hand the
            // half-torn-down runtime to a new caller during shutdown.
            guard.take().map(|cached| cached.runtime)
        };
        if let Some(runtime) = runtime {
            if !runtime.shutdown(SHUTDOWN_TIMEOUT) {
                crate::services::logger::log(
                    "debug",
                    "ai",
                    "llamacpp: inference thread still busy at shutdown; exiting without waiting".to_string(),
                );
            }
        }
    }
}

/// Release the embedded AI runtime, then leave the process without running C++
/// static destructors.
///
/// The single exit path for every binary that can load the embedded provider —
/// the desktop app, `emailops-cli`, and the `examples/*` tools. Skipping the
/// destructors is the backstop: [`shutdown_local_ai`] removes the usual cause,
/// but the bundled embedding runtime and any future vendored at-exit hook can
/// abort the same way, and a crash on exit is never worth the destructors we
/// skip. Safe here — SQLite is in WAL mode and nothing of ours registers an
/// `atexit` handler.
///
/// macOS-only in effect: the abort comes from ggml's Metal residency-set
/// assert, which has no equivalent in the Vulkan/CPU builds, so other platforms
/// exit normally.
pub fn shutdown_and_exit(code: i32) -> ! {
    shutdown_local_ai();

    #[cfg(target_os = "macos")]
    {
        // SAFETY: `_exit` terminates the process; nothing runs after it.
        unsafe { libc::_exit(code) }
    }
    #[cfg(not(target_os = "macos"))]
    std::process::exit(code)
}

#[cfg(feature = "llamacpp")]
fn get_or_create_llamacpp_runtime(
    chat_path: Option<std::path::PathBuf>,
    embed_path: Option<std::path::PathBuf>,
    keep_alive_secs: u32,
    n_ctx_override: u32,
) -> Arc<crate::ai::llama_cpp::runtime::LlamaCppRuntime> {
    use crate::ai::llama_cpp::runtime::LlamaCppRuntime;

    let cache = LLAMACPP_RUNTIME_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    // Reuse when the paths match exactly. A `None` path on either side means
    // the user hasn't configured that model yet; we still treat matching
    // `None`s as a cache hit so repeated calls before the user picks a
    // model don't thrash. Push the live preferences onto the reused runtime —
    // `set_n_ctx_override` respawns the actor if the window changed.
    if let Some(existing) = guard.as_ref() {
        if existing.chat_path == chat_path && existing.embed_path == embed_path {
            existing.runtime.set_keep_alive_secs(keep_alive_secs);
            existing.runtime.set_n_ctx_override(n_ctx_override);
            return Arc::clone(&existing.runtime);
        }
    }

    let runtime = LlamaCppRuntime::new(chat_path.clone(), embed_path.clone());
    runtime.set_keep_alive_secs(keep_alive_secs);
    runtime.set_n_ctx_override(n_ctx_override);
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

/// Read the `chat.n_ctx` preference: the user's configured context window for
/// the embedded llama.cpp chat model. `0` (or unset / unparseable) means
/// "auto" — let the runtime pick the model's trained context capped at the
/// default. The hard `[floor, model-trained]` clamp lives in
/// `planner::effective_n_ctx`, so this reader only sanitises garbage to `0`.
#[cfg(feature = "llamacpp")]
pub fn load_n_ctx_override(db: &Database) -> u32 {
    db.get_preference("chat.n_ctx")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0)
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

/// What a launch should do about the AI provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupPrewarm {
    /// Load the model. Cheap next to the prefix seed (measured 584ms vs 18.6s
    /// on an iPhone 16 Pro) and it is what makes a later seed fast.
    pub warm_model: bool,
    /// Seed the invariant chat prompt prefix into the provider's KV cache.
    pub seed_prefix: bool,
}

/// Decide what to warm at startup.
///
/// `defer_prefix_to_chat_open` is set on platforms where the seed competes with
/// something the user is more likely to want. On a phone the seed prefills
/// ~4,600 tokens on a CPU that is simultaneously downloading the initial
/// mailbox — 19 seconds of contention imposed on every launch, including the
/// launches where chat is never opened. Deferring is safe because both chat
/// surfaces call `prewarm_chat` when they mount, so opening chat still warms
/// the prefix; the cost simply lands on the person who wants chat.
pub fn plan_startup_prewarm(onboarded: bool, ai_enabled: bool, defer_prefix_to_chat_open: bool) -> StartupPrewarm {
    if !onboarded || !ai_enabled {
        return StartupPrewarm {
            warm_model: false,
            seed_prefix: false,
        };
    }
    StartupPrewarm {
        warm_model: true,
        seed_prefix: !defer_prefix_to_chat_open,
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
        super::secrets_vault::get(KEYRING_SERVICE, &api_key_id)?
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

        super::secrets_vault::set(KEYRING_SERVICE, OPENROUTER_KEY_ID, key)?;
        db.set_preference("openrouter_api_key_id", OPENROUTER_KEY_ID)?;
        Ok(())
    }

    pub fn new(db: Arc<Database>) -> Result<Self> {
        let provider = Self::load_provider(&db)?;
        let embedder = Self::embedder_for(&db, &provider);
        Ok(Self { provider, embedder, db })
    }

    /// Build an `AiService` around an already-constructed provider. Used by
    /// eval harnesses that want to exercise the extraction pipeline with a
    /// specific embedded model without mutating the user's prefs.
    pub fn with_provider(db: Arc<Database>, provider: Arc<dyn AIProvider>) -> Self {
        let embedder = Self::embedder_for(&db, &provider);
        Self { provider, embedder, db }
    }

    pub fn provider(&self) -> &dyn AIProvider {
        self.provider.as_ref()
    }

    pub async fn reload_provider(&mut self) -> Result<()> {
        self.provider = Self::load_provider(&self.db)?;
        self.embedder = Self::embedder_for(&self.db, &self.provider);
        Ok(())
    }

    /// An embedding-only embedded llama.cpp backend, using the configured
    /// embedding model and **no chat model** — `llamacpp_model_paths` maps an
    /// empty chat id to `None`, so nothing multi-gigabyte is loaded.
    ///
    /// This is what keeps the iOS promise that retrieval embeddings never leave
    /// the device on any tier, including the remote-only one: the bundled
    /// `nomic-embed-text-v1.5` GGUF is small enough to run everywhere.
    #[cfg(feature = "llamacpp")]
    fn load_local_embedder(db: &Database) -> Result<Arc<dyn AIProvider>> {
        use crate::ai::llama_cpp::LlamaCppBackend;
        let embedding_model = db
            .get_preference("ai_embedding_model")
            .ok()
            .flatten()
            .unwrap_or_default();
        if embedding_model.is_empty() {
            return Err(AppError::AiError("no embedding model is configured".to_string()));
        }
        let (_, embed_path) = llamacpp_model_paths(db, "", &embedding_model);
        if embed_path.as_ref().is_none_or(|p| !p.exists()) {
            return Err(AppError::AiError(format!(
                "embedding model '{embedding_model}' is not installed"
            )));
        }
        let runtime =
            get_or_create_llamacpp_runtime(None, embed_path, load_keep_alive_secs(db), load_n_ctx_override(db));
        Ok(Arc::new(LlamaCppBackend::new(runtime, String::new(), embedding_model)))
    }

    /// Without the embedded runtime compiled in there is no local embedder at
    /// all — the Intel-mac bundle and CI packaging builds land here.
    #[cfg(not(feature = "llamacpp"))]
    fn load_local_embedder(_db: &Database) -> Result<Arc<dyn AIProvider>> {
        Err(AppError::AiError(
            "this build has no embedded AI runtime, so it cannot embed locally".to_string(),
        ))
    }

    /// Build a provider with a custom provider name and model (e.g., for classification).
    pub fn build_provider(db: &Database, provider_name: &str, model: &str) -> Result<Arc<dyn AIProvider>> {
        let keep_alive_secs = load_keep_alive_secs(db);
        let ollama_keep_alive = format_ollama_keep_alive(keep_alive_secs);
        match provider_name {
            "foundation-models" => Ok(Arc::new(
                crate::ai::foundation_models_provider::FoundationModelsProvider::new(),
            )),
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
                ensure_embedded_runtime_supported()?;
                let embedding_model = db
                    .get_preference("ai_embedding_model")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let (chat_path, embed_path) = llamacpp_model_paths(db, model, &embedding_model);
                // Reuse the cached runtime when (chat_path, embed_path) match
                // an earlier load — avoids reloading the multi-GB GGUF every
                // turn. See CachedLlamaCppRuntime above.
                let runtime =
                    get_or_create_llamacpp_runtime(chat_path, embed_path, keep_alive_secs, load_n_ctx_override(db));
                Ok(Arc::new(LlamaCppBackend::new(
                    runtime,
                    model.to_string(),
                    embedding_model,
                )))
            }
            // Without the `llamacpp` feature there is no arm above to match, so
            // a saved "llamacpp" preference used to fall through to Ollama and
            // report "check that Ollama is running" — blaming a component the
            // user never chose. Fail with what is actually wrong instead.
            #[cfg(not(feature = "llamacpp"))]
            "llamacpp" => Err(AppError::AiError(EMBEDDED_AI_UNAVAILABLE.to_string())),
            _ => Ok(Arc::new(
                OllamaClient::new_with_models(Some(model), None).with_keep_alive(ollama_keep_alive),
            )),
        }
    }

    pub fn load_provider(db: &Database) -> Result<Arc<dyn AIProvider>> {
        Self::load_provider_with_model(db, None)
    }

    /// Like [`load_provider`](Self::load_provider), but selects `model_override`
    /// (when `Some` and non-empty) instead of the configured `ai_model`
    /// preference. The chat turn passes its per-turn model here so an explicit
    /// CLI `--model` / REPL `/model` actually drives the runtime, rather than
    /// silently falling back to the stored preference. A `None` or blank
    /// override keeps the configured model. The provider (Ollama / OpenRouter /
    /// llama.cpp) is still chosen by the `ai_provider` preference.
    pub fn load_provider_with_model(db: &Database, model_override: Option<&str>) -> Result<Arc<dyn AIProvider>> {
        let mut config = Self::get_config(db)?;
        if let Some(m) = model_override.map(str::trim).filter(|m| !m.is_empty()) {
            config.model = m.to_string();
        }
        let keep_alive_secs = load_keep_alive_secs(db);
        let ollama_keep_alive = format_ollama_keep_alive(keep_alive_secs);

        match config.provider.as_str() {
            "foundation-models" => Ok(Arc::new(
                crate::ai::foundation_models_provider::FoundationModelsProvider::new(),
            )),
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
                ensure_embedded_runtime_supported()?;
                let (chat_path, embed_path) = llamacpp_model_paths(db, &config.model, &config.embedding_model);
                let runtime =
                    get_or_create_llamacpp_runtime(chat_path, embed_path, keep_alive_secs, load_n_ctx_override(db));
                Ok(Arc::new(LlamaCppBackend::new(
                    runtime,
                    config.model,
                    config.embedding_model,
                )))
            }
            // Same silent-fallback trap as in `load_provider_with_model`.
            #[cfg(not(feature = "llamacpp"))]
            "llamacpp" => Err(AppError::AiError(EMBEDDED_AI_UNAVAILABLE.to_string())),
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
    pub async fn warmup_from_db(db: &Database, seed_prefix: bool) {
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

        // Seed the chat prompt-prefix cache so the first real turn skips most
        // of its prefill (no-op for backends without a persistent prompt
        // cache). Best-effort account pick: the first enabled account — the
        // chat panel re-fires the prewarm with the actually-selected account
        // when it opens, which also covers multi-account setups.
        //
        // Skipped where the caller decided the seed should wait for chat to be
        // opened — see `plan_startup_prewarm`.
        if !seed_prefix {
            log("debug", "chat prefix prewarm deferred until chat is opened");
            return;
        }
        let account_id = db
            .list_accounts()
            .ok()
            .and_then(|accounts| accounts.into_iter().find(|a| a.enabled).map(|a| a.id));
        let Some(account_id) = account_id else {
            return;
        };
        let registry = crate::services::chat::tools::default_registry();
        match crate::services::chat::prewarm_chat(db, &registry, provider.as_ref(), &account_id).await {
            Ok(()) => log(
                "success",
                &format!(
                    "chat prefix prewarmed ({}) in {}ms total",
                    model,
                    started.elapsed().as_millis()
                ),
            ),
            Err(e) => log("warn", &format!("chat prefix prewarm failed ({}): {}", model, e)),
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
        // Apple's on-device model gets first refusal on short, structured work
        // when it is available and the user has not opted out. Anything it
        // declines — guardrails, an oversized prompt, a transient failure —
        // falls through to the configured backend, silently: a classification
        // that failed because a safety filter fired is a bug, not a result.
        if let Some(result) = self.try_apple_intelligence(prompt, operation, &opts).await {
            self.record_usage(&result, operation)?;
            return Ok(result.text);
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

    /// The provider that should serve embeddings, honouring
    /// `capabilities().embeddings`. `None` when nothing can embed.
    ///
    /// Exposed because `services::chat::retrieval` holds a bare
    /// `Arc<dyn AIProvider>` and embeds through it directly; routing there has
    /// to consult the same decision rather than re-deriving it.
    pub fn embedding_provider(&self) -> Option<&Arc<dyn AIProvider>> {
        self.embedder.as_ref()
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let provider = self.embedding_provider().ok_or_else(|| {
            AppError::AiError(
                "The configured AI backend cannot create embeddings, and no local embedding model is installed."
                    .to_string(),
            )
        })?;
        let result = provider.embed(text).await?;
        self.check_budget(result.cost_usd)?;
        Ok(result.embedding)
    }

    /// Try Apple's on-device model, returning `None` when it should not be
    /// asked or when it declined.
    ///
    /// Never returns an error: every failure here is a reason to use the
    /// configured backend instead, and surfacing it would turn a fallback into
    /// a broken feature.
    async fn try_apple_intelligence(
        &self,
        prompt: &str,
        operation: &str,
        options: &CompletionOptions,
    ) -> Option<CompletionResult> {
        let route = plan_afm_route(
            operation,
            generation_registered() && apple_intelligence_status().is_available(),
            self.apple_intelligence_enabled(),
            prompt_fits(prompt.chars().count()),
        );
        if route == AfmRoute::ConfiguredOnly {
            return None;
        }

        match FoundationModelsProvider::new().complete(prompt, options.clone()).await {
            Ok(result) => Some(result),
            Err(e) => {
                // Debug, not error: a fallback happened and the user got their
                // answer. Logged at all because a backend that declines
                // *everything* should be discoverable without a debugger.
                crate::services::logger::log(
                    "debug",
                    "ai",
                    format!("{operation}: Apple Intelligence declined ({e}); using the configured backend"),
                );
                None
            }
        }
    }

    /// Whether the user lets Apple's model take eligible work. Defaults to on
    /// where the model exists: it is free, private and needs no network. The
    /// preference exists so someone paying for a frontier model can keep it.
    fn apple_intelligence_enabled(&self) -> bool {
        !matches!(
            self.db
                .get_preference("ai_apple_intelligence_enabled")
                .ok()
                .flatten()
                .as_deref(),
            Some("false")
        )
    }

    /// The provider that should serve embeddings for `provider`, honouring
    /// `capabilities().embeddings`. `None` when nothing on this machine can
    /// embed — callers degrade to keyword search and say so.
    ///
    /// The single executor for [`plan_embedding_route`]: `AiService` uses it,
    /// and so does the chat turn, which holds a bare provider rather than a
    /// service. Two implementations of this decision would be one too many.
    ///
    /// Building the local embedder is only attempted when it would actually be
    /// used — otherwise this would load a second model that is never asked a
    /// question. A failure means "no local embedder", which the planner turns
    /// into a clear `Unavailable` instead of a backend error surfacing per
    /// query from deep inside retrieval.
    pub fn embedder_for(db: &Database, provider: &Arc<dyn AIProvider>) -> Option<Arc<dyn AIProvider>> {
        let primary_embeds = provider.capabilities().embeddings;
        let local = if primary_embeds {
            None
        } else {
            match Self::load_local_embedder(db) {
                Ok(embedder) => Some(embedder),
                Err(e) => {
                    crate::services::logger::log(
                        "error",
                        "ai",
                        format!("backend cannot embed and no local embedder could be built: {e}"),
                    );
                    None
                }
            }
        };
        match plan_embedding_route(primary_embeds, local.is_some()) {
            EmbeddingRoute::Primary => Some(provider.clone()),
            EmbeddingRoute::LocalFallback => local,
            EmbeddingRoute::Unavailable => None,
        }
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

    /// An explicit per-turn model overrides the stored `ai_model` preference, so
    /// a chat turn started with CLI `--model` / REPL `/model` actually runs the
    /// requested model instead of silently falling back to the preference.
    #[test]
    fn load_provider_with_model_overrides_pref_model() {
        let db = Database::new_for_testing().expect("test db");
        db.set_preference("ai_provider", "ollama").unwrap();
        db.set_preference("ai_model", "qwen3.5-4b-q8_0").unwrap();

        let provider =
            AiService::load_provider_with_model(&db, Some("gemma-4-12b-it-qat-ud-q4_k_xl")).expect("provider");

        assert_eq!(provider.model_name(), "gemma-4-12b-it-qat-ud-q4_k_xl");
    }

    /// A `None` or blank override keeps the configured `ai_model` — so the
    /// desktop / eval callers (which pass the preference's own value) are
    /// unaffected, and an empty model never blanks out the selection.
    #[test]
    fn load_provider_with_model_none_or_blank_uses_pref() {
        let db = Database::new_for_testing().expect("test db");
        db.set_preference("ai_provider", "ollama").unwrap();
        db.set_preference("ai_model", "qwen3.5-4b-q8_0").unwrap();

        assert_eq!(
            AiService::load_provider_with_model(&db, None)
                .expect("provider")
                .model_name(),
            "qwen3.5-4b-q8_0"
        );
        assert_eq!(
            AiService::load_provider_with_model(&db, Some("   "))
                .expect("provider")
                .model_name(),
            "qwen3.5-4b-q8_0"
        );
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

#[cfg(test)]
mod startup_prewarm_tests {
    use super::*;

    #[test]
    fn a_desktop_launch_warms_the_model_and_seeds_the_prefix() {
        assert_eq!(
            plan_startup_prewarm(true, true, false),
            StartupPrewarm {
                warm_model: true,
                seed_prefix: true,
            }
        );
    }

    #[test]
    fn a_phone_launch_warms_the_model_but_defers_the_prefix() {
        // Measured on an iPhone 16 Pro: loading the model costs 584ms, seeding
        // the 4,630-token prefix costs 18,569ms — and it runs while the initial
        // sync is downloading mail, which is what the user actually opened the
        // app for. Opening chat re-fires the prewarm (ChatView does it on
        // mount), so nothing is lost; the cost just moves to someone who wants
        // chat rather than everyone who launches.
        assert_eq!(
            plan_startup_prewarm(true, true, true),
            StartupPrewarm {
                warm_model: true,
                seed_prefix: false,
            }
        );
    }

    #[test]
    fn a_first_launch_does_nothing_at_all() {
        // Before onboarding we must not load a multi-GB model for a user who
        // has not yet chosen whether they want AI.
        assert_eq!(
            plan_startup_prewarm(false, true, false),
            StartupPrewarm {
                warm_model: false,
                seed_prefix: false,
            }
        );
    }

    #[test]
    fn the_master_ai_switch_still_wins_everywhere() {
        for defer in [false, true] {
            assert_eq!(
                plan_startup_prewarm(true, false, defer),
                StartupPrewarm {
                    warm_model: false,
                    seed_prefix: false,
                }
            );
        }
    }

    #[test]
    fn the_prefix_is_never_seeded_without_the_model_being_warm() {
        // Seeding a prefix into a provider that has not loaded is either a
        // no-op or a second cold load; neither is what the caller intends.
        for (onboarded, ai, defer) in [
            (true, true, false),
            (true, true, true),
            (false, true, false),
            (true, false, false),
        ] {
            let plan = plan_startup_prewarm(onboarded, ai, defer);
            assert!(!plan.seed_prefix || plan.warm_model);
        }
    }
}
