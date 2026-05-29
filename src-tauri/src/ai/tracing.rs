//! Phoenix (Arize AI) observability connector — OpenTelemetry OTLP → local Phoenix.
//!
//! Enabled only with the `tracing` cargo feature — **never shipped**.
//! All public items compile to no-ops when the feature is absent, so there is
//! zero overhead in production builds.
//!
//! # Quick start
//!
//! 1. Run an OTLP-compatible local tracing backend (for example, Phoenix in Docker).
//!
//! 2. Optionally set `PHOENIX_HOST` in `.env` (defaults to `http://localhost:6006`):
//!    ```text
//!    PHOENIX_HOST=http://localhost:6006
//!    ```
//!
//! 3. Run with the feature enabled:
//!    ```sh
//!    make dev-trace
//!    ```
//!
//! Phoenix UI: http://localhost:6006  (no login required for local)
//!
//! # How it works
//!
//! The driver is initialised lazily on the first call to [`driver()`]. It builds
//! an OTLP HTTP exporter (HTTP/protobuf, blocking reqwest) and registers a
//! `SdkTracerProvider` as the global OTel tracer.
//!
//! ## Single LLM calls
//! `record_generation` creates a flat LLM span with OpenInference attributes.
//!
//! ## Chat turns
//! `record_chat_turn` creates a parent CHAIN span with child spans for each
//! pipeline step. All spans use `SpanBuilder::with_start_time` +
//! `end_with_timestamp` to inject the pre-recorded per-step latencies so the
//! Phoenix timeline shows real durations instead of 0ms.
//!
//! Steps are laid out sequentially (retrieval → tools → llm rounds) with the
//! cursor advancing by each step's measured duration.
//!
//! The exporter sends to `$PHOENIX_HOST/v1/traces`.
//! (`with_endpoint` in opentelemetry-otlp 0.31 does NOT append the path automatically.)

use std::sync::OnceLock;

// ── Global driver ─────────────────────────────────────────────────────────────

static DRIVER: OnceLock<TracingDriver> = OnceLock::new();

/// Return a reference to the global [`TracingDriver`], initialising it on
/// first call. Reads `PHOENIX_HOST` from the environment (default:
/// `http://localhost:6006`). Returns a disabled no-op driver when the feature
/// is not compiled in.
pub fn driver() -> &'static TracingDriver {
    DRIVER.get_or_init(TracingDriver::init_from_env)
}

/// Flush all in-flight spans and shut down the exporter.
/// Call on clean app exit to ensure the last traces are delivered.
pub fn shutdown() {
    if let Some(d) = DRIVER.get() {
        d.flush();
    }
}

// ── Parameter types ────────────────────────────────────────────────────────────

/// Data for one standalone LLM inference call (classify, ai.complete, etc.).
pub struct GenerationParams<'a> {
    pub trace_name: &'a str,
    pub name: &'a str,
    pub model: &'a str,
    pub input: &'a str,
    pub output: &'a str,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub latency_ms: u64,
    pub error: Option<&'a str>,
}

/// One retrieved document for the RAG retriever span. Matches the
/// OpenInference `retrieval.documents.{i}.document.*` convention so Phoenix
/// can render them in its Documents tab.
pub struct RetrievedDocument {
    pub id: String,
    pub score: f32,
    /// Body excerpt (or any text payload) handed to the LLM for this citation.
    pub content: String,
    /// JSON-encoded metadata (subject, sender, timestamp, citation_number…).
    pub metadata_json: String,
}

/// Retrieval stage data for [`ChatTurnTrace`].
pub struct RetrievalInfo {
    pub vector_hits: i32,
    pub fts_hits: i32,
    pub elapsed_ms: i64,
    pub embedding_ms: Option<i64>,
    pub vec_search_ms: Option<i64>,
    pub fts_search_ms: i64,
    pub rerank_ms: Option<i64>,
    pub query_rewrite_ms: Option<i64>,
    pub expanded_query: String,
    pub vector_fallback: bool,
    pub invalid_citations: i32,
    /// Documents returned by the retriever, in citation order. Empty when
    /// retrieval ran but produced no hits.
    pub documents: Vec<RetrievedDocument>,
}

/// One tool execution during a chat turn.
pub struct ToolCallInfo {
    pub name: String,
    pub arguments_json: String,
    pub result_preview: String,
    pub elapsed_ms: i64,
}

