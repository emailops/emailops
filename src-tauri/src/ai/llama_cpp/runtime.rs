// LlamaCppRuntime — owns the loaded model(s) and drives inference.
//
// ARCHITECTURE
// ────────────
// • One global `LlamaBackend` per process (OnceLock).  Initialization is
//   idempotent: BackendAlreadyInitialized is silently ignored.
// • `LlamaModel` is `Send + Sync` (explicit unsafe impl in llama-cpp-2), so it
//   is stored in an `Arc<LlamaModel>` and shared across requests.
// • `LlamaContext<'_>` is NOT `Send` and borrows from the model.  Chat
//   generation runs on a persistent actor thread (actor.rs) that owns one
//   context and reuses its KV cache across requests; embeddings still create
//   a short-lived context inside `spawn_blocking`.
//
// TOOL CALLING (post llama-cpp-2 0.1.147)
// ───────────────────────────────────────
// llama-cpp-2 0.1.147 removed the `apply_chat_template_oaicompat` / `parse_
// response_oaicompat` helpers that we used to lean on for tool-call rendering
// and parsing. We now drive both ends ourselves:
//
//   • Rendering: `apply_chat_template` (the plain variant) takes a
//     `&[LlamaChatMessage]` and an `add_ass: bool` and produces the wire
//     prompt. We render each assistant turn that carries `tool_calls` by
//     embedding them in the content text as `<tool_call>{json}</tool_call>`
//     blocks — the same syntax Qwen 3 emits and the model is taught to
//     produce via the system-prompt format instructions
//     (see `src/services/prompts/defaults.rs`, CHAT_SYSTEM).
//   • Parsing: `tool_parser::parse_qwen_tool_calls` extracts the same
//     `<tool_call>{json}</tool_call>` blocks back out of the model's output.
//     A two-step fallback to `parse_xml_tool_calls` and
//     `parse_python_call_tool_calls` (in `services/chat/turn.rs`) catches
//     malformed shapes from less-aligned models.
//
// We also lose the OAI helpers' `enable_thinking: false` flag; Qwen 3 now
// emits non-empty `<think>` blocks. The existing `strip_reasoning` filter
// removes them post-hoc, and `ThinkingGate` suppresses them mid-stream.
//
// CHAT TEMPLATE SOURCE
// ────────────────────
// We rely on the embedded `tokenizer.chat_template` from the GGUF. Bartowski's
// builds always have one; mradermacher's stripped Gemma 4 builds do NOT — we
// removed the hand-rolled `GEMMA4_CHAT_TEMPLATE` fallback in the 0.1.147
// migration. If a user with a stripped GGUF reports a regression we can
// re-add a fallback in a follow-up.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// One-shot flag for the Gemma 4 chat-template fallback log. `render_template`
/// is called up to four times per chat turn (full prompt, stable-prompt probe,
/// and two system-prefix probes), and emitting the substitution log on each
/// invocation drowns the output panel. We log on the first occurrence per
/// process: the diagnostic value is "this build is exercising the fallback",
/// which only needs to land once.
static GEMMA4_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);

use tokio::sync::{Mutex, Semaphore};

/// How often the idle-eviction background task wakes up to check if loaded
/// models have been unused long enough to drop.
const EVICTION_POLL_INTERVAL_SECS: u64 = 60;

use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, AddBos, LlamaChatMessage, LlamaModel},
};

use super::actor::{InferenceActorHandle, OnToken};
use super::tool_parser::parse_qwen_tool_calls;
use crate::ai::provider::{AiMessage, AiToolCall, ChatStreamResult, CompletionOptions, ToolStreamResult};
use crate::ai::stream_gate::StreamGate;
use crate::ai::thinking_filter::{strip_reasoning, ThinkingGate};
use crate::models::error::{AppError, Result};
use crate::services::chat::{parse_python_call_tool_calls, parse_xml_tool_calls};

// ── Global backend ────────────────────────────────────────────────────────────

static LLAMA_BACKEND: OnceLock<LlamaBackend> = OnceLock::new();

/// Pure predicate deciding which llama.cpp/ggml log levels survive in debug
/// builds. INFO/DEBUG are backend-init chatter (Metal device probing, residency
/// sets, KV-cache sizing) that floods the terminal on every model load; WARN and
/// ERROR carry the decode-failure breadcrumbs (Metal command-buffer error, OOM,
/// NaN) we rely on when diagnosing an opaque "Decode Error". Keep only the latter.
fn debug_log_level_enabled(level: llama_cpp_sys_2::ggml_log_level) -> bool {
    matches!(
        level,
        llama_cpp_sys_2::GGML_LOG_LEVEL_WARN | llama_cpp_sys_2::GGML_LOG_LEVEL_ERROR
    )
}

/// C log callback installed in debug builds: forwards WARN/ERROR lines to stderr
/// verbatim (matching llama.cpp's default handler) and drops everything quieter.
unsafe extern "C" fn debug_filtered_log(
    level: llama_cpp_sys_2::ggml_log_level,
    text: *const std::os::raw::c_char,
    _user_data: *mut std::os::raw::c_void,
) {
    if !debug_log_level_enabled(level) || text.is_null() {
        return;
    }
    // SAFETY: llama.cpp guarantees `text` is a NUL-terminated C string for the
    // duration of the call.
    let msg = unsafe { std::ffi::CStr::from_ptr(text) }.to_string_lossy();
    eprint!("{msg}");
}

/// C log callback that drops every line — release builds and `LLAMA_SILENT=1`.
unsafe extern "C" fn void_log(
    _level: llama_cpp_sys_2::ggml_log_level,
    _text: *const std::os::raw::c_char,
    _user_data: *mut std::os::raw::c_void,
) {
}

/// Install the global llama.cpp / ggml log handler.
///
/// This MUST run before `LlamaBackend::init()`: the Metal device probe
/// (`ggml_metal_device_init`, residency-set setup, KV-cache sizing) is emitted
/// *during* `llama_backend_init` → `ggml_backend_load_all`, so a handler
/// registered afterwards (e.g. `LlamaBackend::void_logs`) never sees those
/// lines and they leak to stderr. Setting the C callback up front filters them.
///
/// - Release / `LLAMA_SILENT=1`: drop everything (`void_log`).
/// - Debug: keep WARN/ERROR decode breadcrumbs, drop the INFO/DEBUG init noise.
fn install_log_callback() {
    let silent = cfg!(not(debug_assertions)) || std::env::var("LLAMA_SILENT").as_deref() == Ok("1");
    let cb: llama_cpp_sys_2::ggml_log_callback = Some(if silent { void_log } else { debug_filtered_log });
    // SAFETY: registering a global C log callback with a static fn and a null
    // user_data pointer. `llama_log_set` also sets ggml's callback, but we set
    // both explicitly (ggml last) so the Metal backend's logs are covered
    // regardless of upstream ordering changes.
    unsafe {
        llama_cpp_sys_2::llama_log_set(cb, std::ptr::null_mut());
        llama_cpp_sys_2::ggml_log_set(cb, std::ptr::null_mut());
    }
}

