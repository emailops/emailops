//! Chat tool registry.
//!
//! The chat's `run_tool_loop` used to know about every tool via two giant
//! match statements in `services/chat.rs` (`tool_definitions()` for the JSON
//! schema array sent to the LLM and `execute_tool()` for the dispatch). This
//! module replaces both with a typed `Tool` trait + a `ToolRegistry` that
//! collects one impl per tool. New tools land as a new file under this
//! directory; nothing else has to change.
//!
//! Two cross-cutting concerns are baked into the registry:
//!
//! 1. **Feature gating.** Each tool advertises `is_available(&db)`; the
//!    registry filters tools whose backing feature is disabled in Settings
//!    (`memory_enabled`, `task_enabled`, `lenses_enabled`,
//!    `ai_drafts_enabled` — see `Database::is_*_enabled` helpers). The LLM
//!    never sees a tool it can't use.
//! 2. **Tool effects.** `execute()` returns `ToolOutput { text, effects }`.
//!    The `text` flows back to the LLM as the tool-result message (same as
//!    today); each `ToolEffect` is dispatched by the chat loop as a
//!    `chat-tool-effect` Tauri event so the frontend can react (e.g. open the
//!    composer after a draft is generated) without adding a separate
//!    "open the composer" tool.

pub mod create_task;
pub mod generate_email_draft;
pub mod get_attachments;
pub mod get_email_body;
pub mod get_lens_data;
pub mod get_thread;
pub mod list_calendar_events;
pub mod list_drafts;
pub mod list_lenses;
pub mod list_open_threads;
pub mod list_pending_tasks;
pub mod memory_search;
pub mod recall_entity;
pub mod remember;
pub mod search_contacts;
pub mod search_emails;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;

use crate::db::Database;

/// Resources every tool execution needs. Passed by reference so a single
/// chat turn can reuse the same context across multiple tool calls without
/// cloning the DB handle.
pub struct ToolCtx<'a> {
    pub db: &'a Arc<Database>,
    pub account_id: &'a str,
    /// Active category filter for this chat turn. Empty = all categories.
    /// Today only `search_emails` consults it.
    pub categories: &'a [String],
}

/// What a tool returns to the chat loop.
///
/// - `text` is what the LLM sees in the tool-result message — same contract
///   as `execute_tool()` returned today. Tools that need to surface an error
///   to the model put a human-readable message here (`"Error: invalid date"`)
///   instead of bubbling a `ToolError`.
/// - `effects` are side-effects the *frontend* should react to after the
///   tool runs. The chat loop dispatches each one as a `chat-tool-effect`
///   Tauri event. The LLM never sees them.
/// - `email_refs` are the email IDs this tool handed back to the LLM —
///   the *structural truth* the model saw. The chat loop aggregates these
///   across every tool call in a turn and persists them on the assistant
///   message, so the frontend can validate any `email://EMAIL_ID` link the
///   LLM emits against an allowlist (drop hallucinations, log a warning,
///   render the rest as clickable pills). Populated by every tool that
///   returns email metadata to the model (search_emails, get_thread,
///   get_email_body, list_drafts, list_open_threads, recall_entity).
/// - `draft_refs` mirror `email_refs` for *draft* ids — populated by
///   tools that touch the saved-drafts table (`generate_email_draft`,
///   `list_drafts`). The frontend uses them to render
///   `[label](draft://DRAFT_ID)` chips that re-open the draft (inline
///   reply for replies, compose tab for new mail). Same allowlist
///   guarantee: ids the tools never returned are dropped + warned.
#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub text: String,
    pub effects: Vec<ToolEffect>,
    pub email_refs: Vec<String>,
    pub draft_refs: Vec<String>,
}

impl ToolOutput {
    /// Convenience: a text-only result with no effects and no refs (the
    /// shape used by tools that don't return email IDs — `create_task`,
    /// `remember`, `memory_search`, etc.).
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            effects: Vec::new(),
            email_refs: Vec::new(),
            draft_refs: Vec::new(),
        }
    }

    /// Convenience for tools that hand a list of emails to the LLM — sets
    /// the text body and the structural ref list in one call. The LLM
    /// sees the text; the chat loop's aggregator sees the refs and uses
    /// them to validate links the LLM later emits.
    pub fn text_with_email_refs(text: impl Into<String>, refs: Vec<String>) -> Self {
        Self {
            text: text.into(),
            effects: Vec::new(),
            email_refs: refs,
            draft_refs: Vec::new(),
        }
    }

    /// Convenience for tools that hand drafts to the LLM (`list_drafts`,
    /// `generate_email_draft`). Same wire as `text_with_email_refs` but
    /// the refs go on the `draft_refs` slot so the dispatcher routes them
    /// to `referenced_draft_ids` on the assistant message.
    pub fn text_with_draft_refs(text: impl Into<String>, drafts: Vec<String>) -> Self {
        Self {
            text: text.into(),
            effects: Vec::new(),
            email_refs: Vec::new(),
            draft_refs: drafts,
        }
    }
}

/// Side-effects a tool can request the frontend perform after a successful
/// run. The chat loop serialises each variant as `{"kind": "...", ...fields}`
/// over the `chat-tool-effect` Tauri event so a single frontend listener
/// switches once on `kind` and routes to the right handler.
///
/// Adding a new effect = add a variant here + handle the new `kind` in the
/// frontend listener; no per-effect plumbing.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ToolEffect {
    /// Open the composer pre-loaded with a draft. Fired by
    /// `generate_email_draft` so the user can review/send without an extra
    /// "open the composer" tool call. Carries the full draft contents
    /// inline so the frontend listener does not need to round-trip back to
    /// the backend to fetch them — it just calls `openComposeTab(...)`.
    ///
    /// `email_id` is `Some` for replies (the id of the inbound email being
    /// replied to) and `None` for brand-new drafts. The frontend uses it to
    /// open the reply inline inside the matching thread — mirroring what
    /// clicking Reply on the thread does — instead of a standalone compose
    /// tab. New-mail drafts continue to open in a standalone tab because
    /// there is no thread to attach to.
    #[serde(rename_all = "camelCase")]
    OpenComposer {
        draft_id: String,
        account_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        email_id: Option<String>,
        to_addresses: Vec<String>,
        subject: String,
        body: String,
    },
}

/// Errors the registry surfaces to the chat loop.
///
/// Tool *content* errors (DB lookup failed, bad date, no results) stay
/// inside `ToolOutput::text` so the LLM can read them — this enum is
/// reserved for the cases where the tool can't even be called.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// LLM requested a tool name we don't know about.
    #[error("unknown tool: {0}")]
    Unknown(String),
    /// LLM requested a tool whose feature is currently disabled in Settings.
    /// Should be rare — gated tools are omitted from `definitions()` so the
    /// LLM normally doesn't even see their name — but a stale conversation
    /// context can still produce a call.
    #[error("tool '{0}' is currently disabled in Settings")]
    Disabled(String),
}

#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique tool name. Matches the `function.name` the LLM sends back.
    fn name(&self) -> &'static str;

    /// One-line tool description shown to the LLM. Be precise — small models
    /// route entirely on this text.
    fn description(&self) -> &'static str;

    /// JSON Schema for the `arguments` object. Returned to the LLM as
    /// `function.parameters`. Build with `serde_json::json!({...})`.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Short summary inlined into the chat system prompt's `Tools:` section.
    /// Default: full `description()`. Override with a tight one-liner —
    /// small models pick tools off this list, so it must be precise but
    /// short. The rich `description()` still flows through
    /// `parameters_schema()` to the LLM's function-calling menu.
    fn prompt_summary(&self) -> &'static str {
        self.description()
    }

    /// Whether this tool should be advertised to the LLM right now. Default:
    /// always available. Override only for tools whose underlying feature
    /// has a Settings toggle. Returns plain `bool` (not `Result<bool>`) so
    /// `ToolRegistry::definitions` is infallible — gating that can't read
    /// the DB fails closed (tool omitted).
    fn is_available(&self, _db: &Database) -> bool {
        true
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: serde_json::Value) -> Result<ToolOutput, ToolError>;
}