/// One LLM round-trip during a chat turn.
pub struct LlmCallInfo {
    /// `"tool_round"` | `"final_stream"`
    pub kind: String,
    /// 0-based round index; -1 for the final stream.
    pub round: i32,
    pub latency_ms: i64,
    pub tool_calls_requested: i32,
    pub failed: bool,
    /// Prompt text — populated for the final stream, empty for tool rounds.
    pub input: Option<String>,
    /// Response text — populated for the final stream, empty for tool rounds.
    pub output: Option<String>,
}

/// Full data for a complete chat turn. [`TracingDriver::record_chat_turn`] builds
/// a CHAIN → {RETRIEVER, TOOL*, LLM*} span hierarchy in Phoenix, with each
/// child span placed at its actual elapsed position so latencies are non-zero.
pub struct ChatTurnTrace {
    pub model: String,
    pub user_question: String,
    pub final_answer: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_ms: u64,
    pub route_mode: String,
    pub retrieval: Option<RetrievalInfo>,
    pub tool_calls: Vec<ToolCallInfo>,
    pub llm_calls: Vec<LlmCallInfo>,
    pub error: Option<String>,
}

// ── Driver (always present, ZST in non-feature builds) ────────────────────────

pub struct TracingDriver {
    #[cfg(feature = "tracing")]
    inner: Option<Inner>,
}