pub(crate) fn backend() -> &'static LlamaBackend {
    LLAMA_BACKEND.get_or_init(|| {
        // Filter llama.cpp/ggml stderr *before* init so the Metal device-probe
        // chatter emitted inside `LlamaBackend::init()` is already suppressed.
        install_log_callback();
        // `BackendAlreadyInitialized` is not an error in practice — it means
        // the crate's global was initialised by another path (tests, etc.).
        LlamaBackend::init().unwrap_or_else(|_| {
            // If the backend was already initialised, we still need a proof token.
            // Create a dummy one — the backend itself remains intact.
            // SAFETY: We're in a single-threaded init context; the backend IS
            // initialised at this point.
            // Fail-fast: backend is required for any inference; no recovery possible.
            #[allow(clippy::expect_used)]
            LlamaBackend::init().expect("LlamaBackend unexpectedly unavailable")
        })
    })
}

// ── Runtime ───────────────────────────────────────────────────────────────────

/// Shared runtime that owns the loaded model(s) and drives inference.
///
/// Multiple `LlamaCppBackend` instances may share the same `Arc<LlamaCppRuntime>`.
pub struct LlamaCppRuntime {
    /// Full path to the chat GGUF, or `None` if not yet configured.
    chat_model_path: Option<PathBuf>,
    /// Full path to the embedding GGUF, or `None` if not yet configured.
    embed_model_path: Option<PathBuf>,
    /// Lazily loaded chat model.  Initialised on first inference call.
    chat_model: Mutex<Option<Arc<LlamaModel>>>,
    /// Persistent inference actor for the chat model. Spawned lazily together
    /// with the model; holds the KV cache that prefix reuse depends on.
    /// Evicted in lockstep with `chat_model`.
    chat_actor: Mutex<Option<InferenceActorHandle>>,
    /// Lazily loaded embedding model.  Initialised on first embed call.
    embed_model: Mutex<Option<Arc<LlamaModel>>>,
    /// Ensures only one inference (chat or embed) runs at a time on this
    /// runtime.  Concurrent requests on the same Metal/CPU device contend on
    /// the same hardware and hurt total throughput, so we serialise them here.
    inference_sem: Arc<Semaphore>,
    /// Unix-seconds timestamp of the last inference call. Bumped by
    /// `touch_last_used()` before every chat/embed pass so the idle-eviction
    /// task can tell when the model is truly cold.
    last_used: Arc<AtomicI64>,
    /// Seconds of idleness before the loaded model(s) are dropped to free
    /// RAM. 0 = disable eviction (pin forever). Default set by
    /// `LlamaCppRuntime::new`; callable sites override via
    /// `with_keep_alive`.
    keep_alive_secs: Arc<AtomicU32>,
    /// User-configured context window for the chat actor's `LlamaContext`.
    /// `0` = auto (the model's trained context, capped at the default). Read
    /// when the actor is (re)spawned in `get_chat_actor`; `set_n_ctx_override`
    /// drops the live actor on change so the next request rebuilds the context
    /// with the new window.
    n_ctx_override: Arc<AtomicU32>,
}

impl LlamaCppRuntime {
    /// Create a new (unloaded) runtime.  Model files are loaded on demand.
    ///
    /// A background task is spawned to evict loaded models after
    /// `keep_alive_secs` of idleness. The default (30 min) is tuned for a
    /// 16 GB M1: long enough to cover a chat session, short enough that the
    /// model doesn't sit on RAM after the user walks away.
    pub fn new(chat_model_path: Option<PathBuf>, embed_model_path: Option<PathBuf>) -> Arc<Self> {
        let runtime = Arc::new(Self {
            chat_model_path,
            embed_model_path,
            chat_model: Mutex::new(None),
            chat_actor: Mutex::new(None),
            embed_model: Mutex::new(None),
            inference_sem: Arc::new(Semaphore::new(1)),
            last_used: Arc::new(AtomicI64::new(now_secs())),
            keep_alive_secs: Arc::new(AtomicU32::new(30 * 60)),
            n_ctx_override: Arc::new(AtomicU32::new(0)),
        });
        Self::spawn_eviction_task(&runtime);
        runtime
    }

    /// Override the idle-eviction window. 0 pins the model forever.
    pub fn set_keep_alive_secs(&self, secs: u32) {
        self.keep_alive_secs.store(secs, Ordering::Relaxed);
    }

    /// Set the configured chat context window (`0` = auto). The window is baked
    /// into the `LlamaContext` at actor-spawn time, so when the value actually
    /// changes we drop the live actor — best-effort via `try_lock`, since the
    /// only contender is an in-flight inference, and an in-flight request keeps
    /// the old window for its own duration; the *next* `get_chat_actor` reads
    /// the new value and rebuilds the context. This mirrors the idle-eviction
    /// task, which drops the same actor handle.
    pub fn set_n_ctx_override(&self, n_ctx: u32) {
        let prev = self.n_ctx_override.swap(n_ctx, Ordering::Relaxed);
        if prev != n_ctx {
            if let Ok(mut guard) = self.chat_actor.try_lock() {
                *guard = None;
            }
        }
    }

    fn touch_last_used(&self) {
        self.last_used.store(now_secs(), Ordering::Relaxed);
    }

    /// Spawn a periodic task that drops `chat_model` / `embed_model` when they
    /// have been idle longer than `keep_alive_secs`. Uses a weak reference so
    /// the task exits automatically when the last `Arc<LlamaCppRuntime>`
    /// referent is dropped (e.g. when the provider is swapped).
    fn spawn_eviction_task(runtime: &Arc<Self>) {
        let weak = Arc::downgrade(runtime);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(EVICTION_POLL_INTERVAL_SECS));
            // Skip the immediate tick — eviction on t=0 would be pointless.
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(runtime) = weak.upgrade() else {
                    break; // runtime dropped — nothing to evict
                };

                let keep_alive = runtime.keep_alive_secs.load(Ordering::Relaxed);
                if keep_alive == 0 {
                    continue; // eviction disabled
                }

                let idle = now_secs().saturating_sub(runtime.last_used.load(Ordering::Relaxed));
                if idle < keep_alive as i64 {
                    continue;
                }

                // Evict. We drop both models — they'll be lazily reloaded on
                // the next inference call. Use try_lock so an in-flight
                // request is never interrupted; we'll catch it on the next
                // poll.
                //
                // The actor (and its KV cache) is dropped together with the
                // chat model: dropping only one would either leave the old
                // model's memory pinned by the actor, or pair a reloaded
                // model with an actor still running the previous one. The
                // actor thread exits once the last handle clone is gone.
                if let (Ok(mut actor_guard), Ok(mut model_guard)) =
                    (runtime.chat_actor.try_lock(), runtime.chat_model.try_lock())
                {
                    if actor_guard.is_some() || model_guard.is_some() {
                        crate::services::logger::log(
                            "debug",
                            "ai",
                            format!("llamacpp: evicting chat model after {}s idle", idle),
                        );
                        *actor_guard = None;
                        *model_guard = None;
                    }
                }
                if let Ok(mut guard) = runtime.embed_model.try_lock() {
                    if guard.is_some() {
                        crate::services::logger::log(
                            "debug",
                            "ai",
                            format!("llamacpp: evicting embed model after {}s idle", idle),
                        );
                        *guard = None;
                    }
                };

