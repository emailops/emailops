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
// TOOL CALLING
// ────────────
// llama-cpp-2 0.1.144+ exposes apply_chat_template_oaicompat which renders the
// model's native Jinja template with a proper tools= parameter and parses the
// response into structured tool_calls — no custom prompt injection needed.
//
// FALLBACK CHAT TEMPLATES
// ────────────────────────
// Some GGUF builds (notably mradermacher's Gemma 4 builds) omit the
// tokenizer.chat_template metadata key.  When chat_template(None) returns None
// we fall back to a built-in template keyed by model family, derived from the
// model filename.  The fallback string is passed to LlamaChatTemplate::new()
// and handed to apply_chat_template_oaicompat / apply_chat_template as if it
// were embedded.

// ── Built-in fallback templates ───────────────────────────────────────────────

/// Official Gemma 4 Jinja chat template (google/gemma-4-E2B-it, gemma-4-E4B-it).
/// Contains '<|tool_call>call:' which llama.cpp uses for Gemma4 format detection.
/// Source: https://huggingface.co/google/gemma-4-E2B-it/resolve/main/chat_template.jinja
const GEMMA4_CHAT_TEMPLATE: &str = r#"{%- macro format_parameters(properties, required) -%}
    {%- set standard_keys = ['description', 'type', 'properties', 'required', 'nullable'] -%}
    {%- set ns = namespace(found_first=false) -%}
    {%- for key, value in properties | dictsort -%}
        {%- set add_comma = false -%}
        {%- if key not in standard_keys -%}
            {%- if ns.found_first %},{% endif -%}
            {%- set ns.found_first = true -%}
            {{ key }}:{
            {%- if value['description'] -%}
                description:<|"|>{{ value['description'] }}<|"|>
                {%- set add_comma = true -%}
            {%- endif -%}
            {%- if value['type'] | upper == 'STRING' -%}
                {%- if value['enum'] -%}
                    {%- if add_comma %},{%- else -%} {%- set add_comma = true -%} {% endif -%}
                    enum:{{ format_argument(value['enum']) }}
                {%- endif -%}
            {%- elif value['type'] | upper == 'ARRAY' -%}
                {%- if value['items'] is mapping and value['items'] -%}
                    {%- if add_comma %},{%- else -%} {%- set add_comma = true -%} {% endif -%}
                    items:{
                    {%- set ns_items = namespace(found_first=false) -%}
                    {%- for item_key, item_value in value['items'] | dictsort -%}
                        {%- if item_value is not none -%}
                            {%- if ns_items.found_first %},{% endif -%}
                            {%- set ns_items.found_first = true -%}
                            {%- if item_key == 'properties' -%}
                                properties:{
                                {%- if item_value is mapping -%}
                                    {{- format_parameters(item_value, value['items']['required'] | default([])) -}}
                                {%- endif -%}
                                }
                            {%- elif item_key == 'required' -%}
                                required:[
                                {%- for req_item in item_value -%}
                                    <|"|>{{- req_item -}}<|"|>
                                    {%- if not loop.last %},{% endif -%}
                                {%- endfor -%}
                                ]
                            {%- elif item_key == 'type' -%}
                                {%- if item_value is string -%}
                                    type:{{ format_argument(item_value | upper) }}
                                {%- else -%}
                                    type:{{ format_argument(item_value | map('upper') | list) }}
                                {%- endif -%}
                            {%- else -%}
                                {{ item_key }}:{{ format_argument(item_value) }}
                            {%- endif -%}
                        {%- endif -%}
                    {%- endfor -%}
                    }
                {%- endif -%}
            {%- endif -%}
            {%- if value['nullable'] %}
                {%- if add_comma %},{%- else -%} {%- set add_comma = true -%} {% endif -%}
                nullable:true
            {%- endif -%}
            {%- if value['type'] | upper == 'OBJECT' -%}
                {%- if value['properties'] is defined and value['properties'] is mapping -%}
                    {%- if add_comma %},{%- else -%} {%- set add_comma = true -%} {% endif -%}
                    properties:{
                    {{- format_parameters(value['properties'], value['required'] | default([])) -}}
                    }
                {%- elif value is mapping -%}
                    {%- if add_comma %},{%- else -%} {%- set add_comma = true -%} {% endif -%}
                    properties:{
                    {{- format_parameters(value, value['required'] | default([])) -}}
                    }
                {%- endif -%}
                {%- if value['required'] -%}
                    {%- if add_comma %},{%- else -%} {%- set add_comma = true -%} {% endif -%}
                    required:[
                    {%- for item in value['required'] | default([]) -%}
                        <|"|>{{- item -}}<|"|>
                        {%- if not loop.last %},{% endif -%}
                    {%- endfor -%}
                    ]
                {%- endif -%}
            {%- endif -%}
            {%- if add_comma %},{%- else -%} {%- set add_comma = true -%} {% endif -%}
            type:<|"|>{{ value['type'] | upper }}<|"|>}
        {%- endif -%}
    {%- endfor -%}
{%- endmacro -%}
{%- macro format_function_declaration(tool_data) -%}
    declaration:{{- tool_data['function']['name'] -}}{description:<|"|>{{- tool_data['function']['description'] -}}<|"|>
    {%- set params = tool_data['function']['parameters'] -%}
    {%- if params -%}
        ,parameters:{
        {%- if params['properties'] -%}
            properties:{ {{- format_parameters(params['properties'], params['required']) -}} },
        {%- endif -%}
        {%- if params['required'] -%}
            required:[
            {%- for item in params['required'] -%}
                <|"|>{{- item -}}<|"|>
                {{- ',' if not loop.last -}}
            {%- endfor -%}
            ],
        {%- endif -%}
        {%- if params['type'] -%}
            type:<|"|>{{- params['type'] | upper -}}<|"|>}
        {%- endif -%}
    {%- endif -%}
    {%- if 'response' in tool_data['function'] -%}
        {%- set response_declaration = tool_data['function']['response'] -%}
        ,response:{
        {%- if response_declaration['description'] -%}
            description:<|"|>{{- response_declaration['description'] -}}<|"|>,
        {%- endif -%}
        {%- if response_declaration['type'] | upper == 'OBJECT' -%}
            type:<|"|>{{- response_declaration['type'] | upper -}}<|"|>}
        {%- endif -%}
    {%- endif -%}
    }
{%- endmacro -%}
{%- macro format_argument(argument, escape_keys=True) -%}
    {%- if argument is string -%}
        {{- '<|"|>' + argument + '<|"|>' -}}
    {%- elif argument is boolean -%}
        {{- 'true' if argument else 'false' -}}
    {%- elif argument is mapping -%}
        {{- '{' -}}
        {%- set ns = namespace(found_first=false) -%}
        {%- for key, value in argument | dictsort -%}
            {%- if ns.found_first %},{% endif -%}
            {%- set ns.found_first = true -%}
            {%- if escape_keys -%}
                {{- '<|"|>' + key + '<|"|>' -}}
            {%- else -%}
                {{- key -}}
            {%- endif -%}
            :{{- format_argument(value, escape_keys=escape_keys) -}}
        {%- endfor -%}
        {{- '}' -}}
    {%- elif argument is sequence -%}
        {{- '[' -}}
        {%- for item in argument -%}
            {{- format_argument(item, escape_keys=escape_keys) -}}
            {%- if not loop.last %},{% endif -%}
        {%- endfor -%}
        {{- ']' -}}
    {%- else -%}
        {{- argument -}}
    {%- endif -%}
{%- endmacro -%}
{%- macro strip_thinking(text) -%}
    {%- set ns = namespace(result='') -%}
    {%- for part in text.split('<channel|>') -%}
        {%- if '<|channel>' in part -%}
            {%- set ns.result = ns.result + part.split('<|channel>')[0] -%}
        {%- else -%}
            {%- set ns.result = ns.result + part -%}
        {%- endif -%}
    {%- endfor -%}
    {{- ns.result | trim -}}
{%- endmacro -%}
{%- macro format_tool_response_block(tool_name, response) -%}
    {{- '<|tool_response>' -}}
    {%- if response is mapping -%}
        {{- 'response:' + tool_name + '{' -}}
        {%- for key, value in response | dictsort -%}
            {{- key -}}:{{- format_argument(value, escape_keys=False) -}}
            {%- if not loop.last %},{% endif -%}
        {%- endfor -%}
        {{- '}' -}}
    {%- else -%}
        {{- 'response:' + tool_name + '{value:' + format_argument(response, escape_keys=False) + '}' -}}
    {%- endif -%}
    {{- '<tool_response|>' -}}
{%- endmacro -%}
{%- set ns = namespace(prev_message_type=None) -%}
{%- set loop_messages = messages -%}
{{- bos_token -}}
{%- if (enable_thinking is defined and enable_thinking) or tools or messages[0]['role'] in ['system', 'developer'] -%}
    {{- '<|turn>system\n' -}}
    {%- if enable_thinking is defined and enable_thinking -%}
        {{- '<|think|>\n' -}}
        {%- set ns.prev_message_type = 'think' -%}
    {%- endif -%}
    {%- if messages[0]['role'] in ['system', 'developer'] -%}
        {{- messages[0]['content'] | trim -}}
        {%- set loop_messages = messages[1:] -%}
    {%- endif -%}
    {%- if tools -%}
        {%- for tool in tools %}
            {{- '<|tool>' -}}
            {{- format_function_declaration(tool) | trim -}}
            {{- '<tool|>' -}}
        {%- endfor %}
        {%- set ns.prev_message_type = 'tool' -%}
    {%- endif -%}
    {{- '<turn|>\n' -}}
{%- endif %}
{%- set ns_turn = namespace(last_user_idx=-1) -%}
{%- for i in range(loop_messages | length) -%}
    {%- if loop_messages[i]['role'] == 'user' -%}
        {%- set ns_turn.last_user_idx = i -%}
    {%- endif -%}
{%- endfor -%}
{%- for message in loop_messages -%}
    {%- if message['role'] != 'tool' -%}
    {%- set ns.prev_message_type = None -%}
    {%- set role = 'model' if message['role'] == 'assistant' else message['role'] -%}
    {%- set prev_nt = namespace(role=None, found=false) -%}
    {%- if loop.index0 > 0 -%}
        {%- for j in range(loop.index0 - 1, -1, -1) -%}
            {%- if not prev_nt.found -%}
                {%- if loop_messages[j]['role'] != 'tool' -%}
                    {%- set prev_nt.role = loop_messages[j]['role'] -%}
                    {%- set prev_nt.found = true -%}
                {%- endif -%}
            {%- endif -%}
        {%- endfor -%}
    {%- endif -%}
    {%- set continue_same_model_turn = (role == 'model' and prev_nt.role == 'assistant') -%}
    {%- if not continue_same_model_turn -%}
        {{- '<|turn>' + role + '\n' }}
    {%- endif -%}
    {%- set thinking_text = message.get('reasoning') or message.get('reasoning_content') -%}
    {%- if thinking_text and loop.index0 > ns_turn.last_user_idx and message.get('tool_calls') -%}
        {{- '<|channel>thought\n' + thinking_text + '\n<channel|>' -}}
    {%- endif -%}
            {%- if message['tool_calls'] -%}
                {%- for tool_call in message['tool_calls'] -%}
                    {%- set function = tool_call['function'] -%}
                    {{- '<|tool_call>call:' + function['name'] + '{' -}}
                    {%- if function['arguments'] is mapping -%}
                        {%- set ns_args = namespace(found_first=false) -%}
                        {%- for key, value in function['arguments'] | dictsort -%}
                            {%- if ns_args.found_first %},{% endif -%}
                            {%- set ns_args.found_first = true -%}
                            {{- key -}}:{{- format_argument(value, escape_keys=False) -}}
                        {%- endfor -%}
                    {%- elif function['arguments'] is string -%}
                        {{- function['arguments'] -}}
                    {%- endif -%}
                    {{- '}<tool_call|>' -}}
                {%- endfor -%}
                {%- set ns.prev_message_type = 'tool_call' -%}
            {%- endif -%}
            {%- set ns_tr_out = namespace(flag=false) -%}
            {%- if message.get('tool_responses') -%}
                {%- for tool_response in message['tool_responses'] -%}
                    {{- format_tool_response_block(tool_response['name'] | default('unknown'), tool_response['response']) -}}
                    {%- set ns_tr_out.flag = true -%}
                    {%- set ns.prev_message_type = 'tool_response' -%}
                {%- endfor -%}
            {%- elif message.get('tool_calls') -%}
                {%- set ns_tool_scan = namespace(stopped=false) -%}
                {%- for k in range(loop.index0 + 1, loop_messages | length) -%}
                    {%- if ns_tool_scan.stopped -%}
                    {%- elif loop_messages[k]['role'] != 'tool' -%}
                        {%- set ns_tool_scan.stopped = true -%}
                    {%- else -%}
                        {%- set follow = loop_messages[k] -%}
                        {%- set ns_tname = namespace(name=follow.get('name') | default('unknown')) -%}
                        {%- for tc in message['tool_calls'] -%}
                            {%- if tc.get('id') == follow.get('tool_call_id') -%}
                                {%- set ns_tname.name = tc['function']['name'] -%}
                            {%- endif -%}
                        {%- endfor -%}
                        {%- set tool_body = follow.get('content') -%}
                        {%- if tool_body is string -%}
                            {{- format_tool_response_block(ns_tname.name, tool_body) -}}
                        {%- elif tool_body is sequence and tool_body is not string -%}
                            {%- set ns_txt = namespace(s='') -%}
                            {%- for part in tool_body -%}
                                {%- if part.get('type') == 'text' -%}
                                    {%- set ns_txt.s = ns_txt.s + (part.get('text') | default('')) -%}
                                {%- endif -%}
                            {%- endfor -%}
                            {{- format_tool_response_block(ns_tname.name, ns_txt.s) -}}
                        {%- else -%}
                            {{- format_tool_response_block(ns_tname.name, tool_body) -}}
                        {%- endif -%}
                        {%- set ns_tr_out.flag = true -%}
                        {%- set ns.prev_message_type = 'tool_response' -%}
                    {%- endif -%}
                {%- endfor -%}
            {%- endif -%}
            {%- if message['content'] is string -%}
                {%- if role == 'model' -%}
                    {{- strip_thinking(message['content']) -}}
                {%- else -%}
                    {{- message['content'] | trim -}}
                {%- endif -%}
            {%- elif message['content'] is sequence -%}
                {%- for item in message['content'] -%}
                    {%- if item['type'] == 'text' -%}
                        {%- if role == 'model' -%}
                            {{- strip_thinking(item['text']) -}}
                        {%- else -%}
                            {{- item['text'] | trim -}}
                        {%- endif -%}
                    {%- elif item['type'] == 'image' -%}
                        {{- '<|image|>' -}}
                        {%- set ns.prev_message_type = 'image' -%}
                    {%- elif item['type'] == 'audio' -%}
                        {{- '<|audio|>' -}}
                        {%- set ns.prev_message_type = 'audio' -%}
                    {%- elif item['type'] == 'video' -%}
                        {{- '<|video|>' -}}
                        {%- set ns.prev_message_type = 'video' -%}
                    {%- endif -%}
                {%- endfor -%}
            {%- endif -%}
        {%- if ns.prev_message_type == 'tool_call' and not ns_tr_out.flag -%}
            {{- '<|tool_response>' -}}
        {%- elif not (ns_tr_out.flag and not message.get('content')) -%}
            {{- '<turn|>\n' -}}
        {%- endif -%}
    {%- endif -%}
{%- endfor -%}
{%- if add_generation_prompt -%}
    {%- if ns.prev_message_type != 'tool_response' and ns.prev_message_type != 'tool_call' -%}
        {{- '<|turn>model\n' -}}
    {%- endif -%}
{%- endif -%}"#;

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{Mutex, Semaphore};