impl TracingDriver {
    fn init_from_env() -> Self {
        Self {
            #[cfg(feature = "tracing")]
            inner: Inner::from_env(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        #[cfg(feature = "tracing")]
        {
            self.inner.is_some()
        }
        #[cfg(not(feature = "tracing"))]
        {
            false
        }
    }

    /// Record one standalone LLM call as a flat LLM span. No-op when disabled.
    #[allow(unused_variables)]
    #[inline]
    pub fn record_generation(&self, params: GenerationParams<'_>) {
        #[cfg(feature = "tracing")]
        if let Some(ref inner) = self.inner {
            inner.record_generation(params);
        }
    }

    /// Record a full chat turn as a CHAIN span with child spans for retrieval,
    /// tool calls, and LLM round-trips. No-op when disabled.
    #[allow(unused_variables)]
    pub fn record_chat_turn(&self, params: ChatTurnTrace) {
        #[cfg(feature = "tracing")]
        if let Some(ref inner) = self.inner {
            inner.record_chat_turn(params);
        }
    }

    fn flush(&self) {
        #[cfg(feature = "tracing")]
        if let Some(ref inner) = self.inner {
            inner.flush();
        }
    }
}

// ── Feature-gated implementation ──────────────────────────────────────────────

#[cfg(feature = "tracing")]
struct Inner {
    provider: opentelemetry_sdk::trace::SdkTracerProvider,
}

#[cfg(feature = "tracing")]
impl Inner {
    fn from_env() -> Option<Self> {
        use opentelemetry::global;
        use opentelemetry_otlp::{Protocol, WithExportConfig};
        use opentelemetry_sdk::trace::SdkTracerProvider;

        // `with_endpoint` in opentelemetry-otlp 0.31 uses the URL verbatim —
        // it does NOT append /v1/traces automatically.
        let host = std::env::var("PHOENIX_HOST").unwrap_or_else(|_| "http://localhost:6006".to_string());
        let endpoint = format!("{}/v1/traces", host.trim_end_matches('/'));

        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(&endpoint)
            .build()
            .map_err(|e| eprintln!("[tracing] Phoenix exporter init failed: {e}"))
            .ok()?;

        let provider = SdkTracerProvider::builder().with_batch_exporter(exporter).build();

        global::set_tracer_provider(provider.clone());
        eprintln!("[tracing] Phoenix tracing active → {endpoint}");

        Some(Inner { provider })
    }

    // ── Single LLM call ──────────────────────────────────────────────────────

    fn record_generation(&self, params: GenerationParams<'_>) {
        use opentelemetry::{
            global,
            trace::{Span, Tracer},
            KeyValue,
        };

        let tracer = global::tracer("emailops");
        let mut span = tracer.start(params.name.to_owned());

        span.set_attribute(KeyValue::new("openinference.span.kind", "LLM"));
        span.set_attribute(KeyValue::new("llm.model_name", params.model.to_owned()));
        span.set_attribute(KeyValue::new("input.value", params.input.to_owned()));
        span.set_attribute(KeyValue::new("output.value", params.output.to_owned()));
        span.set_attribute(KeyValue::new("llm.token_count.prompt", params.prompt_tokens as i64));
        span.set_attribute(KeyValue::new(
            "llm.token_count.completion",
            params.completion_tokens as i64,
        ));
        span.set_attribute(KeyValue::new(
            "llm.token_count.total",
            (params.prompt_tokens + params.completion_tokens) as i64,
        ));
        span.set_attribute(KeyValue::new("latency_ms", params.latency_ms as i64));
        span.set_attribute(KeyValue::new("emailops.operation", params.trace_name.to_owned()));

        if let Some(err) = params.error {
            span.set_status(opentelemetry::trace::Status::Error {
                description: err.to_owned().into(),
            });
            span.set_attribute(KeyValue::new("error.message", err.to_owned()));
        }

        span.end();
    }

    // ── Full chat turn with hierarchy and real timestamps ────────────────────

    fn record_chat_turn(&self, params: ChatTurnTrace) {
        use opentelemetry::{
            global,
            trace::{Span, SpanBuilder, TraceContextExt, Tracer},
            Context, KeyValue,
        };
        use std::time::{Duration, SystemTime};

        let tracer = global::tracer("emailops");

        // Anchor: we know when the turn ended (now) and how long it took.
        let t_now = SystemTime::now();
        let t_turn_start = t_now
            .checked_sub(Duration::from_millis(params.total_ms))
            .unwrap_or(t_now);

        // `cursor_ms` tracks how far through the turn each child span starts.
        // Children are laid out sequentially: retrieval → tools → llm rounds.
        let mut cursor_ms: u64 = 0;

        // ── Root CHAIN span ────────────────────────────────────────────────
        // Start time is set explicitly; end time is recorded automatically when
        // `root_cx` drops (SdkSpan Drop impl calls end() with SystemTime::now(),
        // which is effectively t_now — correct for the full turn duration).
        let mut root = tracer.build(SpanBuilder::from_name("chat_turn").with_start_time(t_turn_start));
        root.set_attribute(KeyValue::new("openinference.span.kind", "CHAIN"));
        root.set_attribute(KeyValue::new("input.value", params.user_question.clone()));
        root.set_attribute(KeyValue::new("output.value", params.final_answer.clone()));
        root.set_attribute(KeyValue::new("llm.model_name", params.model.clone()));
        root.set_attribute(KeyValue::new("llm.token_count.prompt", params.prompt_tokens as i64));
        root.set_attribute(KeyValue::new(
            "llm.token_count.completion",
            params.completion_tokens as i64,
        ));
        root.set_attribute(KeyValue::new("latency_ms", params.total_ms as i64));
        root.set_attribute(KeyValue::new("emailops.route", params.route_mode.clone()));

        if let Some(err) = &params.error {
            root.set_status(opentelemetry::trace::Status::Error {
                description: err.clone().into(),
            });
        }

        // Wrap root in a context so children can reference it as their parent.
        let root_cx = Context::current().with_span(root);

        // ── RAG retrieval child ────────────────────────────────────────────
        if let Some(ret) = &params.retrieval {
            let step_start = t_turn_start + Duration::from_millis(cursor_ms);
            let step_dur = ret.elapsed_ms.max(0) as u64;

            let mut span = tracer.build_with_context(
                SpanBuilder::from_name("rag_retrieval").with_start_time(step_start),
                &root_cx,
            );
            span.set_attribute(KeyValue::new("openinference.span.kind", "RETRIEVER"));
            span.set_attribute(KeyValue::new("input.value", params.user_question.clone()));
            span.set_attribute(KeyValue::new("input.mime_type", "text/plain"));
            span.set_attribute(KeyValue::new("latency_ms", ret.elapsed_ms));
            span.set_attribute(KeyValue::new("retrieval.vector_hits", ret.vector_hits as i64));
            span.set_attribute(KeyValue::new("retrieval.fts_hits", ret.fts_hits as i64));
            span.set_attribute(KeyValue::new("retrieval.documents_count", ret.documents.len() as i64));

            // Per-document attributes — Phoenix's Documents tab keys off these.
            for (i, doc) in ret.documents.iter().enumerate() {
                span.set_attribute(KeyValue::new(
                    format!("retrieval.documents.{i}.document.id"),
                    doc.id.clone(),
                ));
                span.set_attribute(KeyValue::new(
                    format!("retrieval.documents.{i}.document.score"),
                    doc.score as f64,
                ));
                span.set_attribute(KeyValue::new(
                    format!("retrieval.documents.{i}.document.content"),
                    doc.content.clone(),
                ));
                if !doc.metadata_json.is_empty() {
                    span.set_attribute(KeyValue::new(
                        format!("retrieval.documents.{i}.document.metadata"),
                        doc.metadata_json.clone(),
                    ));
                }
            }

            // `output.value` — JSON summary so the Phoenix span list shows a
            // useful snippet next to the retrieval row, not just the raw count.
            // Built from the same documents we just emitted as structured attrs.
            let output_summary: Vec<serde_json::Value> = ret
                .documents
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "id": d.id,
                        "score": d.score,
                        "content": d.content,
                    })
                })
                .collect();
            let output_value = serde_json::to_string(&output_summary).unwrap_or_else(|_| "[]".to_string());
            span.set_attribute(KeyValue::new("output.value", output_value));
            span.set_attribute(KeyValue::new("output.mime_type", "application/json"));
            if let Some(ms) = ret.embedding_ms {
                span.set_attribute(KeyValue::new("retrieval.embedding_ms", ms));
            }
            if let Some(ms) = ret.vec_search_ms {
                span.set_attribute(KeyValue::new("retrieval.vec_search_ms", ms));
            }
            span.set_attribute(KeyValue::new("retrieval.fts_search_ms", ret.fts_search_ms));
            if let Some(ms) = ret.rerank_ms {
                span.set_attribute(KeyValue::new("retrieval.rerank_ms", ms));
            }
            if let Some(ms) = ret.query_rewrite_ms {
                span.set_attribute(KeyValue::new("retrieval.query_rewrite_ms", ms));
            }
            if !ret.expanded_query.is_empty() {
                span.set_attribute(KeyValue::new("retrieval.expanded_query", ret.expanded_query.clone()));
            }
            if ret.vector_fallback {
                span.set_attribute(KeyValue::new("retrieval.vector_fallback", "true".to_string()));
            }
            if ret.invalid_citations >= 0 {
                span.set_attribute(KeyValue::new(
                    "retrieval.invalid_citations",
                    ret.invalid_citations as i64,
                ));
            }
            span.end_with_timestamp(step_start + Duration::from_millis(step_dur));
            cursor_ms += step_dur;
        }

        // ── Tool call children ─────────────────────────────────────────────
        for tool in &params.tool_calls {
            let step_start = t_turn_start + Duration::from_millis(cursor_ms);
            let step_dur = tool.elapsed_ms.max(0) as u64;

            let mut span = tracer.build_with_context(
                SpanBuilder::from_name(format!("tool:{}", tool.name)).with_start_time(step_start),
                &root_cx,
            );
            span.set_attribute(KeyValue::new("openinference.span.kind", "TOOL"));
            span.set_attribute(KeyValue::new("tool.name", tool.name.clone()));
            span.set_attribute(KeyValue::new("input.value", tool.arguments_json.clone()));
            span.set_attribute(KeyValue::new("output.value", tool.result_preview.clone()));
            span.set_attribute(KeyValue::new("latency_ms", tool.elapsed_ms));
            span.end_with_timestamp(step_start + Duration::from_millis(step_dur));
            cursor_ms += step_dur;
        }

        // ── LLM round-trip children ────────────────────────────────────────
        for llm in &params.llm_calls {
            let span_name = if llm.kind == "final_stream" {
                "llm_call:final_stream".to_string()
            } else {
                format!("llm_call:round_{}", llm.round)
            };
            let step_start = t_turn_start + Duration::from_millis(cursor_ms);
            let step_dur = llm.latency_ms.max(0) as u64;

            let mut span =
                tracer.build_with_context(SpanBuilder::from_name(span_name).with_start_time(step_start), &root_cx);
            span.set_attribute(KeyValue::new("openinference.span.kind", "LLM"));
            span.set_attribute(KeyValue::new("llm.model_name", params.model.clone()));
            span.set_attribute(KeyValue::new("latency_ms", llm.latency_ms));
            if llm.kind != "final_stream" {
                span.set_attribute(KeyValue::new("llm.round", llm.round as i64));
                span.set_attribute(KeyValue::new(
                    "llm.tool_calls_requested",
                    llm.tool_calls_requested as i64,
                ));
            }
            if let Some(ref input) = llm.input {
                span.set_attribute(KeyValue::new("input.value", input.clone()));
            }
            if let Some(ref output) = llm.output {
                span.set_attribute(KeyValue::new("output.value", output.clone()));
            }
            if llm.failed {
                span.set_status(opentelemetry::trace::Status::Error {
                    description: "LLM call failed or timed out".into(),
                });
            }
            span.end_with_timestamp(step_start + Duration::from_millis(step_dur));
            cursor_ms += step_dur;
        }

        // root_cx drops here → root span auto-ends with end_time = SystemTime::now() ≈ t_now
    }

    fn flush(&self) {
        if let Err(e) = self.provider.shutdown() {
            eprintln!("[tracing] Phoenix flush error: {e}");
        }
    }
}