                // Drop the Arc before the next `interval.tick().await`, so we
                // don't hold a strong ref across the sleep (which would block
                // weak.upgrade() from observing runtime drop).
                drop(runtime);
            }
        });
    }

    /// Returns `true` when a chat model file is configured and present on disk.
    pub fn is_ready(&self) -> bool {
        self.chat_model_path.as_ref().is_some_and(|p| p.exists())
    }

    // ── Lazy model loading ────────────────────────────────────────────────────

    async fn get_chat_model(&self) -> Result<Arc<LlamaModel>> {
        let mut guard = self.chat_model.lock().await;
        if let Some(m) = guard.as_ref() {
            return Ok(m.clone());
        }

        let path = self.chat_model_path.clone().ok_or_else(|| {
            AppError::AiError("No chat model configured. Use the AI settings to download a model.".to_string())
        })?;
        if !path.exists() {
            return Err(AppError::AiError(format!(
                "Chat model file not found: {}. Download the model first.",
                path.display()
            )));
        }

        crate::services::logger::log(
            "debug",
            "ai",
            format!("llamacpp: loading chat model: {}", path.display()),
        );
        let model = tokio::task::spawn_blocking(move || {
            // Offload all layers to Metal (Apple Silicon) / CUDA / CPU fallback.
            let params = LlamaModelParams::default().with_n_gpu_layers(u32::MAX);
            LlamaModel::load_from_file(backend(), &path, &params)
                .map_err(|e| AppError::AiError(format!("Failed to load chat model: {}", e)))
        })
        .await
        .map_err(|e| AppError::AiError(format!("Model load panicked: {}", e)))??;

        crate::services::logger::log("debug", "ai", "llamacpp: chat model loaded");
        let model = Arc::new(model);
        *guard = Some(model.clone());
        Ok(model)
    }

    /// Get (or lazily spawn) the persistent inference actor for the chat
    /// model. The actor owns the context whose KV cache is reused across
    /// requests; it lives until evicted alongside `chat_model`.
    async fn get_chat_actor(&self) -> Result<InferenceActorHandle> {
        let mut guard = self.chat_actor.lock().await;
        if let Some(actor) = guard.as_ref() {
            return Ok(actor.clone());
        }
        let model = self.get_chat_model().await?;
        let n_ctx_override = self.n_ctx_override.load(Ordering::Relaxed);
        let actor = InferenceActorHandle::spawn(model, n_ctx_override).map_err(AppError::AiError)?;
        *guard = Some(actor.clone());
        Ok(actor)
    }

    async fn get_embed_model(&self) -> Result<Arc<LlamaModel>> {
        let mut guard = self.embed_model.lock().await;
        if let Some(m) = guard.as_ref() {
            return Ok(m.clone());
        }

        let path = self
            .embed_model_path
            .clone()
            .ok_or_else(|| AppError::AiError("No embedding model configured.".to_string()))?;
        if !path.exists() {
            return Err(AppError::AiError(format!(
                "Embedding model file not found: {}",
                path.display()
            )));
        }

        crate::services::logger::log(
            "debug",
            "ai",
            format!("llamacpp: loading embedding model: {}", path.display()),
        );
        let model = tokio::task::spawn_blocking(move || {
            let params = LlamaModelParams::default().with_n_gpu_layers(u32::MAX);
            LlamaModel::load_from_file(backend(), &path, &params)
                .map_err(|e| AppError::AiError(format!("Failed to load embed model: {}", e)))
        })
        .await
        .map_err(|e| AppError::AiError(format!("Model load panicked: {}", e)))??;

        crate::services::logger::log("debug", "ai", "llamacpp: embedding model loaded");
        let model = Arc::new(model);
        *guard = Some(model.clone());
        Ok(model)
    }

    // ── Prompt rendering ──────────────────────────────────────────────────────

    /// Render the chat template on a blocking thread.
    ///
    /// Uses the plain `apply_chat_template` (the only chat-template entry
    /// point in llama-cpp-2 0.1.147+). Returns the wire prompt as `String`.
    ///
    /// The `apply_chat_template` API takes `&[LlamaChatMessage]`
    /// (role + content) and an `add_ass` boolean. It deliberately doesn't
    /// accept tools or template kwargs — those used to be the OAI-compat
    /// layer's job, which 0.1.147 removed. We carry tool calls into the
    /// content text via `to_llama_chat_messages` so the model still sees the
    /// conversation history correctly; the system prompt teaches it the wire
    /// format via `CHAT_SYSTEM` in `services/prompts/defaults.rs`.
    async fn render_template(
        model: Arc<LlamaModel>,
        messages: Vec<AiMessage>,
        add_generation_prompt: bool,
    ) -> Result<String> {
        tokio::task::spawn_blocking(move || -> std::result::Result<String, String> {
            let template = model
                .chat_template(None)
                .map_err(|e| format!("model lacks embedded chat template: {e}"))?;
            let chat = to_llama_chat_messages(&messages)?;
            match model.apply_chat_template(&template, &chat, add_generation_prompt) {
                Ok(s) => Ok(s),
                // ffi error -1 = llama.cpp's `llama_chat_apply_template` returned
                // "template not supported": its string-pattern detector did not
                // recognise the embedded Jinja. The Gemma 4 family (unsloth's
                // `<|turn>…<turn|>` + `<|channel>…<channel|>` delimiters) is the
                // first one we ship that trips this — substitute a hand-rolled
                // render whose byte output matches the official Jinja for the
                // chat-only path we use. Other failures (NulError, FromUtf8Error,
                // FfiError with a non-(-1) code) propagate so they get surfaced
                // instead of silently producing a wrong-shape prompt.
                Err(e) => {
                    let text = e.to_string();
                    if text.contains("ffi error -1") {
                        // Surface the substitution so debug runs (LLAMA_SILENT off)
                        // record which template family the GGUF carries. One log
                        // per render is fine — these calls are rare relative to
                        // token generation.
                        let tmpl_str = template.to_str().unwrap_or("");
                        if looks_like_gemma4_template(tmpl_str) {
                            // Each chat turn calls `render_template` up to four times
                            // (full prompt + three probes), so logging unconditionally
                            // floods the output panel. Log the substitution exactly
                            // once per process — it's a "build is exercising the
                            // fallback" signal, not per-call diagnostic.
                            if !GEMMA4_FALLBACK_LOGGED.swap(true, Ordering::Relaxed) {
                                crate::services::logger::log(
                                    "debug",
                                    "ai",
                                    "llamacpp: apply_chat_template returned ffi error -1; using hand-rolled Gemma 4 template render (subsequent renders are silent)",
                                );
                            }
                            return Ok(render_gemma4_chat_template(&messages, add_generation_prompt));
                        }
                    }
                    Err(format!("apply_chat_template failed: {text}"))
                }
            }
        })
        .await
        .map_err(|e| AppError::AiError(format!("Template render task panicked: {e}")))?
        .map_err(AppError::AiError)
    }

    /// Byte length of the prompt prefix that re-appears verbatim in the next
    /// turn's render — everything except the generation header (which e.g.
    /// Qwen under `enable_thinking=false` suffixes with an empty think block
    /// that the next turn does NOT re-render). The actor keeps tokens past
    /// this boundary out of the persistent seq-0 cache so the prefix stays
    /// purely extendable on hybrid-attention caches.
    ///
    /// `None` (cache the whole prompt, pre-existing behavior) when the
    /// header-less render is not a strict prefix of the full render or the
    /// render fails — losing cache reuse is acceptable, wrong output is not.
    async fn stable_prompt_bytes(model: Arc<LlamaModel>, messages: Vec<AiMessage>, full_prompt: &str) -> Option<usize> {
        match Self::render_template(model, messages, false).await {
            Ok(hist) if full_prompt.starts_with(&hist) && hist.len() < full_prompt.len() => Some(hist.len()),
            Ok(_) => None,
            Err(e) => {
                crate::services::logger::log(
                    "debug",
                    "ai",
                    format!("stable-prefix render failed (caching whole prompt): {e}"),
                );
                None
            }
        }
    }

    /// Byte length of the INVARIANT system prefix — the leading system
    /// message(s) plus the opening of the first user turn, which is identical
    /// for every conversation that takes the same route. The actor pins these
    /// tokens on the never-evicted anchor sequence so a brand-new conversation
    /// can reuse the system prefix without a partial KV eviction.
    ///
    /// We cannot render the system message(s) alone — most chat templates
    /// reject a conversation with no user turn (`apply_chat_template_oaicompat`
    /// returns ffi error -3). Instead we render two prompts that share the
    /// system block but whose first user byte differs, then take the longest
    /// common prefix: that is exactly the span from the start of the prompt up
    /// to (but not including) the first user-content byte — the invariant
    /// region — and it is template-agnostic.
    ///
    /// `None` when there is no leading system message, the rendered prefix is
    /// not a strict prefix of the full prompt, or rendering fails — losing
    /// anchor reuse is acceptable, wrong output is not.
    async fn system_prefix_bytes(model: Arc<LlamaModel>, messages: &[AiMessage], full_prompt: &str) -> Option<usize> {
        // Every `return None` path is annotated with a one-line diagnostic log
        // at info level so the chat reasoning trace's
        // "anchor wiped and NOT reseeded — sys_tok=0" message has a paired
        // explanation in the output panel. Branch names are stable so they
        // can be grepped from a bench run's stderr.
        let log_none = |reason: &str| {
            crate::services::logger::log(
                "info",
                "ai",
                format!("llamacpp sys_prefix_bytes: returned None ({reason}) — anchor will not be seeded this call"),
            );
        };

        let sys_only: Vec<AiMessage> = messages.iter().take_while(|m| m.role == "system").cloned().collect();
        if sys_only.is_empty() {
            let first_role = messages.first().map(|m| m.role.as_str()).unwrap_or("(none)");
            log_none(&format!("no leading system message (first role={first_role})"));
            return None;
        }
        let probe = |marker: &str| -> Vec<AiMessage> {
            let mut msgs = sys_only.clone();
            msgs.push(AiMessage {
                role: "user".to_string(),
                content: marker.to_string(),
                tool_calls: None,
            });
            msgs
        };
        let a = match Self::render_template(Arc::clone(&model), probe("A"), false).await {
            Ok(r) => r,
            Err(e) => {
                log_none(&format!("probe A render failed: {e}"));
                return None;
            }
        };
        let b = match Self::render_template(model, probe("B"), false).await {
            Ok(r) => r,
            Err(e) => {
                log_none(&format!("probe B render failed: {e}"));
                return None;
            }
        };
        let lcp = a
            .as_bytes()
            .iter()
            .zip(b.as_bytes())
            .take_while(|(x, y)| x == y)
            .count();
        // Snap back to a UTF-8 boundary so downstream `&prompt[..len]` is valid.
        let mut len = lcp;
        while len > 0 && !a.is_char_boundary(len) {
            len -= 1;
        }
        if len == 0 {
            log_none(&format!(
                "probe LCP collapsed to 0 (raw lcp={lcp}, A.len={}, B.len={}) — templates diverge at the first byte",
                a.len(),
                b.len()
            ));
            return None;
        }
        if len >= full_prompt.len() {
            log_none(&format!(
                "probe LCP {len} ≥ full_prompt.len() {} — nothing left to decode after the system prefix",
                full_prompt.len()
            ));
            return None;
        }
        if !full_prompt.is_char_boundary(len) {
            log_none(&format!(
                "full_prompt is not on a char boundary at byte {len} (probe lcp={lcp})"
            ));
            return None;
        }
        if full_prompt.as_bytes()[..len] != a.as_bytes()[..len] {
            // The render with add_generation_prompt=true (full_prompt) doesn't
            // share its first `len` bytes with the probe render (which used
            // add_generation_prompt=false). This means the chat template's
            // output isn't a clean superset of the no-gen-prompt render — e.g.
            // a template branch that gates the SYSTEM block on later messages
            // or on `add_generation_prompt` itself. Surface enough context that
            // the next debugging session can spot the divergence.
            let div = full_prompt
                .as_bytes()
                .iter()
                .zip(a.as_bytes())
                .take_while(|(x, y)| x == y)
                .count();
            log_none(&format!(
                "full_prompt does NOT share probe A's first {len} bytes — diverges at byte {div}"
            ));
            return None;
        }
        Some(len)
    }

    // ── Public inference API ──────────────────────────────────────────────────

    /// Non-streaming single-turn completion.
    pub async fn generate(&self, prompt: &str, opts: &CompletionOptions) -> Result<String> {
        self.touch_last_used();
        let model = self.get_chat_model().await?;
        let actor = self.get_chat_actor().await?;
        let temperature = opts.temperature.unwrap_or(0.8) as f32;
        let max_tokens = opts.max_tokens.unwrap_or(2048) as usize;

        // Instruction-tuned models (Gemma 4, Llama 3, Qwen) require chat-template
        // turn tokens to produce output — a raw prompt makes the model emit EOG
        // immediately → empty response.
        let messages = vec![AiMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
            tool_calls: None,
        }];

        let _permit = Arc::clone(&self.inference_sem)
            .acquire_owned()
            .await
            .map_err(|_| AppError::AiError("Inference semaphore closed".into()))?;

        let mut prompt_str = Self::render_template(Arc::clone(&model), messages, true).await?;
        // One-shot completions (classification, rewrite, rerank, summary
        // extraction) need raw payload, not reasoning — disable thinking so an
        // unbounded `<think>…</think>` span can't swallow the token budget and
        // collapse the reply to "". See `no_think_priming`.
        prompt_str.push_str(no_think_priming(self.chat_model_path.as_deref()));
        let outcome = actor
            // One-shot completion (rewrite/rerank/extraction/warmup): never
            // cached — its prompt would evict the reusable chat prefix. No
            // anchoring either (cache_prompt=false runs entirely on seq 1).
            .generate(prompt_str, temperature, max_tokens, false, None, None, None)
            .await
            .map_err(AppError::AiError)?;

        // Strip any reasoning/thinking markers (Gemma 4 `<|channel>…<channel|>`,
        // Qwen `<think>…</think>`) the model leaked into the visible answer.
        Ok(strip_reasoning(&outcome.text))
    }

    /// Streaming generation.  `on_token` is called for each piece; returning
    /// `false` cancels generation.
    pub async fn chat_stream(
        &self,
        messages: Vec<AiMessage>,
        on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ChatStreamResult> {
        self.touch_last_used();
        let model = self.get_chat_model().await?;
        let actor = self.get_chat_actor().await?;
        let temperature = 0.8f32;
        let max_tokens = 2048usize;

        let _permit = Arc::clone(&self.inference_sem)
            .acquire_owned()
            .await
            .map_err(|_| AppError::AiError("Inference semaphore closed".into()))?;

        let mut prompt_str = Self::render_template(Arc::clone(&model), messages.clone(), true).await?;
        let stable_bytes = Self::stable_prompt_bytes(Arc::clone(&model), messages.clone(), &prompt_str).await;
        let system_bytes = Self::system_prefix_bytes(model, &messages, &prompt_str).await;
        // Disable thinking on the chat answer pass too — on long/multi-email
        // prompts the generation reserve shrinks to GEN_RESERVE_TOKENS and an
        // unbounded `<think>` span would consume it, yielding an empty reply.
        // Appended after the prefix byte counts so the cache anchor is intact.
        prompt_str.push_str(no_think_priming(self.chat_model_path.as_deref()));

        // Suppress reasoning/thinking spans from the live stream so the user
        // never sees `<|channel>…<channel|>` / `<think>…</think>`. The actor
        // invokes the callback on its own thread, so the gate travels inside
        // the callback behind a shared handle; the end-of-stream flush happens
        // here, caller-side, after the actor replies.
        let gate_state = Arc::new(std::sync::Mutex::new((ThinkingGate::new(), on_token)));
        let actor_cb: OnToken = {
            let gate_state = Arc::clone(&gate_state);
            Box::new(move |piece: String| {
                let mut guard = gate_state.lock().unwrap_or_else(PoisonError::into_inner);
                let (gate, cb) = &mut *guard;
                let out = gate.push(&piece);
                if out.is_empty() {
                    true
                } else {
                    cb(out)
                }
            })
        };

        let outcome = actor
            .generate(
                prompt_str,
                temperature,
                max_tokens,
                true,
                stable_bytes,
                system_bytes,
                Some(actor_cb),
            )
            .await
            .map_err(AppError::AiError)?;

        {
            let mut guard = gate_state.lock().unwrap_or_else(PoisonError::into_inner);
            let (gate, cb) = &mut *guard;
            let tail = gate.finish();
            if !tail.is_empty() {
                let _ = cb(tail);
            }
        }

        Ok(ChatStreamResult {
            content: strip_reasoning(&outcome.text),
            eval_count: Some(outcome.gen_tokens),
            prompt_eval_count: Some(outcome.prompt_tokens),
            prefill_ms: Some(outcome.prefill_ms),
            cached_prompt_tokens: Some(outcome.cached_prompt_tokens),
            prefix_plan: outcome.prefix_plan,
            sys_cached_before: Some(outcome.sys_cached_before),
            sys_cached_after: Some(outcome.sys_cached_after),
            system_prefix_tokens: Some(outcome.system_prefix_tokens),
            stable_tokens: Some(outcome.stable_tokens),
            dropped_front_tokens: Some(outcome.dropped_front_tokens),
        })
    }

    /// Non-streaming tool-call round.
    ///
    /// Renders the prompt with the model's native chat template and extracts
    /// any `<tool_call>{json}</tool_call>` blocks the model emitted via
    /// `parse_qwen_tool_calls` (primary), falling back to the more permissive
    /// `parse_xml_tool_calls` / `parse_python_call_tool_calls` salvage parsers
    /// for model families that emit a different syntax. The `tools` argument
    /// is no longer plumbed into the template (the 0.1.147 plain
    /// `apply_chat_template` doesn't accept one) — the model sees the tool
    /// catalogue via the system prompt's `tools_section` template variable.
    pub async fn chat_with_tools(&self, messages: &[AiMessage], _tools: &[serde_json::Value]) -> Result<AiMessage> {
        self.touch_last_used();
        let model = self.get_chat_model().await?;
        let actor = self.get_chat_actor().await?;
        let temperature = 0.0f32; // greedy for deterministic tool selection
                                  // 4096 leaves headroom for tool-calls that include thinking traces
                                  // or wide structured schemas (e.g. Lens extraction with many fields).
        let max_tokens = 4096usize;

        let _permit = Arc::clone(&self.inference_sem)
            .acquire_owned()
            .await
            .map_err(|_| AppError::AiError("Inference semaphore closed".into()))?;

        let mut prompt_str = Self::render_template(Arc::clone(&model), messages.to_vec(), true).await?;
        let stable_bytes = Self::stable_prompt_bytes(Arc::clone(&model), messages.to_vec(), &prompt_str).await;
        let system_bytes = Self::system_prefix_bytes(model, messages, &prompt_str).await;
        // Disable thinking on tool-call rounds — keeps the reserve for the
        // tool call / answer instead of an unbounded `<think>` span.
        prompt_str.push_str(no_think_priming(self.chat_model_path.as_deref()));
        let outcome = actor
            .generate(
                prompt_str,
                temperature,
                max_tokens,
                true,
                stable_bytes,
                system_bytes,
                None,
            )
            .await
            .map_err(AppError::AiError)?;

        Ok(extract_tool_calls(&outcome.text))
    }

    /// Decode an OpenAI-shaped assistant message JSON (as produced by
    /// `parse_response_oaicompat` or the content-only fallback) into an
    /// `AiMessage`. When tool calls are present the content is dropped — the
    /// Streaming tool-call round. Combines `chat_with_tools` (native tool-call
    /// parsing via `parse_qwen_tool_calls` + salvage chain) with live token
    /// streaming: the model's output is fed through a [`StreamGate`] so any
    /// `<tool_call>` syntax is suppressed while prose is forwarded to
    /// `on_token`. After generation we re-parse the full output to extract
    /// the structured `tool_calls`, which were never streamed.
    pub async fn chat_stream_with_tools(
        &self,
        messages: &[AiMessage],
        _tools: &[serde_json::Value],
        on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ToolStreamResult> {
        self.touch_last_used();
        let model = self.get_chat_model().await?;
        let actor = self.get_chat_actor().await?;
        let temperature = 0.0f32; // greedy for deterministic tool selection
        let max_tokens = 4096usize;

        let _permit = Arc::clone(&self.inference_sem)
            .acquire_owned()
            .await
            .map_err(|_| AppError::AiError("Inference semaphore closed".into()))?;

        let mut prompt_str = Self::render_template(Arc::clone(&model), messages.to_vec(), true).await?;
        let stable_bytes = Self::stable_prompt_bytes(Arc::clone(&model), messages.to_vec(), &prompt_str).await;
        let system_bytes = Self::system_prefix_bytes(model, messages, &prompt_str).await;
        // Disable thinking on the streaming tool-call round — see
        // `no_think_priming`; prevents the unbounded-`<think>` empty-reply path.
        prompt_str.push_str(no_think_priming(self.chat_model_path.as_deref()));

        // Gate each piece through two filters: first strip reasoning spans
        // (`<|channel>…`, `<think>…`), then suppress tool-call syntax;
        // whatever survives is flushed live to the user. The gates travel
        // inside the actor callback; the end-of-stream flush happens here.
        let gate_state = Arc::new(std::sync::Mutex::new((
            ThinkingGate::new(),
            StreamGate::new(),
            on_token,
        )));
        let actor_cb: OnToken = {
            let gate_state = Arc::clone(&gate_state);
            Box::new(move |piece: String| {
                let mut guard = gate_state.lock().unwrap_or_else(PoisonError::into_inner);
                let (think_gate, gate, cb) = &mut *guard;
                let dereasoned = think_gate.push(&piece);
                if dereasoned.is_empty() {
                    return true;
                }
                let out = gate.push(&dereasoned);
                if out.is_empty() {
                    true
                } else {
                    cb(out)
                }
            })
        };

        let outcome = actor
            .generate(
                prompt_str,
                temperature,
                max_tokens,
                true,
                stable_bytes,
                system_bytes,
                Some(actor_cb),
            )
            .await
            .map_err(AppError::AiError)?;

        {
            // Flush both gates at end of stream: the thinking gate only ever
            // holds truncated markup (dropped), then forward buffered prose.
            let mut guard = gate_state.lock().unwrap_or_else(PoisonError::into_inner);
            let (think_gate, gate, cb) = &mut *guard;
            think_gate.finish();
            let tail = gate.finish();
            if !tail.is_empty() {
                let _ = cb(tail);
            }
        }

        let message = extract_tool_calls(&outcome.text);
        Ok(ToolStreamResult {
            message,
            eval_count: Some(outcome.gen_tokens),
            prompt_eval_count: Some(outcome.prompt_tokens),
            prefill_ms: Some(outcome.prefill_ms),
            cached_prompt_tokens: Some(outcome.cached_prompt_tokens),
            prefix_plan: outcome.prefix_plan,
            sys_cached_before: Some(outcome.sys_cached_before),
            sys_cached_after: Some(outcome.sys_cached_after),
            system_prefix_tokens: Some(outcome.system_prefix_tokens),
            stable_tokens: Some(outcome.stable_tokens),
            dropped_front_tokens: Some(outcome.dropped_front_tokens),
        })
    }

    // ── Embeddings ────────────────────────────────────────────────────────────

    /// Generate a normalised embedding vector for `text`.
    ///
    /// Uses the embedding model (encoder-mode GGUF) and returns the mean-pooled,
    /// L2-normalised vector via `embeddings_seq_ith(0)`.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.touch_last_used();
        let model = self.get_embed_model().await?;
        let text_owned = text.to_string();

        let _permit = Arc::clone(&self.inference_sem)
            .acquire_owned()
            .await
            .map_err(|_| AppError::AiError("Inference semaphore closed".into()))?;

        let embedding = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<f32>, String> {
            // Max context for most embedding models is 512 tokens.
            let ctx_params = LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(512))
                .with_embeddings(true);

            let mut ctx = model
                .new_context(backend(), ctx_params)
                .map_err(|e| format!("Embedding context creation failed: {}", e))?;

            let tokens = model
                .str_to_token(&text_owned, AddBos::Always)
                .map_err(|e| format!("Tokenisation failed: {}", e))?;

            if tokens.is_empty() {
                return Ok(vec![]);
            }

            // Clamp to the context window.
            let n = tokens.len().min(511);
            let mut batch = LlamaBatch::new(n, 1);
            for (i, &token) in tokens.iter().take(n).enumerate() {
                // Enable logits for every token so mean pooling works correctly.
                batch
                    .add(token, i as i32, &[0], true)
                    .map_err(|e| format!("Batch add error: {}", e))?;
            }

            // For encoder-mode (embedding) models, use `encode` not `decode`.
            // `decode` also works for causal models with embeddings=true.
            ctx.decode(&mut batch)
                .map_err(|e| format!("Embedding decode failed: {}", e))?;

            // `embeddings_seq_ith(0)` returns the mean-pooled embedding for
            // sequence 0, which is exactly what nomic-embed and e5 expect.
            let raw = ctx
                .embeddings_seq_ith(0)
                .map_err(|e| format!("Failed to read embeddings: {}", e))?;

            // L2-normalise so cosine similarity == dot product.
            let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
            let vec: Vec<f32> = if norm > 1e-9 {
                raw.iter().map(|x| x / norm).collect()
            } else {
                raw.to_vec()
            };

            Ok(vec)
        })
        .await
        .map_err(|e| AppError::AiError(format!("Embedding task panicked: {}", e)))?
        .map_err(AppError::AiError)?;

        Ok(embedding)
    }

    // ── Warmup ────────────────────────────────────────────────────────────────

    /// Force-load the chat model into RAM and fire a 1-token pass so any
    /// lazy GPU/Metal buffer allocations happen before the user's first turn.
    /// Called at app startup so the first chat doesn't eat the 3-6s cold
    /// load of a multi-GB GGUF.
    pub async fn warmup_chat(&self) -> Result<()> {
        if self.chat_model_path.is_none() {
            return Ok(()); // nothing configured — not a failure
        }
        self.touch_last_used();
        let opts = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(1),
            think: Some(false),
        };
        // Tiny prompt; we throw the output away. Errors bubble up so the
        // caller can log them, but warmup failures must not block startup.
        let _ = self.generate("hi", &opts).await?;
        Ok(())
    }
}