/// How often the idle-eviction background task wakes up to check if loaded
/// models have been unused long enough to drop.
const EVICTION_POLL_INTERVAL_SECS: u64 = 60;

use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, AddBos, LlamaModel},
    openai::OpenAIChatTemplateParams,
};

use super::actor::{InferenceActorHandle, OnToken};
use crate::ai::provider::{
    AiMessage, AiToolCall, AiToolCallFunction, ChatStreamResult, CompletionOptions, ToolStreamResult,
};
use crate::ai::stream_gate::StreamGate;
use crate::ai::thinking_filter::{strip_reasoning, ThinkingGate};
use crate::models::error::{AppError, Result};

// ── Global backend ────────────────────────────────────────────────────────────

static LLAMA_BACKEND: OnceLock<LlamaBackend> = OnceLock::new();

pub(crate) fn backend() -> &'static LlamaBackend {
    LLAMA_BACKEND.get_or_init(|| {
        // `BackendAlreadyInitialized` is not an error in practice — it means
        // the crate's global was initialised by another path (tests, etc.).
        #[allow(unused_mut)]
        let mut b = LlamaBackend::init().unwrap_or_else(|_| {
            // If the backend was already initialised, we still need a proof token.
            // Create a dummy one — the backend itself remains intact.
            // SAFETY: We're in a single-threaded init context; the backend IS
            // initialised at this point.
            // Fail-fast: backend is required for any inference; no recovery possible.
            #[allow(clippy::expect_used)]
            LlamaBackend::init().expect("LlamaBackend unexpectedly unavailable")
        });
        // Silence llama.cpp's chatty stderr in release builds, but keep it
        // in debug builds — when `llama_decode` returns a fatal error it
        // logs the actual reason (Metal command-buffer error, OOM, NaN, etc.)
        // to stderr just before returning. Voiding logs in dev hides those
        // breadcrumbs and leaves only the opaque "Decode Error -3: unknown".
        // Set LLAMA_SILENT=1 to suppress logs in debug builds (e.g. eval runs).
        #[cfg(not(debug_assertions))]
        b.void_logs();
        #[cfg(debug_assertions)]
        if std::env::var("LLAMA_SILENT").as_deref() == Ok("1") {
            b.void_logs();
        }
        b
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
        });
        Self::spawn_eviction_task(&runtime);
        runtime
    }

    /// Override the idle-eviction window. 0 pins the model forever.
    pub fn set_keep_alive_secs(&self, secs: u32) {
        self.keep_alive_secs.store(secs, Ordering::Relaxed);
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
        let actor = InferenceActorHandle::spawn(model).map_err(AppError::AiError)?;
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

    // ── Chat template helpers ─────────────────────────────────────────────────

    /// Returns a built-in fallback Jinja template for models whose GGUF doesn't
    /// embed one, keyed by family name derived from the chat model path.
    fn chat_template_fallback(&self) -> Option<&'static str> {
        let stem = self.chat_model_path.as_ref()?.file_stem()?.to_str()?.to_lowercase();
        if stem.contains("gemma-4") || stem.contains("gemma4") {
            Some(GEMMA4_CHAT_TEMPLATE)
        } else {
            None
        }
    }

    /// Resolve the model's chat template, falling back to a built-in one when
    /// the GGUF omits `tokenizer.chat_template`.
    fn resolve_chat_template(
        model: &LlamaModel,
        fallback: Option<&str>,
    ) -> std::result::Result<llama_cpp_2::model::LlamaChatTemplate, String> {
        match model.chat_template(None) {
            Ok(t) => Ok(t),
            Err(_) => {
                let tmpl = fallback.ok_or_else(|| {
                    "Model has no embedded chat template. Use a GGUF build that \
                     includes tokenizer.chat_template (e.g. bartowski's builds)."
                        .to_string()
                })?;
                llama_cpp_2::model::LlamaChatTemplate::new(tmpl)
                    .map_err(|e| format!("Built-in fallback template is invalid: {}", e))
            }
        }
    }

    // ── Prompt rendering ──────────────────────────────────────────────────────

    /// Render the chat template on a blocking thread.
    ///
    /// Returns the full `ChatTemplateResult` — tool paths need it afterwards
    /// to parse the model's response (`parse_response_oaicompat`).
    ///
    /// We route through apply_chat_template_oaicompat (not the simpler
    /// apply_chat_template) because the latter uses llama.cpp's limited
    /// built-in template renderer which can't handle Gemma 4's complex Jinja
    /// macros → "ffi error -1", and because the oaicompat path can pass
    /// `enable_thinking: false` (Qwen 3's template defaults it to true, which
    /// would prepend a <think> block to every response).
    async fn render_template(
        model: Arc<LlamaModel>,
        fallback_template: Option<&'static str>,
        messages_json: String,
        tools_json: Option<String>,
        parse_tool_calls: bool,
        add_generation_prompt: bool,
    ) -> Result<llama_cpp_2::model::ChatTemplateResult> {
        tokio::task::spawn_blocking(
            move || -> std::result::Result<llama_cpp_2::model::ChatTemplateResult, String> {
                let template = Self::resolve_chat_template(&model, fallback_template)?;

                let params = OpenAIChatTemplateParams {
                    messages_json: &messages_json,
                    tools_json: tools_json.as_deref(),
                    tool_choice: tools_json.as_ref().map(|_| "auto"),
                    json_schema: None,
                    grammar: None,
                    reasoning_format: None,
                    chat_template_kwargs: None,
                    add_generation_prompt,
                    use_jinja: true,
                    parallel_tool_calls: false,
                    enable_thinking: false,
                    // The actor's tokenizer adds BOS via AddBos::Always; llama.cpp
                    // skips duplicate BOS when the rendered prompt already contains it.
                    add_bos: false,
                    add_eos: false,
                    parse_tool_calls,
                };

                model
                    .apply_chat_template_oaicompat(&template, &params)
                    .map_err(|e| format!("apply_chat_template_oaicompat failed: {}", e))
            },
        )
        .await
        .map_err(|e| AppError::AiError(format!("Template render task panicked: {}", e)))?
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
    async fn stable_prompt_bytes(
        model: Arc<LlamaModel>,
        fallback_template: Option<&'static str>,
        messages_json: String,
        tools_json: Option<String>,
        full_prompt: &str,
    ) -> Option<usize> {
        match Self::render_template(model, fallback_template, messages_json, tools_json, false, false).await {
            Ok(hist) if full_prompt.starts_with(&hist.prompt) && hist.prompt.len() < full_prompt.len() => {
                Some(hist.prompt.len())
            }
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
        let messages_json = serde_json::to_string(&serde_json::json!([
            {"role": "user", "content": prompt}
        ]))
        .map_err(|e| AppError::AiError(format!("messages JSON encode: {}", e)))?;

        let fallback_template = self.chat_template_fallback();

        let _permit = Arc::clone(&self.inference_sem)
            .acquire_owned()
            .await
            .map_err(|_| AppError::AiError("Inference semaphore closed".into()))?;

        let tmpl = Self::render_template(model, fallback_template, messages_json, None, false, true).await?;
        let outcome = actor
            // One-shot completion (rewrite/rerank/extraction/warmup): never
            // cached — its prompt would evict the reusable chat prefix.
            .generate(tmpl.prompt, temperature, max_tokens, false, None, None)
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

        let messages_value = normalize_messages_for_oaicompat(&messages);
        let messages_json = serde_json::to_string(&messages_value)
            .map_err(|e| AppError::AiError(format!("messages JSON encode: {}", e)))?;

        let fallback_template = self.chat_template_fallback();

        let _permit = Arc::clone(&self.inference_sem)
            .acquire_owned()
            .await
            .map_err(|_| AppError::AiError("Inference semaphore closed".into()))?;

        let tmpl = Self::render_template(
            Arc::clone(&model),
            fallback_template,
            messages_json.clone(),
            None,
            false,
            true,
        )
        .await?;
        let stable_bytes = Self::stable_prompt_bytes(model, fallback_template, messages_json, None, &tmpl.prompt).await;

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
            .generate(tmpl.prompt, temperature, max_tokens, true, stable_bytes, Some(actor_cb))
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
        })
    }

    /// Non-streaming tool-call round.
    ///
    /// Renders the prompt through llama.cpp's OpenAI-compatible template path,
    /// which activates the model's *native* tool-call formatting (e.g. Llama
    /// 3.1's `<|python_tag|>{…}` or Qwen's `<tool_call>…</tool_call>`), and
    /// parses the response with `parse_response_oaicompat` so each model family
    /// uses its own tool-call grammar without custom prompting.
    pub async fn chat_with_tools(&self, messages: &[AiMessage], tools: &[serde_json::Value]) -> Result<AiMessage> {
        self.touch_last_used();
        let model = self.get_chat_model().await?;
        let actor = self.get_chat_actor().await?;
        let temperature = 0.0f32; // greedy for deterministic tool selection
                                  // 1024 truncated mid `<tool_call>...</tool_call>` on Qwen 4B, causing
                                  // parse_response_oaicompat to fail with ffi error -3. 4096 leaves
                                  // headroom for tool-calls that include thinking traces or wide
                                  // structured schemas (e.g. Lens extraction with many fields).
        let max_tokens = 4096usize;

        // Messages are already in OpenAI shape via AiMessage's serde derive
        // (role / content / tool_calls). Tools come in as OpenAI function-tool
        // JSON from `tool_definitions()`, so we can hand them through directly.
        //
        // llama.cpp's common_chat_msgs_parse_oaicompat validator (chat.cpp:307)
        // requires each tool_call to carry `"type": "function"` and works best
        // when assistant↔tool messages correlate via `id`/`tool_call_id`. Our
        // AiToolCall doesn't carry those, so synthesise them here.
        let messages_value = normalize_messages_for_oaicompat(messages);
        let messages_json = serde_json::to_string(&messages_value)
            .map_err(|e| AppError::AiError(format!("messages JSON encode: {}", e)))?;
        let tools_json_owned = if tools.is_empty() {
            None
        } else {
            Some(serde_json::to_string(tools).map_err(|e| AppError::AiError(format!("tools JSON encode: {}", e)))?)
        };

        let fallback_template = self.chat_template_fallback();

        let _permit = Arc::clone(&self.inference_sem)
            .acquire_owned()
            .await
            .map_err(|_| AppError::AiError("Inference semaphore closed".into()))?;

        let tmpl = Self::render_template(
            Arc::clone(&model),
            fallback_template,
            messages_json.clone(),
            tools_json_owned.clone(),
            true,
            true,
        )
        .await?;
        let stable_bytes =
            Self::stable_prompt_bytes(model, fallback_template, messages_json, tools_json_owned, &tmpl.prompt).await;
        let outcome = actor
            .generate(tmpl.prompt.clone(), temperature, max_tokens, true, stable_bytes, None)
            .await
            .map_err(AppError::AiError)?;

        // If the OAI-compatible parser can't grok the response (e.g. the model
        // emitted a partial `<tool_call>` block, an unexpected wrapper, or
        // plain JSON instead of the templated tool-call syntax), don't fail
        // the whole call — synthesise a content-only assistant message so the
        // downstream caller can apply its own salvage logic (the Lens
        // extractor's `try_parse_json_object`, for instance).
        let parsed_json = match tmpl.parse_response_oaicompat(&outcome.text, false) {
            Ok(parsed) => parsed,
            Err(_) => serde_json::json!({
                "role": "assistant",
                "content": outcome.text,
                "tool_calls": [],
            })
            .to_string(),
        };

        Self::parse_oai_assistant_message(&parsed_json)
    }

    /// Decode an OpenAI-shaped assistant message JSON (as produced by
    /// `parse_response_oaicompat` or the content-only fallback) into an
    /// `AiMessage`. When tool calls are present the content is dropped — the
    /// tool round's job is to dispatch calls, not surface prose.
    ///
    /// Shape:
    ///   {"role":"assistant","content":"…","tool_calls":[{"id":…,"type":"function",
    ///    "function":{"name":"…","arguments":"{…}"}}, …]}
    fn parse_oai_assistant_message(parsed_json: &str) -> Result<AiMessage> {
        let parsed: serde_json::Value = serde_json::from_str(parsed_json)
            .map_err(|e| AppError::AiError(format!("Parsed tool JSON malformed: {}", e)))?;

        let content = strip_reasoning(parsed.get("content").and_then(|v| v.as_str()).unwrap_or(""));

        let tool_calls: Vec<AiToolCall> = parsed
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| {
                        let f = tc.get("function")?;
                        let name = f.get("name")?.as_str()?.to_string();
                        // OpenAI encodes `arguments` as a JSON-encoded string; some
                        // backends return a JSON object directly. Normalise to Value.
                        let args = match f.get("arguments") {
                            Some(serde_json::Value::String(s)) => {
                                serde_json::from_str(s).unwrap_or(serde_json::Value::Object(Default::default()))
                            }
                            Some(v) => v.clone(),
                            None => serde_json::Value::Object(Default::default()),
                        };
                        Some(AiToolCall {
                            function: AiToolCallFunction { name, arguments: args },
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        if tool_calls.is_empty() {
            Ok(AiMessage {
                role: "assistant".to_string(),
                content,
                tool_calls: None,
            })
        } else {
            Ok(AiMessage {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: Some(tool_calls),
            })
        }
    }

    /// Streaming tool-call round. Combines `chat_with_tools` (native tool-call
    /// rendering + parsing) with live token streaming: the model's output is fed
    /// through a [`StreamGate`] so any tool-call syntax is suppressed while prose
    /// is forwarded to `on_token`. After generation the full output is parsed via
    /// `parse_response_oaicompat`, so structured `tool_calls` survive even though
    /// they were never streamed.
    pub async fn chat_stream_with_tools(
        &self,
        messages: &[AiMessage],
        tools: &[serde_json::Value],
        on_token: Box<dyn FnMut(String) -> bool + Send>,
    ) -> Result<ToolStreamResult> {
        self.touch_last_used();
        let model = self.get_chat_model().await?;
        let actor = self.get_chat_actor().await?;
        let temperature = 0.0f32; // greedy for deterministic tool selection
        let max_tokens = 4096usize;

        let messages_value = normalize_messages_for_oaicompat(messages);
        let messages_json = serde_json::to_string(&messages_value)
            .map_err(|e| AppError::AiError(format!("messages JSON encode: {}", e)))?;
        let tools_json_owned = if tools.is_empty() {
            None
        } else {
            Some(serde_json::to_string(tools).map_err(|e| AppError::AiError(format!("tools JSON encode: {}", e)))?)
        };

        let fallback_template = self.chat_template_fallback();

        let _permit = Arc::clone(&self.inference_sem)
            .acquire_owned()
            .await
            .map_err(|_| AppError::AiError("Inference semaphore closed".into()))?;

        let tmpl = Self::render_template(
            Arc::clone(&model),
            fallback_template,
            messages_json.clone(),
            tools_json_owned.clone(),
            true,
            true,
        )
        .await?;
        let stable_bytes =
            Self::stable_prompt_bytes(model, fallback_template, messages_json, tools_json_owned, &tmpl.prompt).await;

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
                tmpl.prompt.clone(),
                temperature,
                max_tokens,
                true,
                stable_bytes,
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

        let parsed_json = match tmpl.parse_response_oaicompat(&outcome.text, false) {
            Ok(p) => p,
            Err(_) => serde_json::json!({
                "role": "assistant",
                "content": &outcome.text,
                "tool_calls": [],
            })
            .to_string(),
        };

        let message = Self::parse_oai_assistant_message(&parsed_json)?;
        Ok(ToolStreamResult {
            message,
            eval_count: Some(outcome.gen_tokens),
            prompt_eval_count: Some(outcome.prompt_tokens),
            prefill_ms: Some(outcome.prefill_ms),
            cached_prompt_tokens: Some(outcome.cached_prompt_tokens),
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

// ── OpenAI-compat message normalization ───────────────────────────────────────

/// Reshape `AiMessage`s into the JSON shape llama.cpp's
/// `common_chat_msgs_parse_oaicompat` expects:
///   • every `tool_calls[*]` carries `"type": "function"` and a stable `"id"`;
///   • `arguments` is emitted as a JSON-encoded string (OpenAI spec);
///   • `role: "tool"` messages receive a matching `"tool_call_id"` so
///     assistant→tool correlation survives the round trip.
///
/// IDs are synthesised as `call_<round>_<idx>` in message order — good enough
/// because we only live within a single `run_tool_loop` invocation.
fn normalize_messages_for_oaicompat(messages: &[AiMessage]) -> serde_json::Value {
    let mut out = Vec::with_capacity(messages.len());
    // IDs produced by the most recent assistant `tool_calls`. Consumed in order
    // by subsequent `role: "tool"` messages so each tool result is bound to
    // the call that requested it.
    let mut pending_ids: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut call_counter: usize = 0;

    for msg in messages {
        let mut obj = serde_json::Map::new();
        obj.insert("role".into(), serde_json::Value::String(msg.role.clone()));

        match msg.role.as_str() {
            "assistant" if msg.tool_calls.as_ref().map(|v| !v.is_empty()).unwrap_or(false) => {
                // Assistant tool-call turn.
                obj.insert("content".into(), serde_json::Value::String(msg.content.clone()));
                let mut calls_out = Vec::new();
                // Infallible by construction: guard above ensures tool_calls is Some(non-empty).
                #[allow(clippy::unwrap_used)]
                for tc in msg.tool_calls.as_ref().unwrap() {
                    let id = format!("call_{}", call_counter);
                    call_counter += 1;
                    pending_ids.push_back(id.clone());

                    // arguments → JSON-encoded string (OpenAI spec).
                    let args_str = serde_json::to_string(&tc.function.arguments).unwrap_or_else(|_| "{}".to_string());

                    calls_out.push(serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": tc.function.name,
                            "arguments": args_str,
                        }
                    }));
                }
                obj.insert("tool_calls".into(), serde_json::Value::Array(calls_out));
            }
            "tool" => {
                // Tool-result turn. Bind to the next pending assistant call id.
                obj.insert("content".into(), serde_json::Value::String(msg.content.clone()));
                if let Some(id) = pending_ids.pop_front() {
                    obj.insert("tool_call_id".into(), serde_json::Value::String(id));
                }
            }
            _ => {
                // Plain system/user/assistant turn.
                obj.insert("content".into(), serde_json::Value::String(msg.content.clone()));
            }
        }

        out.push(serde_json::Value::Object(obj));
    }

    serde_json::Value::Array(out)
}