/// Collection of all tools the chat can call. Built once at startup
/// (`AppState::tool_registry`) and shared across every chat turn.
pub struct ToolRegistry {
    tools: HashMap<&'static str, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn with_tools(tools: Vec<Arc<dyn Tool>>) -> Self {
        let mut map = HashMap::new();
        for tool in tools {
            map.insert(tool.name(), tool);
        }
        Self { tools: map }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name(), tool);
    }

    /// JSON array of `{type:"function", function:{name, description, parameters}}`
    /// objects in Ollama / OpenAI function-calling format. Filtered by
    /// `is_available(&db)` so the LLM never sees tools whose backing feature
    /// is turned off.
    pub fn definitions(&self, db: &Database) -> Vec<serde_json::Value> {
        let mut defs: Vec<_> = self
            .tools
            .values()
            .filter(|t| t.is_available(db))
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameters_schema(),
                    },
                })
            })
            .collect();
        // Stable ordering — HashMap iteration order is non-deterministic, but
        // some eval harnesses snapshot the definitions array, and a stable
        // order also helps when manually diffing the JSON.
        defs.sort_by_key(|def| {
            def.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string()
        });
        defs
    }

    /// Look up a tool by name, honouring feature gating. Returns `None` if
    /// the tool isn't registered OR its feature flag is off — the chat loop
    /// turns either into a `ToolError` for the LLM.
    pub fn get(&self, name: &str, db: &Database) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name).filter(|t| t.is_available(db))
    }

    /// Distinct from `get()`: returns the tool ignoring the feature gate,
    /// used by the chat loop to distinguish "unknown" from "disabled" so the
    /// user gets a clearer error message.
    pub fn lookup(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// All registered tool names (independent of feature gating). Used by
    /// the chat-turn salvager so a bare `name(args)` line emitted as text
    /// can be recognised as a tool call only when `name` is one we know
    /// about — keeps prose mentions of other identifiers from triggering
    /// phantom dispatches.
    pub fn names(&self) -> Vec<&'static str> {
        self.tools.keys().copied().collect()
    }

    /// Render the chat system prompt's `Tools:` section from the available
    /// tools. Each line follows the format
    ///     `  - tool_name(required, optional?): summary`
    /// derived from `parameters_schema()`. Adding a new tool automatically
    /// updates the prompt — no template edit needed.
    pub fn render_system_prompt_section(&self, db: &Database) -> String {
        let mut entries: Vec<(&'static str, String)> = self
            .tools
            .values()
            .filter(|t| t.is_available(db))
            .map(|t| {
                (
                    t.name(),
                    format!(
                        "  - {}({}): {}",
                        t.name(),
                        tool_arg_signature(&t.parameters_schema()),
                        t.prompt_summary()
                    ),
                )
            })
            .collect();
        entries.sort_by_key(|(name, _)| *name);
        let mut out = String::from("Tools:\n");
        for (_, line) in entries {
            out.push_str(&line);
            out.push('\n');
        }

        // Qwen 3 expects the tool catalogue as a structured `<tools>…</tools>`
        // JSON block. Before the llama-cpp-2 0.1.147 migration this was
        // injected by the Jinja template's `tools=` parameter; the plain
        // `apply_chat_template` API no longer accepts that, so we render it
        // ourselves into the system prompt. Without this, Qwen 3 emits
        // inconsistent tool-call shapes (sometimes `tool_call: name(args)`,
        // sometimes `<tool_call>{…}</tool_call>` with the `name` field
        // dropped) and turns silently fail to produce an answer.
        let defs = self.definitions(db);
        if !defs.is_empty() {
            out.push_str("\n<tools>\n");
            // One JSON object per line — easier for the model to read and
            // matches the format Qwen 3 was trained on (line-delimited JSON
            // inside the wrapper tags).
            for def in &defs {
                if let Ok(s) = serde_json::to_string(def) {
                    out.push_str(&s);
                    out.push('\n');
                }
            }
            out.push_str("</tools>\n");
        }

        out
    }

    /// `(name, property keys, required keys)` per AVAILABLE tool — used to infer
    /// the target of a NAMELESS tool-call block from its argument keys (Qwen 3.6
    /// under no-think batches body reads but drops the function name).
    pub fn arg_key_schemas(&self, db: &Database) -> Vec<(&'static str, Vec<String>, Vec<String>)> {
        self.tools
            .values()
            .filter(|t| t.is_available(db))
            .map(|t| {
                let schema = t.parameters_schema();
                let props = schema
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                let required = schema
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                (t.name(), props, required)
            })
            .collect()
    }
}