/// Unix-seconds timestamp. Saturates to 0 if the system clock is pre-epoch
/// (shouldn't happen but avoids an ugly unwrap).
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── Message conversion / tool-call extraction ────────────────────────────────

/// Convert our provider-neutral `AiMessage` representation into the typed
/// `LlamaChatMessage` shape that `apply_chat_template` accepts. Tool calls and
/// tool results are inlined into the content text using Qwen's
/// `<tool_call>{json}</tool_call>` syntax so the model's render of the
/// conversation history matches what we taught it to produce in the system
/// prompt.
///
/// Returns `Err` if any message contains a null byte (CString refuses them);
/// this is theoretically impossible for messages coming through the chat
/// pipeline because we strip null bytes at sanitisation time, but we surface
/// the error properly rather than silently dropping content.
fn to_llama_chat_messages(messages: &[AiMessage]) -> std::result::Result<Vec<LlamaChatMessage>, String> {
    let mut out = Vec::with_capacity(messages.len());
    for msg in messages {
        let content = match msg.role.as_str() {
            "assistant" => match &msg.tool_calls {
                Some(calls) if !calls.is_empty() => render_assistant_tool_calls(&msg.content, calls),
                _ => msg.content.clone(),
            },
            _ => msg.content.clone(),
        };
        out.push(LlamaChatMessage::new(msg.role.clone(), content).map_err(|e| {
            format!(
                "invalid chat message (role={}, len={}): {e}",
                msg.role,
                msg.content.len()
            )
        })?);
    }
    Ok(out)
}

/// Render an assistant turn's tool calls into the `<tool_call>{json}</tool_call>`
/// syntax that mirrors how Qwen 3 emits them. Embeds the calls AFTER any
/// pre-existing `content` so models see a coherent "I'll do X, here is the
/// call" structure when they read it back from history.
fn render_assistant_tool_calls(content: &str, calls: &[AiToolCall]) -> String {
    let mut buf = String::with_capacity(content.len() + calls.len() * 64);
    if !content.is_empty() {
        buf.push_str(content);
        if !content.ends_with('\n') {
            buf.push('\n');
        }
    }
    for tc in calls {
        buf.push_str("<tool_call>");
        let payload = serde_json::json!({
            "name": tc.function.name,
            "arguments": tc.function.arguments,
        });
        if let Ok(s) = serde_json::to_string(&payload) {
            buf.push_str(&s);
        }
        buf.push_str("</tool_call>");
    }
    buf
}

/// Parse the model's final text output into an `AiMessage`, with any
/// tool-call blocks lifted into the structured `tool_calls` field.
///
/// Strategy: try the Qwen-native `<tool_call>{json}</tool_call>` parser
/// (`parse_qwen_tool_calls`) first; fall back to the existing Hermes-style
/// `parse_xml_tool_calls` and Python-call-literal `parse_python_call_tool_calls`
/// salvage parsers if Qwen-style finds nothing. When ANY parser produces a
/// non-empty list, treat the turn as a tool-call turn and zero out the prose
/// content (the tool round's job is to dispatch, not to surface the JSON
/// markup back to the user).
fn extract_tool_calls(text: &str) -> AiMessage {
    let stripped = strip_reasoning(text);
    let mut tool_calls = parse_qwen_tool_calls(&stripped);
    if tool_calls.is_empty() {
        tool_calls = parse_xml_tool_calls(&stripped);
    }
    if tool_calls.is_empty() {
        // Python-call fallback requires the list of known tool names so it
        // doesn't false-positive on prose like "see `search_emails(...)` for
        // details". We don't have the registry here, so the empty allowlist
        // restricts this branch to the explicit `tool_call:` prefix form.
        tool_calls = parse_python_call_tool_calls(&stripped, &[]);
    }
    if tool_calls.is_empty() {
        AiMessage {
            role: "assistant".to_string(),
            content: stripped,
            tool_calls: None,
        }
    } else {
        AiMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(tool_calls),
        }
    }
}