/// Derive the `name, name?, …` parenthesised arg list from a JSON Schema
/// object. Properties present in the schema's `required` array are emitted
/// without a `?`; everything else gets one. Property ordering follows the
/// JSON insertion order so tool authors control display order via their
/// schema's `properties` definition.
fn tool_arg_signature(schema: &serde_json::Value) -> String {
    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return String::new(),
    };
    let required: std::collections::HashSet<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let parts: Vec<String> = properties
        .keys()
        .map(|k| {
            if required.contains(k.as_str()) {
                k.clone()
            } else {
                format!("{k}?")
            }
        })
        .collect();
    parts.join(", ")
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a registry pre-loaded with every production chat tool.
///
/// Called once during `AppState` construction; the resulting `Arc<ToolRegistry>`
/// is shared across every chat turn. Adding a new tool = add the file under
/// `services/chat/tools/`, declare the module above, and append it here.
pub fn default_registry() -> ToolRegistry {
    ToolRegistry::with_tools(vec![
        Arc::new(search_contacts::SearchContactsTool),
        Arc::new(search_emails::SearchEmailsTool),
        Arc::new(get_email_body::GetEmailBodyTool),
        Arc::new(get_thread::GetThreadTool),
        Arc::new(get_attachments::GetAttachmentsTool),
        Arc::new(memory_search::MemorySearchTool),
        Arc::new(recall_entity::RecallEntityTool),
        Arc::new(list_calendar_events::ListCalendarEventsTool),
        Arc::new(list_pending_tasks::ListPendingTasksTool),
        Arc::new(list_open_threads::ListOpenThreadsTool),
        Arc::new(remember::RememberTool),
        Arc::new(create_task::CreateTaskTool),
        Arc::new(generate_email_draft::GenerateEmailDraftTool),
        Arc::new(list_drafts::ListDraftsTool),
        Arc::new(list_lenses::ListLensesTool),
        Arc::new(get_lens_data::GetLensDataTool),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    // Tests below (date-window cases for list_pending_tasks etc.) call into
    // the chat module's shared date parser; the helper lives one level up.
    use crate::services::chat::parse_iso_date_secs;

    /// Minimal tool impl used by the registry tests. `available_when` lets
    /// each test wire its own gating predicate without depending on real
    /// pref keys.
    struct FakeTool {
        name: &'static str,
        description: &'static str,
        params: serde_json::Value,
        available_when: fn(&Database) -> bool,
        result_text: String,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &'static str {
            self.name
        }
        fn description(&self) -> &'static str {
            self.description
        }
        fn parameters_schema(&self) -> serde_json::Value {
            self.params.clone()
        }
        fn is_available(&self, db: &Database) -> bool {
            (self.available_when)(db)
        }
        async fn execute(&self, _ctx: &ToolCtx<'_>, _args: serde_json::Value) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text(self.result_text.clone()))
        }
    }

    fn always(_db: &Database) -> bool {
        true
    }

    fn only_when_lenses(db: &Database) -> bool {
        db.is_lenses_enabled().unwrap_or(false)
    }

    fn make_tool(name: &'static str, gate: fn(&Database) -> bool) -> Arc<dyn Tool> {
        Arc::new(FakeTool {
            name,
            description: "fake",
            params: serde_json::json!({"type": "object", "properties": {}}),
            available_when: gate,
            result_text: format!("{} ran", name),
        })
    }

    #[test]
    fn definitions_includes_every_registered_tool_when_all_available() {
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![make_tool("alpha", always), make_tool("beta", always)]);
        let defs = registry.definitions(&db);
        let names: Vec<&str> = defs
            .iter()
            .map(|d| d["function"]["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn definitions_omits_tools_whose_feature_is_disabled() {
        // Lenses is the only tool here, gated on `lenses_enabled` — which
        // defaults to false. Registry should expose zero tools to the LLM.
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![make_tool("get_lens_data", only_when_lenses)]);
        assert!(
            registry.definitions(&db).is_empty(),
            "lenses-gated tool should be hidden when lenses_enabled defaults to false"
        );
    }

    #[test]
    fn definitions_re_includes_tool_after_feature_is_enabled() {
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![make_tool("get_lens_data", only_when_lenses)]);
        db.set_preference("lenses_enabled", "true").expect("set pref");
        let defs = registry.definitions(&db);
        let names: Vec<&str> = defs
            .iter()
            .map(|d| d["function"]["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, vec!["get_lens_data"]);
    }

    #[test]
    fn definitions_use_function_calling_shape() {
        // Guards against accidental schema drift — the LLM provider expects
        // exactly this envelope (Ollama + OpenAI function-calling format).
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![make_tool("alpha", always)]);
        let defs = registry.definitions(&db);
        assert_eq!(defs.len(), 1);
        let def = &defs[0];
        assert_eq!(def["type"], "function");
        assert_eq!(def["function"]["name"], "alpha");
        assert_eq!(def["function"]["description"], "fake");
        assert_eq!(def["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn get_returns_some_for_available_tool() {
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![make_tool("alpha", always)]);
        assert!(registry.get("alpha", &db).is_some());
    }

    #[test]
    fn get_returns_none_for_unknown_tool() {
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![make_tool("alpha", always)]);
        assert!(registry.get("does_not_exist", &db).is_none());
    }

    #[test]
    fn get_returns_none_for_disabled_tool() {
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![make_tool("get_lens_data", only_when_lenses)]);
        assert!(registry.get("get_lens_data", &db).is_none());
    }

    #[test]
    fn lookup_returns_some_for_disabled_tool_so_loop_can_distinguish() {
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![make_tool("get_lens_data", only_when_lenses)]);
        // Distinguishing "unknown" from "disabled" lets the chat loop give
        // the LLM a better error message.
        assert!(registry.get("get_lens_data", &db).is_none());
        assert!(registry.lookup("get_lens_data").is_some());
    }

    #[tokio::test]
    async fn execute_returns_text_in_tool_output() {
        let db = Database::new_for_testing().expect("test db");
        let db = Arc::new(db);
        let categories: Vec<String> = Vec::new();
        let tool = make_tool("alpha", always);
        let ctx = ToolCtx {
            db: &db,
            account_id: "acct",
            categories: &categories,
        };
        let out = tool.execute(&ctx, serde_json::json!({})).await.expect("ok");
        assert_eq!(out.text, "alpha ran");
        assert!(out.effects.is_empty());
    }

    struct FakeToolWithSummary;
    #[async_trait]
    impl Tool for FakeToolWithSummary {
        fn name(&self) -> &'static str {
            "fake_summary"
        }
        fn description(&self) -> &'static str {
            "full long description for the LLM function-calling menu"
        }
        fn prompt_summary(&self) -> &'static str {
            "short summary for the system prompt"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {"q": {"type": "string"}, "limit": {"type": "integer"}},
                "required": ["q"],
            })
        }
        async fn execute(&self, _ctx: &ToolCtx<'_>, _args: serde_json::Value) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text("ran"))
        }
    }

    #[test]
    fn render_system_prompt_section_uses_summary_and_arg_signature() {
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![Arc::new(FakeToolWithSummary)]);
        let section = registry.render_system_prompt_section(&db);
        assert!(section.starts_with("Tools:\n"));

        // Split the rendering into its two distinct regions: the human-readable
        // `Tools:` summary block (kept terse for the model's quick-reference
        // reading) and the structured `<tools>` JSON catalogue (consumed by
        // Qwen 3 as its training-pinned tool-call schema, MUST include the
        // full description for correct tool selection).
        let (summary, structured) = section.split_once("<tools>").unwrap_or((section.as_str(), ""));

        // Required arg has no `?`; optional arg has `?` suffix. Order
        // follows serde_json::Map's default sort, so just assert presence.
        assert!(summary.contains("fake_summary("), "missing tool name: {summary}");
        assert!(summary.contains("limit?"), "missing optional arg: {summary}");
        assert!(
            summary.contains(", q)") || summary.contains("(q,") || summary.contains("(q)"),
            "missing required arg without ?: {summary}"
        );
        // Conversational summary uses the SHORT prompt_summary.
        assert!(
            summary.contains("short summary for the system prompt"),
            "wrong summary; got: {summary}"
        );
        // The full description must NOT leak into the conversational block —
        // that's the original tightness goal of this test.
        assert!(
            !summary.contains("full long description"),
            "leaked description into conversational summary: {summary}"
        );
        // …but the structured <tools> block IS expected to carry it (Qwen
        // selects tools based on the long description, so this is correct).
        assert!(
            structured.contains("full long description"),
            "structured tools block missing description: {structured}"
        );
    }

    #[test]
    fn render_system_prompt_section_omits_gated_tools() {
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![
            Arc::new(FakeToolWithSummary),
            make_tool("hidden", only_when_lenses),
        ]);
        let section = registry.render_system_prompt_section(&db);
        assert!(section.contains("fake_summary"));
        assert!(!section.contains("hidden"), "gated tool leaked: {section}");
    }

    #[test]
    fn render_system_prompt_section_includes_qwen_tools_block() {
        // Qwen 3 was trained to consume a `<tools>…</tools>` JSON block in the
        // system message; without it (lost when llama-cpp-2 0.1.147 dropped the
        // OAI compat layer that used to inject this via the Jinja template) the
        // model emits inconsistent tool-call shapes turn-to-turn. Asserts both:
        //   1. The wrapper tags are present.
        //   2. The JSON shape inside matches the OAI function-tool format
        //      (`{"type":"function","function":{"name":…,…}}`), since that is
        //      the format the model's training pinned.
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![Arc::new(FakeToolWithSummary)]);
        let section = registry.render_system_prompt_section(&db);
        assert!(section.contains("<tools>\n"), "missing opening tag: {section}");
        assert!(section.contains("\n</tools>"), "missing closing tag: {section}");
        // The inner JSON must keep the `{type, function:{name, description, parameters}}`
        // OAI shape — that's what Qwen 3 expects as the tool-call training schema.
        assert!(
            section.contains(r#""type":"function""#),
            "missing function-tool wrapper: {section}"
        );
        assert!(
            section.contains(r#""name":"fake_summary""#),
            "missing tool name in JSON block: {section}"
        );
    }

    #[test]
    fn render_system_prompt_section_omits_tools_block_when_registry_empty() {
        // No tools enabled → no <tools> wrapper (avoids an empty block that
        // would just confuse the model).
        let db = Database::new_for_testing().expect("test db");
        let registry = ToolRegistry::with_tools(vec![]);
        let section = registry.render_system_prompt_section(&db);
        assert!(
            !section.contains("<tools>"),
            "empty registry leaked tools tag: {section}"
        );
        assert!(!section.contains("</tools>"));
    }

    #[test]
    fn tool_arg_signature_handles_no_properties() {
        let schema = serde_json::json!({"type": "object", "properties": {}, "required": []});
        assert_eq!(tool_arg_signature(&schema), "");
    }

    #[test]
    fn tool_arg_signature_marks_optional_args() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"a": {}, "b": {}, "c": {}},
            "required": ["a"],
        });
        let sig = tool_arg_signature(&schema);
        assert!(sig.contains("a"));
        assert!(!sig.contains("a?"));
        assert!(sig.contains("b?"));
        assert!(sig.contains("c?"));
    }

    #[test]
    fn tool_effect_serialises_with_kind_tag() {
        // The frontend's chat-tool-effect listener switches on `kind`, so the
        // tagged JSON shape is load-bearing. Lock it in — including the field
        // names the React side reads off the payload (camelCase).
        let effect = ToolEffect::OpenComposer {
            draft_id: "abc-123".into(),
            account_id: "acc".into(),
            email_id: Some("eml-7".into()),
            to_addresses: vec!["a@x.com".into()],
            subject: "Hi".into(),
            body: "body".into(),
        };
        let v = serde_json::to_value(&effect).expect("serialize");
        assert_eq!(v["kind"], "openComposer");
        assert_eq!(v["draftId"], "abc-123");
        assert_eq!(v["accountId"], "acc");
        // Replies carry the inbound email id so the frontend can route the
        // draft to the inline reply inside that thread instead of opening
        // a standalone compose tab.
        assert_eq!(v["emailId"], "eml-7");
        assert_eq!(v["toAddresses"][0], "a@x.com");
        assert_eq!(v["subject"], "Hi");
        assert_eq!(v["body"], "body");
    }

    #[test]
    fn tool_effect_omits_email_id_for_new_drafts() {
        // New-mail drafts have no thread to attach to. The field is skipped
        // (not emitted as `null`) so the frontend treats absence as "open a
        // standalone compose tab" without needing a null check.
        let effect = ToolEffect::OpenComposer {
            draft_id: "abc-123".into(),
            account_id: "acc".into(),
            email_id: None,
            to_addresses: vec!["a@x.com".into()],
            subject: "Hi".into(),
            body: "body".into(),
        };
        let v = serde_json::to_value(&effect).expect("serialize");
        assert!(v.get("emailId").is_none(), "emailId must be omitted, got: {v}");
    }

    // ── Tool dispatch (integration) ─────────────────────────────────────
    //
    // These tests exercise `execute_tool` end-to-end against an in-memory
    // SQLite DB. They do NOT call Ollama — each test invokes `execute_tool`
    // with the same JSON arguments an LLM would produce, so we verify:
    //   - the chat tool contract (arg names, required filters, date parsing);
    //   - the DB methods backing each tool;
    //   - the string format returned to the model (which the report also shows).

    // `Database`/`Arc` come from the module-level `use super::*;`.
    use crate::services::chat::turn::execute_tool;
    use rusqlite::params;

    /// Build a test DB with the extra tables the chat tools need beyond what
    /// `Database::new_for_testing()` provides (email_bodies / attachments /
    /// email_attachment_meta). The production schema is the source of truth;
    /// these CREATE statements mirror it closely enough to exercise the tools.
    fn tools_test_db() -> Arc<Database> {
        let db = Database::new_for_testing().expect("test DB");
        db.connection()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS email_bodies (
                     email_id TEXT PRIMARY KEY,
                     body TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS email_attachment_meta (
                     id TEXT PRIMARY KEY,
                     email_id TEXT NOT NULL,
                     account_id TEXT NOT NULL,
                     provider_attachment_id TEXT NOT NULL DEFAULT '',
                     filename TEXT NOT NULL,
                     mime_type TEXT NOT NULL,
                     file_size INTEGER NOT NULL DEFAULT 0,
                     file_path TEXT
                 );
                 CREATE TABLE IF NOT EXISTS attachments (
                     id TEXT PRIMARY KEY,
                     account_id TEXT NOT NULL,
                     email_id TEXT NOT NULL,
                     rule_id TEXT NOT NULL,
                     gmail_attachment_id TEXT NOT NULL,
                     filename TEXT NOT NULL,
                     mime_type TEXT NOT NULL,
                     file_size INTEGER NOT NULL,
                     file_path TEXT NOT NULL,
                     tags_json TEXT NOT NULL DEFAULT '[]',
                     sender_email TEXT NOT NULL,
                     subject TEXT NOT NULL,
                     email_timestamp INTEGER NOT NULL,
                     created_at INTEGER NOT NULL
                 );",
            )
            .expect("extra tables");
        Arc::new(db)
    }

    /// Insert a full email row (and its FTS + body side-tables) for tool tests.
    #[allow(clippy::too_many_arguments)]
    fn seed_email(
        db: &Database,
        id: &str,
        account: &str,
        thread_id: &str,
        sender: &str,
        sender_email: &str,
        subject: &str,
        body: &str,
        timestamp: i64,
    ) {
        let sender_domain = sender_email
            .rsplit_once('@')
            .map(|(_, d)| d.to_lowercase())
            .unwrap_or_default();
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails
             (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
              recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'[]','[]','snip',?8,0,'primary',0)",
            params![
                id,
                account,
                thread_id,
                subject,
                sender,
                sender_email,
                sender_domain,
                timestamp,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1,?2,?3,?4)",
            params![id, subject, sender, body],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO email_bodies(email_id, body) VALUES (?1,?2)",
            params![id, body],
        )
        .unwrap();
    }

    /// Variant of `seed_email` that lets a test pick the email's category,
    /// used to exercise the Primary/Updates/Other grouping in search results.
    #[allow(clippy::too_many_arguments)]
    fn seed_email_with_category(
        db: &Database,
        id: &str,
        account: &str,
        thread_id: &str,
        sender: &str,
        sender_email: &str,
        subject: &str,
        body: &str,
        timestamp: i64,
        category: &str,
    ) {
        let sender_domain = sender_email
            .rsplit_once('@')
            .map(|(_, d)| d.to_lowercase())
            .unwrap_or_default();
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails
             (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
              recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'[]','[]','snip',?8,0,?9,0)",
            params![
                id,
                account,
                thread_id,
                subject,
                sender,
                sender_email,
                sender_domain,
                timestamp,
                category,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1,?2,?3,?4)",
            params![id, subject, sender, body],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO email_bodies(email_id, body) VALUES (?1,?2)",
            params![id, body],
        )
        .unwrap();
    }

    fn arg(json: serde_json::Value) -> serde_json::Value {
        json
    }

    // ── search_contacts ─────────────────────────────────────────────────

    #[test]
    fn search_contacts_resolves_name_plus_domain() {
        let db = tools_test_db();
        seed_email(
            &db,
            "e1",
            "acc",
            "t1",
            "Alice Smith",
            "alice.smith@emailops.com",
            "hola",
            "body",
            100,
        );
        seed_email(
            &db,
            "e2",
            "acc",
            "t2",
            "Alice Jones",
            "alice@other.com",
            "hi",
            "body",
            200,
        );
        seed_email(&db, "e3", "acc", "t3", "Ana", "ana@emailops.com", "hi", "body", 300);

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_contacts",
            &arg(serde_json::json!({ "query": "alice de emailops" })),
        );
        // Must surface alice.smith@emailops.com (name="alice", domain="emailops")
        // and NOT alice@other.com (no emailops) or ana@emailops.com (no alice).
        assert!(out.contains("alice.smith@emailops.com"), "output was: {}", out);
        assert!(!out.contains("alice@other.com"), "output was: {}", out);
        assert!(!out.contains("ana@emailops.com"), "output was: {}", out);
    }

    #[test]
    fn search_contacts_ignores_stop_words_and_punctuation() {
        let db = tools_test_db();
        seed_email(
            &db,
            "e1",
            "acc",
            "t1",
            "Maria Dolores",
            "mdolores@acme.com",
            "s",
            "b",
            1,
        );
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_contacts",
            &arg(serde_json::json!({ "query": "de la maria, dolores!" })),
        );
        assert!(out.contains("mdolores@acme.com"), "output was: {}", out);
    }

    #[test]
    fn search_contacts_empty_query_errors() {
        let db = tools_test_db();
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_contacts",
            &arg(serde_json::json!({ "query": "  " })),
        );
        assert!(out.starts_with("Error:"), "expected error, got: {}", out);
    }

    #[test]
    fn search_contacts_no_match_returns_explanatory_string() {
        let db = tools_test_db();
        seed_email(&db, "e1", "acc", "t1", "Alice", "alice@example.com", "s", "b", 1);
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_contacts",
            &arg(serde_json::json!({ "query": "nonexistent_person_xyz" })),
        );
        assert!(out.contains("No contacts matched"), "output was: {}", out);
    }

    // ── search_emails ───────────────────────────────────────────────────

    #[test]
    fn search_emails_requires_at_least_one_filter() {
        let db = tools_test_db();
        let out = execute_tool(&db, "acc", &[], "search_emails", &arg(serde_json::json!({})));
        assert!(out.starts_with("Error:"), "expected error, got: {}", out);
    }

    #[test]
    fn search_emails_by_from_filter_finds_sender() {
        let db = tools_test_db();
        seed_email(
            &db,
            "e1",
            "acc",
            "t1",
            "Alice Smith",
            "alice.smith@emailops.com",
            "contrato",
            "cuerpo del email",
            100,
        );
        seed_email(
            &db,
            "e2",
            "acc",
            "t2",
            "Other Person",
            "other@other.com",
            "spam",
            "body",
            200,
        );

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "from": "alice.smith@emailops.com", "limit": 5 })),
        );
        assert!(out.contains("id=e1"), "output was: {}", out);
        assert!(!out.contains("id=e2"), "output was: {}", out);
    }

    /// Regression: `search_emails(from=alice)` used to return the latest email
    /// in any thread containing a alice message — which for a reply chain is the
    /// user's own reply TO alice, not alice's email. The tool must return the
    /// email actually FROM alice (the matching email) even when the reply is
    /// newer in the same thread.
    #[test]
    fn search_emails_from_filter_returns_matching_email_not_reply() {
        let db = tools_test_db();
        // Same thread: alice wrote first (t=100), user replied later (t=200).
        seed_email(
            &db,
            "alice_msg",
            "acc",
            "thread-1",
            "Alice Smith",
            "alice.smith@emailops.com",
            "contrato firma",
            "hola, aqui el contrato",
            100,
        );
        seed_email(
            &db,
            "user_reply",
            "acc",
            "thread-1",
            "Me",
            "me@mine.com",
            "Re: contrato firma",
            "gracias, lo reviso",
            200,
        );

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "from": "alice.smith@emailops.com", "limit": 5 })),
        );
        assert!(
            out.contains("id=alice_msg"),
            "expected alice's email to be returned; output was: {}",
            out
        );
        assert!(
            !out.contains("id=user_reply"),
            "must NOT return the user's reply (not from alice); output was: {}",
            out
        );
    }

    /// Same regression but exercised through the general (no-from_match) path
    /// using a `query` search. Guarantees the fix also covers the `query`-only
    /// code path, not just the sender-indexed fast path.
    #[test]
    fn search_emails_query_returns_matching_email_not_reply() {
        let db = tools_test_db();
        seed_email(
            &db,
            "match_msg",
            "acc",
            "thread-q",
            "Alice",
            "alice@emailops.com",
            "quarterly_keyword_marker",
            "body with keyword unique_body_token_xyz",
            100,
        );
        seed_email(
            &db,
            "later_reply",
            "acc",
            "thread-q",
            "Me",
            "me@mine.com",
            "Re: unrelated",
            "reply text without the marker",
            200,
        );

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "query": "unique_body_token_xyz", "limit": 5 })),
        );
        assert!(
            out.contains("id=match_msg"),
            "expected the matching email; output was: {}",
            out
        );
        assert!(
            !out.contains("id=later_reply"),
            "must NOT return the later non-matching reply; output was: {}",
            out
        );
    }

    /// Regression: an explicit `from:` lookup must return that sender's NEWEST
    /// email regardless of Gmail category. A chat turn scopes retrieval to
    /// `categories=["primary"]` by default — that scope is meant for broad RAG
    /// queries, not for an explicit sender lookup. A newsletter sender whose
    /// newest mail lands in `updates` was wrongly hidden, so `from:X limit:1`
    /// returned an older `primary` email instead of the genuinely latest one.
    #[test]
    fn search_emails_from_filter_ignores_category_scope() {
        let db = tools_test_db();
        // Older email from the sender, category=primary.
        seed_email_with_category(
            &db,
            "old_primary",
            "acc",
            "t-old",
            "Team Hackers",
            "teamhackers@substack.com",
            "Older issue",
            "body",
            100,
            "primary",
        );
        // Newer email from the same sender, category=updates (the real latest).
        seed_email_with_category(
            &db,
            "new_updates",
            "acc",
            "t-new",
            "Team Hackers",
            "teamhackers@substack.com",
            "Latest issue",
            "body",
            200,
            "updates",
        );

        // Simulate the chat turn default, which scopes retrieval to primary only.
        let categories = vec!["primary".to_string()];
        let out = execute_tool(
            &db,
            "acc",
            &categories,
            "search_emails",
            &arg(serde_json::json!({ "from": "teamhackers@substack.com", "limit": 1 })),
        );
        assert!(
            out.contains("id=new_updates"),
            "explicit from: must return the newest email regardless of category; out:\n{}",
            out
        );
        assert!(
            !out.contains("id=old_primary"),
            "must not fall back to the older primary email; out:\n{}",
            out
        );
    }

    /// `search_emails` output must be grouped by Gmail category in the order
    /// Primary → Updates → Other, with the `category=` field emitted on every
    /// row so the LLM can reference it when summarising.
    #[test]
    fn search_emails_groups_results_by_category_priority() {
        let db = tools_test_db();
        let t = parse_iso_date_secs("2026-04-17").unwrap();

        // Seed one email per category on the same day, all matching the sender.
        seed_email_with_category(
            &db,
            "prom1",
            "acc",
            "tp",
            "Sale",
            "x@x.com",
            "50% off",
            "spam",
            t + 100,
            "promotions",
        );
        seed_email_with_category(
            &db,
            "upd1",
            "acc",
            "tu",
            "Shipping",
            "x@x.com",
            "Your order",
            "tracking",
            t + 200,
            "updates",
        );
        seed_email_with_category(
            &db,
            "prim1",
            "acc",
            "tpr",
            "Real Person",
            "x@x.com",
            "Hey",
            "hello",
            t + 300,
            "primary",
        );
        seed_email_with_category(
            &db,
            "soc1",
            "acc",
            "ts",
            "LinkedIn",
            "x@x.com",
            "Connection",
            "request",
            t + 400,
            "social",
        );

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({
                "from": "x@x.com",
                "since": "2026-04-17",
                "until": "2026-04-18",
                "limit": 25,
            })),
        );

        // Each section header is present when it has entries.
        let p_pos = out.find("## Primary").expect("missing Primary header");
        let u_pos = out.find("## Updates").expect("missing Updates header");
        let o_pos = out.find("## Other").expect("missing Other header");

        // Order: Primary → Updates → Other.
        assert!(p_pos < u_pos, "Primary must come before Updates; out:\n{}", out);
        assert!(u_pos < o_pos, "Updates must come before Other; out:\n{}", out);

        // Primary / Updates rows omit the redundant `category=` field — the
        // section header already conveys it and the field was pure token noise.
        assert!(
            !out.contains("category=primary"),
            "category= is redundant under `## Primary`; out:\n{}",
            out
        );
        assert!(
            !out.contains("category=updates"),
            "category= is redundant under `## Updates`; out:\n{}",
            out
        );
        // "Other" rows keep `category=` because the bucket lumps
        // social/promotions/forums together — without it the LLM can't tell
        // them apart.
        assert!(
            out.contains("category=social") || out.contains("category=promotions"),
            "'Other' rows must still expose their real category; out:\n{}",
            out
        );

        // Rows land under the correct header. A promotions/social row must
        // appear after the Other header, not inside Primary.
        let prim_section = &out[p_pos..u_pos];
        let other_section = &out[o_pos..];
        assert!(
            prim_section.contains("id=prim1"),
            "primary row must appear under Primary; primary section was:\n{}",
            prim_section
        );
        assert!(
            !prim_section.contains("id=prom1") && !prim_section.contains("id=soc1"),
            "Other-category rows must not appear under Primary; got:\n{}",
            prim_section
        );
        assert!(
            other_section.contains("id=prom1") && other_section.contains("id=soc1"),
            "promotions + social must appear under Other; got:\n{}",
            other_section
        );
    }

    /// Empty sections should not be emitted — no "## Other\n" with zero rows.
    #[test]
    fn search_emails_skips_empty_category_sections() {
        let db = tools_test_db();
        seed_email(&db, "only_primary", "acc", "t1", "A", "a@x.com", "subj", "body", 100);

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "from": "a@x.com", "limit": 5 })),
        );
        assert!(out.contains("## Primary"), "out:\n{}", out);
        assert!(!out.contains("## Updates"), "empty Updates section leaked:\n{}", out);
        assert!(!out.contains("## Other"), "empty Other section leaked:\n{}", out);
    }

    #[test]
    fn search_emails_date_bounds_filter_by_timestamp() {
        let db = tools_test_db();
        // Derive timestamps from the parser itself so this test remains honest
        // regardless of leap-year / calendar arithmetic.
        let t_16 = parse_iso_date_secs("2026-04-16").unwrap(); // 00:00 UTC on 16th
        let t_17 = parse_iso_date_secs("2026-04-17").unwrap();
        let t_18 = parse_iso_date_secs("2026-04-18").unwrap();
        seed_email(
            &db,
            "before_day",
            "acc",
            "t1",
            "A",
            "a@x.com",
            "yesterday",
            "body",
            t_16 + 43200,
        );
        seed_email(
            &db,
            "on_day",
            "acc",
            "t2",
            "A",
            "a@x.com",
            "today",
            "body",
            t_17 + 43200,
        );
        seed_email(
            &db,
            "after_day",
            "acc",
            "t3",
            "A",
            "a@x.com",
            "tomorrow",
            "body",
            t_18 + 43200,
        );

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({
                "from": "a@x.com",
                "since": "2026-04-17",
                "until": "2026-04-18",
                "limit": 10,
            })),
        );
        assert!(out.contains("id=on_day"), "output was: {}", out);
        assert!(!out.contains("id=before_day"), "output was: {}", out);
        assert!(!out.contains("id=after_day"), "output was: {}", out);
    }

    /// Exposing `to` lets the model answer "enviada a emailops" style queries
    /// instead of dropping the recipient constraint and returning random
    /// invoices.
    #[test]
    fn search_emails_by_to_filter_matches_recipient() {
        let db = tools_test_db();
        // Seed two invoices: one sent to Emailops, one to someone else.
        // Insert directly so we can populate recipients_json.
        let t = parse_iso_date_secs("2026-04-17").unwrap();
        db.connection()
            .execute(
                "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acc', 'gmail', 'acc', 'Test', 0)",
                [],
            )
            .unwrap();
        db.connection()
            .execute(
                "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                 VALUES ('to_emailops','acc','t1','factura emailops','Me','me@mine.com','mine.com',
                         '[\"billing@emailops.com\"]','[]','snip',?1,0,'primary',0)",
                params![t + 100],
            )
            .unwrap();
        db.connection()
            .execute(
                "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                 VALUES ('to_other','acc','t2','factura other','Me','me@mine.com','mine.com',
                         '[\"billing@other.com\"]','[]','snip',?1,0,'primary',0)",
                params![t + 200],
            )
            .unwrap();
        // emails_fts rows so the FTS virtual table stays consistent — not
        // strictly needed for this test but avoids surprising test drift.
        db.connection()
            .execute(
                "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES
                 ('to_emailops','factura emailops','Me','b'),
                 ('to_other','factura other','Me','b')",
                [],
            )
            .unwrap();

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "to": "emailops.com", "limit": 5 })),
        );
        assert!(out.contains("id=to_emailops"), "expected emailops row; out:\n{}", out);
        assert!(!out.contains("id=to_other"), "other recipient leaked; out:\n{}", out);
    }

    /// Pragmatic retry: when the model over-constrains with a date window
    /// that produces zero hits, the tool should retry without since/until
    /// and annotate the fallback. This mirrors the real failure we saw on
    /// "la última factura de emailops" → the model added since=today and
    /// the tool returned empty, causing the model to give up.
    #[test]
    fn search_emails_zero_result_retries_without_date_window() {
        let db = tools_test_db();
        // Seed an older invoice that will NOT match since=today but matches
        // the rest of the filter.
        let today = parse_iso_date_secs("2026-04-17").unwrap();
        let old = today - 30 * 86_400; // 30 days earlier
        seed_email(
            &db,
            "old_invoice",
            "acc",
            "t1",
            "Emailops",
            "billing@emailops.com",
            "factura 2026",
            "body",
            old + 3600,
        );

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({
                "from": "billing@emailops.com",
                "since": "2026-04-17",
                "limit": 5,
            })),
        );
        // Retry must kick in AND surface the older invoice.
        assert!(
            out.contains("no matches in the requested date window"),
            "expected fallback annotation; out:\n{}",
            out
        );
        assert!(
            out.contains("id=old_invoice"),
            "older invoice must appear via retry; out:\n{}",
            out
        );
    }

    /// Regression: when the user/model asks for "emails from today" with
    /// `{"since":"2026-04-22"}` and the mailbox only has yesterday's mail,
    /// the tool must NOT silently widen the window and return yesterday's
    /// emails as if they matched — that's indistinguishable from a genuine
    /// hit and misleads the chat model into answering "here are today's
    /// emails" when there are none. The correct behavior is to respect the
    /// explicit date window and return empty.
    ///
    /// The fallback-retry is only legitimate when there is a non-date
    /// anchor (from/to/subject/query) that makes "widen the window"
    /// semantically meaningful.
    #[test]
    fn search_emails_bare_since_does_not_fall_back_to_older_emails() {
        let db = tools_test_db();
        // Seed two emails from yesterday — they must NOT appear when the
        // caller asks for `since=today`.
        let today = parse_iso_date_secs("2026-04-22").unwrap();
        let yesterday = today - 86_400;
        seed_email(
            &db,
            "yesterday_1",
            "acc",
            "t1",
            "Alberto",
            "alberto@example.com",
            "Coordinación temas IA",
            "body",
            yesterday + 3600,
        );
        seed_email(
            &db,
            "yesterday_2",
            "acc",
            "t2",
            "Alice",
            "alice@example.com",
            "Weekly Alice - User",
            "body",
            yesterday + 7200,
        );

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({
                "since": "2026-04-22",
                "limit": 20,
            })),
        );

        assert!(
            !out.contains("no matches in the requested date window"),
            "must not surface the 'widen window' annotation when only a date was given; out:\n{}",
            out
        );
        assert!(
            !out.contains("id=yesterday_1") && !out.contains("id=yesterday_2"),
            "yesterday's emails leaked into a 'since=today' query; out:\n{}",
            out
        );
        assert!(
            out.contains("No matching emails found"),
            "expected explicit 'no match' response; out:\n{}",
            out
        );
    }

    /// Regression: search_emails(since=today) was returning ONE email per thread
    /// instead of all individual emails. A thread with 3 messages from today
    /// produced 1 result — the user saw N emails in their inbox but only 1 in
    /// the chat answer.
    ///
    /// Fix: when there are no text filters (no query/from/to/subject/tag), skip
    /// thread deduplication and return every matching email up to the limit.
    #[test]
    fn search_emails_since_returns_all_emails_not_just_latest_per_thread() {
        let db = tools_test_db();
        let today = parse_iso_date_secs("2026-04-22").unwrap();

        // One thread with 3 emails from today (a real multi-reply thread)
        seed_email(
            &db,
            "reply_1",
            "acc",
            "thread_apple",
            "Alice",
            "alice@apple.com",
            "Hola",
            "body",
            today + 1000,
        );
        seed_email(
            &db,
            "reply_2",
            "acc",
            "thread_apple",
            "Bob",
            "bob@apple.com",
            "Re: Hola",
            "body",
            today + 2000,
        );
        seed_email(
            &db,
            "reply_3",
            "acc",
            "thread_apple",
            "Carol",
            "carol@apple.com",
            "Re: Hola",
            "body",
            today + 3000,
        );

        // A separate single-message thread from today
        seed_email(
            &db,
            "standalone",
            "acc",
            "thread_crack",
            "Dave",
            "dave@crack.com",
            "Invoice",
            "body",
            today + 500,
        );

        // An email from yesterday — must NOT appear
        seed_email(
            &db,
            "old",
            "acc",
            "thread_old",
            "Eve",
            "eve@old.com",
            "Old",
            "body",
            today - 3600,
        );

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "since": "2026-04-22", "limit": 20 })),
        );

        // All 4 emails from today must be returned (thread dedup must NOT apply)
        assert!(out.contains("id=reply_1"), "reply_1 missing; out:\n{}", out);
        assert!(out.contains("id=reply_2"), "reply_2 missing; out:\n{}", out);
        assert!(out.contains("id=reply_3"), "reply_3 missing; out:\n{}", out);
        assert!(out.contains("id=standalone"), "standalone missing; out:\n{}", out);
        // Yesterday's email must not appear
        assert!(!out.contains("id=old"), "yesterday email leaked; out:\n{}", out);
    }

    /// When the retry ALSO returns nothing, the tool must say so explicitly
    /// rather than leaving the model to guess whether there is data at all.
    #[test]
    fn search_emails_zero_result_retry_still_empty_reports_both_attempts() {
        let db = tools_test_db();
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({
                "from": "nobody@example.com",
                "since": "2026-04-17",
                "limit": 5,
            })),
        );
        assert!(
            out.contains("also tried without the date window"),
            "expected explicit 'tried without window' wording; out:\n{}",
            out
        );
    }

    /// Without a date window we must NOT retry — preserves the old
    /// "No matching emails found." contract and avoids an extra query on
    /// every legitimate empty result.
    #[test]
    fn search_emails_zero_result_no_retry_when_no_date_window() {
        let db = tools_test_db();
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "from": "nobody@example.com", "limit": 5 })),
        );
        assert_eq!(out, "No matching emails found.");
    }

    /// The summary shortcuts preseed `include_bodies:true` so the model can
    /// summarise emails in one pass instead of chaining a `get_email_body`
    /// call per result (which a weak local model often emits as raw markup).
    /// With the flag set, the tool output must inline each email's body
    /// (excerpt) — not just the snippet.
    #[test]
    fn search_emails_with_include_bodies_emits_body() {
        let db = tools_test_db();
        // Distinctive body token that is NOT in the snippet ("snip").
        let body = format!("INTRO line.\n{}\nSIGNOFF.", "RESUMEN_BODY_TOKEN ".repeat(30));
        seed_email(&db, "e1", "acc", "t1", "Alice", "a@x.com", "subj", &body, 100);
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "from": "a@x.com", "include_bodies": true, "limit": 5 })),
        );
        assert!(
            out.contains("RESUMEN_BODY_TOKEN"),
            "body must be inlined when include_bodies=true; out:\n{}",
            out
        );
    }

    /// Regression: preseeding bodies used the full-body cap (8000 chars) per
    /// row, so one long newsletter ate the whole context and the model
    /// summarised only that email (ignoring the table-across-all-emails
    /// request). Bodies are now fair-shared from a tight budget — a very long
    /// email is excerpted, never inlined whole.
    #[test]
    fn search_emails_with_include_bodies_caps_a_long_email() {
        let db = tools_test_db();
        // 8500-char body with a sentinel near the very end. Under the old
        // 8000-cap it would be inlined nearly whole and the sentinel would show.
        let long_body = format!("HEAD_TOKEN {} TAIL_SENTINEL", "x ".repeat(4200));
        seed_email(&db, "e1", "acc", "t1", "Alice", "a@x.com", "subj", &long_body, 100);
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "from": "a@x.com", "include_bodies": true, "limit": 5 })),
        );
        assert!(
            out.contains("HEAD_TOKEN"),
            "excerpt should keep the start; out:\n{}",
            out
        );
        assert!(
            !out.contains("TAIL_SENTINEL"),
            "long body must be capped, not inlined whole; out len {}",
            out.chars().count()
        );
    }

    /// Default search (no include_bodies) keeps the lean snippet-only output —
    /// the body must NOT leak in, so normal searches stay token-cheap.
    #[test]
    fn search_emails_without_include_bodies_omits_body() {
        let db = tools_test_db();
        let body = "UNIQUE_BODY_TOKEN should not appear in default search output";
        seed_email(&db, "e1", "acc", "t1", "Alice", "a@x.com", "subj", body, 100);
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "from": "a@x.com", "limit": 5 })),
        );
        assert!(
            !out.contains("UNIQUE_BODY_TOKEN"),
            "body must NOT be included without include_bodies; out:\n{}",
            out
        );
    }

    /// `include_bodies` is an internal, heuristic-only arg — it must never be
    /// advertised to the LLM in the tool's JSON schema.
    #[test]
    fn search_emails_schema_hides_include_bodies() {
        let schema = search_emails::SearchEmailsTool.parameters_schema();
        let props = schema["properties"].as_object().expect("properties object");
        assert!(
            !props.contains_key("include_bodies"),
            "include_bodies must stay out of the LLM-facing schema; got: {:?}",
            props.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_emails_invalid_date_returns_error() {
        let db = tools_test_db();
        seed_email(&db, "e1", "acc", "t1", "A", "a@x.com", "s", "b", 1);
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "search_emails",
            &arg(serde_json::json!({ "from": "a@x.com", "since": "not-a-date" })),
        );
        assert!(out.starts_with("Error: invalid 'since' date"), "output was: {}", out);
    }

    // ── get_email_body ──────────────────────────────────────────────────

    #[test]
    fn get_email_body_returns_text() {
        let db = tools_test_db();
        seed_email(
            &db,
            "e1",
            "acc",
            "t1",
            "A",
            "a@x.com",
            "subj",
            "<p>hola <b>mundo</b></p>",
            1,
        );
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "get_email_body",
            &arg(serde_json::json!({ "email_id": "e1" })),
        );
        assert!(out.contains("hola"));
        assert!(out.contains("mundo"));
        assert!(!out.contains("<b>"), "HTML should be stripped; got: {}", out);
    }

    /// Regression: `get_email_body` used to clip the body at a hard 3000-char
    /// cap (truncate_chars), which sliced long emails mid-sentence. It now
    /// reuses `thread_clean::clean_email_body` with the single-email ceiling
    /// (`MAX_CHARS_PER_EMAIL` = 16000), matching the "chat about this email"
    /// path — so a 5000-char body comes back whole.
    #[test]
    fn get_email_body_returns_full_body_not_clipped_at_old_3000_cap() {
        let db = tools_test_db();
        // 5000 chars of unbroken plain text — beyond the old 3000 cap but under
        // the ceiling, so the cleaner returns it intact (no "…" marker).
        let long_body = "x".repeat(5000);
        seed_email(&db, "e1", "acc", "t1", "A", "a@x.com", "subj", &long_body, 1);
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "get_email_body",
            &arg(serde_json::json!({ "email_id": "e1" })),
        );
        assert!(
            out.chars().count() > 3000,
            "body must not be clipped at the old 3000 cap; got {} chars",
            out.chars().count()
        );
        assert!(
            !out.ends_with('…'),
            "5000-char body is under the ceiling — should not be truncated; out len {}",
            out.chars().count()
        );
    }

    /// Regression: a long newsletter (e.g. a 100 KB Substack issue that cleans
    /// to ~10 K chars of plain text) was clipped at the old 8000-char ceiling,
    /// so the model only saw the first half of the email. The ceiling is now
    /// 16000, so a 10 000-char body comes back whole.
    #[test]
    fn get_email_body_returns_long_newsletter_whole_past_old_8000_ceiling() {
        let db = tools_test_db();
        // 10 000 chars of unbroken plain text — past the old 8000 ceiling but
        // under the new 16000 one, so it must survive intact (no "…" marker).
        let long_body = "y".repeat(10_000);
        seed_email(&db, "e1", "acc", "t1", "A", "a@x.com", "subj", &long_body, 1);
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "get_email_body",
            &arg(serde_json::json!({ "email_id": "e1" })),
        );
        assert!(
            out.chars().count() >= 10_000,
            "10 000-char body must not be clipped at the old 8000 ceiling; got {} chars",
            out.chars().count()
        );
        assert!(
            !out.ends_with('…'),
            "10 000-char body is under the 16000 ceiling — should not be truncated; out len {}",
            out.chars().count()
        );
    }

    #[test]
    fn get_email_body_missing_id_returns_error() {
        let db = tools_test_db();
        let out = execute_tool(&db, "acc", &[], "get_email_body", &arg(serde_json::json!({})));
        assert!(out.starts_with("Error:"), "output was: {}", out);
    }

    #[test]
    fn get_email_body_unknown_email_returns_empty_notice() {
        let db = tools_test_db();
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "get_email_body",
            &arg(serde_json::json!({ "email_id": "does-not-exist" })),
        );
        assert!(
            out.contains("empty") || out.contains("not yet downloaded"),
            "output was: {}",
            out
        );
    }

    // ── get_thread ──────────────────────────────────────────────────────

    #[test]
    fn get_thread_returns_emails_in_chronological_order() {
        let db = tools_test_db();
        seed_email(&db, "a", "acc", "thread-1", "A", "a@x.com", "first msg", "body A", 100);
        seed_email(&db, "b", "acc", "thread-1", "B", "b@x.com", "reply", "body B", 200);
        seed_email(
            &db,
            "c",
            "acc",
            "other-thread",
            "C",
            "c@x.com",
            "unrelated",
            "body C",
            150,
        );

        let out = execute_tool(
            &db,
            "acc",
            &[],
            "get_thread",
            &arg(serde_json::json!({ "thread_id": "thread-1" })),
        );
        let a_pos = out.find("first msg").expect("thread missing first msg");
        let b_pos = out.find("reply").expect("thread missing reply");
        assert!(a_pos < b_pos, "earlier email should appear first; got: {}", out);
        assert!(!out.contains("unrelated"), "other thread leaked: {}", out);
    }

    /// Regression: `get_thread` used to clip each email at a hard 1500-char
    /// cap. It now reuses `thread_clean::clean_email_body` with the
    /// thread-aware budget (`chars_per_email(n)`), so a short thread keeps each
    /// message nearly whole instead of slicing it mid-sentence.
    #[test]
    fn get_thread_returns_full_bodies_not_clipped_at_old_1500_cap() {
        let db = tools_test_db();
        // A 2-email thread → chars_per_email(2) = 6000, so each 4000-char body
        // survives whole (well beyond the old 1500 cap).
        let body_a = "a".repeat(4000);
        let body_b = "b".repeat(4000);
        seed_email(&db, "ta", "acc", "thread-1", "A", "a@x.com", "first", &body_a, 100);
        seed_email(&db, "tb", "acc", "thread-1", "B", "b@x.com", "reply", &body_b, 200);
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "get_thread",
            &arg(serde_json::json!({ "thread_id": "thread-1" })),
        );
        assert!(
            out.contains(&body_a),
            "first email body clipped below 4000 chars; out len {}",
            out.len()
        );
        assert!(
            out.contains(&body_b),
            "second email body clipped below 4000 chars; out len {}",
            out.len()
        );
    }

    #[test]
    fn get_thread_missing_id_returns_error() {
        let db = tools_test_db();
        let out = execute_tool(&db, "acc", &[], "get_thread", &arg(serde_json::json!({})));
        assert!(out.starts_with("Error:"), "output was: {}", out);
    }

    // ── get_attachments ─────────────────────────────────────────────────

    #[test]
    fn get_attachments_no_files_returns_notice() {
        let db = tools_test_db();
        seed_email(&db, "e1", "acc", "t1", "A", "a@x.com", "s", "b", 1);
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "get_attachments",
            &arg(serde_json::json!({ "email_id": "e1" })),
        );
        assert!(out.contains("No attachments"), "output was: {}", out);
    }

    #[test]
    fn get_attachments_renders_meta_links() {
        let db = tools_test_db();
        seed_email(&db, "e1", "acc", "t1", "A", "a@x.com", "s", "b", 1);
        db.connection()
            .execute(
                "INSERT INTO email_attachment_meta (id, email_id, account_id, provider_attachment_id, filename, mime_type, file_size, file_path)
                 VALUES ('att-1', 'e1', 'acc', 'pid', 'invoice.pdf', 'application/pdf', 12345, NULL)",
                [],
            )
            .unwrap();
        let out = execute_tool(
            &db,
            "acc",
            &[],
            "get_attachments",
            &arg(serde_json::json!({ "email_id": "e1" })),
        );
        assert!(out.contains("invoice.pdf"), "output was: {}", out);
        assert!(out.contains("attachment://meta/att-1"), "output was: {}", out);
        assert!(out.contains("application/pdf"), "output was: {}", out);
    }

    // ── Unknown tool ────────────────────────────────────────────────────

    #[test]
    fn unknown_tool_returns_unknown_marker() {
        let db = tools_test_db();
        let out = execute_tool(&db, "acc", &[], "definitely_not_a_tool", &arg(serde_json::json!({})));
        assert!(out.starts_with("Unknown tool:"), "output was: {}", out);
    }

    #[test]
    fn registry_omits_memory_tools_when_memory_disabled() {
        use crate::services::chat::tools::default_registry;
        let db = tools_test_db();
        // memory_enabled defaults to false; the registry should hide every memory tool.
        let registry = default_registry();
        let names: Vec<&str> = registry
            .definitions(db.as_ref())
            .iter()
            .map(|d| d["function"]["name"].as_str().unwrap_or(""))
            .map(|s| {
                // Borrow extension: leak the &str so it lives as long as the test
                // (the JSON Values are owned by the Vec we just dropped).
                Box::leak(s.to_string().into_boxed_str()) as &str
            })
            .collect();
        for forbidden in ["memory_search", "recall_entity", "remember"] {
            assert!(
                !names.contains(&forbidden),
                "{} should be omitted when memory_enabled is false; got names={:?}",
                forbidden,
                names,
            );
        }
    }
}