/// True when the chat-model GGUF on disk is a Qwen 3 family build (Qwen 3 or
/// Qwen 3.5, instruct or base). The OAI-compat layer's `enable_thinking=false`
/// flag is no longer plumbed through `apply_chat_template`, and the
/// in-message `/no_think` directive is silently ignored by some Qwen 3 GGUFs
/// (notably `qwen3.5-4b-q4_k_m`). The caller uses this flag to manually
/// append the canonical "thinking disabled" priming block after the assistant
/// generation header so one-shot completions don't get swallowed by an
/// unbounded `<think>…</think>` span.
fn is_qwen3_model_path(path: &std::path::Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.starts_with("qwen3")
}

/// The empty closed `<think></think>` block that puts a Qwen 3 family model
/// into no-think mode — Qwen reads it as "thinking already done, emit the
/// answer now" and skips the (otherwise unbounded) reasoning span. Returns an
/// empty string for non-Qwen models.
///
/// Why this matters for CHAT (not just one-shot completions): on a near-full
/// context window the prompt-budget planner shrinks the generation reserve to
/// `GEN_RESERVE_TOKENS` (1024). A reasoning model then spends that whole reserve
/// inside `<think>…` and `strip_reasoning` collapses the reply to "" — an empty
/// answer on long/multi-email prompts. Priming no-think keeps the answer inside
/// the reserve. Append it AFTER the cached-prefix byte counts are computed: it
/// lands at the generation point (prompt tail), so it never shifts the prefix.
fn no_think_priming(model_path: Option<&std::path::Path>) -> &'static str {
    match model_path {
        Some(p) if is_qwen3_model_path(p) => "<think>\n\n</think>\n\n",
        _ => "",
    }
}

/// True when the GGUF-embedded chat template is the Gemma 4 family — uses
/// `<|turn>` / `<turn|>` / `<|channel>` / `<channel|>` instead of the
/// Gemma 2/3 `<start_of_turn>` / `<end_of_turn>` delimiters. llama.cpp's
/// `llama_chat_apply_template` only recognises the older marker pair and
/// returns "template not supported" (ffi error -1) on these new templates;
/// the caller uses this flag to decide whether to substitute the hand-rolled
/// fallback render below.
fn looks_like_gemma4_template(template_src: &str) -> bool {
    template_src.contains("<|turn>") && template_src.contains("<channel|>")
}

/// Hand-rolled Gemma 4 chat template render.
///
/// Mirrors the exact byte output of the official Gemma 4 Jinja template for
/// the plain-text, single-modal, no-tools chat path that EmailOps actually
/// uses. Unsupported template branches (image / audio / video items, the tool
/// catalogue macros, `enable_thinking=true`) are out of scope — this exists
/// only to keep drafts and chat working on Gemma 4 12B until llama.cpp ships
/// a built-in pattern for this template family.
///
/// Shape (matches Jinja2 ground truth):
///   per message:   "<|turn>{role}\n{content}<turn|>\n"
///                  where role: user → "user", assistant → "model",
///                              system → "system", tool → "tool_response"
///   if add_generation_prompt:
///                  "<|turn>model\n<|channel>thought\n<channel|>"
fn render_gemma4_chat_template(messages: &[AiMessage], add_generation_prompt: bool) -> String {
    let mut out = String::new();
    for msg in messages {
        let role = match msg.role.as_str() {
            "assistant" => "model",
            "tool" => "tool_response",
            other => other, // "user", "system", and anything else pass through
        };
        let content = match msg.role.as_str() {
            "assistant" => match &msg.tool_calls {
                Some(calls) if !calls.is_empty() => render_assistant_tool_calls(&msg.content, calls),
                _ => msg.content.clone(),
            },
            _ => msg.content.clone(),
        };
        out.push_str("<|turn>");
        out.push_str(role);
        out.push('\n');
        out.push_str(&content);
        out.push_str("<turn|>\n");
    }
    if add_generation_prompt {
        out.push_str("<|turn>model\n<|channel>thought\n<channel|>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> AiMessage {
        AiMessage {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
        }
    }

    #[test]
    fn debug_log_filter_drops_info_keeps_warn_error() {
        use llama_cpp_sys_2::{GGML_LOG_LEVEL_DEBUG, GGML_LOG_LEVEL_ERROR, GGML_LOG_LEVEL_INFO, GGML_LOG_LEVEL_WARN};
        // Chatty backend-init lines (Metal device probe, KV sizing) are INFO/DEBUG.
        assert!(!debug_log_level_enabled(GGML_LOG_LEVEL_INFO));
        assert!(!debug_log_level_enabled(GGML_LOG_LEVEL_DEBUG));
        // Decode-failure breadcrumbs ride in on WARN/ERROR — keep them in debug.
        assert!(debug_log_level_enabled(GGML_LOG_LEVEL_WARN));
        assert!(debug_log_level_enabled(GGML_LOG_LEVEL_ERROR));
    }

    #[test]
    fn is_qwen3_model_path_detects_qwen3_ggufs() {
        use std::path::PathBuf;
        // The catalog ids that ship as defaults.
        assert!(is_qwen3_model_path(&PathBuf::from(
            "/models/chat/qwen3.5-4b-q4_k_m.gguf"
        )));
        assert!(is_qwen3_model_path(&PathBuf::from(
            "/models/chat/qwen3.5-9b-q4_k_m.gguf"
        )));
        assert!(is_qwen3_model_path(&PathBuf::from("Qwen3-14B-Instruct.gguf")));
        // Older Qwen families don't get the thinking-disable primer — they
        // never had a `<think>` mode to begin with.
        assert!(!is_qwen3_model_path(&PathBuf::from(
            "/models/chat/qwen2.5-7b-instruct.gguf"
        )));
        // Other thinking families need a different priming shape (Gemma 4
        // uses `<|channel>`, DeepSeek-R1 uses `<think>` but ships with the
        // closed-block hint already in the template).
        assert!(!is_qwen3_model_path(&PathBuf::from(
            "/models/chat/gemma-4-12b-it-qat-ud-q4_k_xl.gguf"
        )));
        assert!(!is_qwen3_model_path(&PathBuf::from(
            "/models/chat/deepseek-r1-distill-llama-8b.gguf"
        )));
        assert!(!is_qwen3_model_path(&PathBuf::from("")));
    }

    #[test]
    fn no_think_priming_only_for_qwen3() {
        use std::path::PathBuf;
        // Qwen 3 family (incl. 3.6 MoE) gets the closed think block so an
        // unbounded `<think>` span can't swallow the generation reserve.
        let qwen = PathBuf::from("/models/chat/qwen3.6-35b-a3b-ud-q4_k_xl.gguf");
        assert_eq!(no_think_priming(Some(qwen.as_path())), "<think>\n\n</think>\n\n");
        // Non-Qwen and absent paths prime nothing.
        let gemma = PathBuf::from("/models/chat/gemma-4-12b-it-qat-ud-q4_k_xl.gguf");
        assert_eq!(no_think_priming(Some(gemma.as_path())), "");
        assert_eq!(no_think_priming(None), "");
    }

    #[test]
    fn looks_like_gemma4_template_detects_new_markers() {
        // Real Gemma 4 templates contain both `<|turn>` and `<channel|>`.
        assert!(looks_like_gemma4_template(
            "{{- '<|turn>user\n' -}}{{- '<channel|>' -}}"
        ));
        // Gemma 2/3 templates use a completely different delimiter pair.
        assert!(!looks_like_gemma4_template(
            "{{ '<start_of_turn>user' }}{{ '<end_of_turn>' }}"
        ));
        // Chatml uses `<|im_start|>` etc. — must not false-positive.
        assert!(!looks_like_gemma4_template("<|im_start|>user<|im_end|>"));
        // Empty template never looks like Gemma 4.
        assert!(!looks_like_gemma4_template(""));
    }

    #[test]
    fn render_gemma4_user_only_with_gen_prompt() {
        // Ground truth from rendering the actual GGUF-embedded Jinja with
        // Python jinja2: single user turn + add_generation_prompt=true →
        // "<|turn>user\nHello!<turn|>\n<|turn>model\n<|channel>thought\n<channel|>"
        let out = render_gemma4_chat_template(&[msg("user", "Hello!")], true);
        assert_eq!(
            out,
            "<|turn>user\nHello!<turn|>\n<|turn>model\n<|channel>thought\n<channel|>"
        );
    }

    #[test]
    fn render_gemma4_system_plus_user_with_gen_prompt() {
        // Ground truth: system prefix is emitted via `<|turn>system\n…<turn|>`
        // when present, NOT merged into the first user turn (Gemma 4 differs
        // from Gemma 2/3 on this point).
        let out = render_gemma4_chat_template(&[msg("system", "You are helpful."), msg("user", "Hi!")], true);
        assert_eq!(
            out,
            "<|turn>system\nYou are helpful.<turn|>\n<|turn>user\nHi!<turn|>\n<|turn>model\n<|channel>thought\n<channel|>"
        );
    }

    #[test]
    fn render_gemma4_assistant_role_is_renamed_to_model() {
        // Ground truth: `assistant` → `model`. Other roles pass through.
        let out = render_gemma4_chat_template(&[msg("user", "A"), msg("assistant", "B"), msg("user", "C")], true);
        assert_eq!(
            out,
            "<|turn>user\nA<turn|>\n<|turn>model\nB<turn|>\n<|turn>user\nC<turn|>\n<|turn>model\n<|channel>thought\n<channel|>"
        );
    }

    #[test]
    fn render_gemma4_without_gen_prompt_omits_model_header() {
        // The probe paths (`stable_prompt_bytes`, `system_prefix_bytes`) call
        // the render with add_generation_prompt=false; the result must end at
        // the last `<turn|>\n` with no model/channel suffix.
        let out = render_gemma4_chat_template(&[msg("user", "A")], false);
        assert_eq!(out, "<|turn>user\nA<turn|>\n");
    }

    #[test]
    fn render_gemma4_tool_role_becomes_tool_response() {
        // Tool messages map to the `tool_response` role used by the Gemma 4
        // template's tool-handling branch.
        let out = render_gemma4_chat_template(&[msg("tool", "ok")], false);
        assert_eq!(out, "<|turn>tool_response\nok<turn|>\n");
    }

    #[test]
    fn render_gemma4_assistant_tool_calls_inlined_in_content() {
        // Assistant turns carrying tool_calls are rendered with the calls
        // inlined into the content text (same convention as the Qwen path),
        // so the model reads back a coherent history.
        let asst = AiMessage {
            role: "assistant".to_string(),
            content: "I'll look it up.".to_string(),
            tool_calls: Some(vec![AiToolCall {
                function: crate::ai::provider::AiToolCallFunction {
                    name: "search_emails".to_string(),
                    arguments: serde_json::json!({"q": "x"}),
                },
            }]),
        };
        let out = render_gemma4_chat_template(&[asst], false);
        assert!(out.starts_with("<|turn>model\nI'll look it up.\n<tool_call>"));
        assert!(out.contains("\"name\":\"search_emails\""));
        assert!(out.ends_with("</tool_call><turn|>\n"));
    }

    #[test]
    fn render_gemma4_preserves_newlines_in_content() {
        // Multi-line prompts (the draft-generation case) must keep their
        // internal `\n` characters intact — the model trained on that shape.
        let out = render_gemma4_chat_template(&[msg("user", "line one\nline two")], true);
        assert_eq!(
            out,
            "<|turn>user\nline one\nline two<turn|>\n<|turn>model\n<|channel>thought\n<channel|>"
        );
    }
}
