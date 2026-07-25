//! Static registry of user-editable prompts.
//!
//! Each `PromptDef` describes a prompt the user can override from Settings:
//! its id, label, the default template string, and the list of variables that
//! get substituted at render time. The id is also the suffix used to persist
//! the override in `user_preferences` (`prompt.<id>`).

use serde::Serialize;

use super::defaults;

#[derive(Debug, Clone, Copy)]
pub struct VariableDef {
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PromptCategory {
    Chat,
    Classification,
    Memory,
    Tasks,
    Translation,
}

#[derive(Debug, Clone, Copy)]
pub struct PromptDef {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub category: PromptCategory,
    /// Hide behind the "Show advanced prompts" toggle in the UI.
    pub advanced: bool,
    pub default_template: &'static str,
    pub variables: &'static [VariableDef],
}

// ── Variable definitions, grouped per prompt for clarity ────────────────────

const CLASSIFY_EMAIL_VARS: &[VariableDef] = &[
    VariableDef {
        name: "today",
        description: "Current date (UTC) as YYYY-MM-DD.",
    },
    VariableDef {
        name: "language_clause",
        description: "Output-language instruction (empty when no preference is set).",
    },
    VariableDef {
        name: "intents",
        description: "Comma-separated list of intents from your Classification settings.",
    },
    VariableDef {
        name: "topics",
        description: "Comma-separated list of topics from your Classification settings.",
    },
];

const MEMORY_TASKS_VARS: &[VariableDef] = &[
    VariableDef {
        name: "today",
        description: "Current date (UTC) as YYYY-MM-DD.",
    },
    VariableDef {
        name: "language_clause",
        description: "Output-language instruction (empty when no preference is set).",
    },
    VariableDef {
        name: "max_tasks_clause",
        description: "Auto-generated cap on tasks per email (from Memory settings).",
    },
    VariableDef {
        name: "dedup_block",
        description: "Existing open tasks for this thread that the model should not duplicate.",
    },
    VariableDef {
        name: "sender",
        description: "Email sender display name.",
    },
    VariableDef {
        name: "sender_email",
        description: "Email sender address.",
    },
    VariableDef {
        name: "subject",
        description: "Email subject line.",
    },
    VariableDef {
        name: "snippet",
        description: "Truncated email body (up to ~1500 chars).",
    },
];

const MEMORY_FACTS_VARS: &[VariableDef] = &[
    VariableDef {
        name: "today",
        description: "Current date (UTC) as YYYY-MM-DD.",
    },
    VariableDef {
        name: "language_clause",
        description: "Output-language instruction (empty when no preference is set).",
    },
    VariableDef {
        name: "sender",
        description: "Email sender display name.",
    },
    VariableDef {
        name: "sender_email",
        description: "Email sender address.",
    },
    VariableDef {
        name: "subject",
        description: "Email subject line.",
    },
    VariableDef {
        name: "snippet",
        description: "Truncated email body (up to ~1500 chars).",
    },
];

const CHAT_SYSTEM_VARS: &[VariableDef] = &[
    VariableDef {
        name: "today",
        description: "Current date (UTC) as YYYY-MM-DD.",
    },
    VariableDef {
        name: "tomorrow",
        description: "Tomorrow's date (UTC) as YYYY-MM-DD — used in `since/until` examples.",
    },
    VariableDef {
        name: "language_instruction",
        description: "Reply-language instruction (default: 'Reply in the language the user writes in.').",
    },
    VariableDef {
        name: "user_identity",
        description: "Active account address plus guidance for mapping first-person sender/recipient references (\"emails I sent\", \"sent to me\") onto search_emails' from/to filters. Blank when no account is on the turn.",
    },
    VariableDef {
        name: "tools_section",
        description: "`Tools:` section auto-generated from the registry; lists the tools the LLM may call this turn, honouring Settings feature flags.",
    },
];

const CHAT_QUERY_REWRITE_VARS: &[VariableDef] = &[VariableDef {
    name: "user_question",
    description: "The raw user question being rewritten for retrieval.",
}];

const CHAT_RERANK_VARS: &[VariableDef] = &[
    VariableDef {
        name: "user_question",
        description: "The raw user question.",
    },
    VariableDef {
        name: "candidates",
        description: "Numbered candidate list with subject + smart-snippet body slices.",
    },
];

const CHAT_QUERY_PLAN_VARS: &[VariableDef] = &[
    VariableDef {
        name: "user_email",
        description: "The active account's address — used to resolve first-person sender/recipient references into from/to filters.",
    },
    VariableDef {
        name: "today",
        description: "Current date (UTC) as YYYY-MM-DD, for resolving relative date ranges.",
    },
    VariableDef {
        name: "query",
        description: "The raw user question being planned into a search_emails filter.",
    },
    VariableDef {
        name: "this_week_since",
        description: "Monday of the current week (YYYY-MM-DD) — deterministic 'this week' range start.",
    },
    VariableDef {
        name: "this_week_until",
        description: "Next Monday (YYYY-MM-DD, end-exclusive) — deterministic 'this week' range end.",
    },
    VariableDef {
        name: "last_week_since",
        description: "Monday of the previous week (YYYY-MM-DD) — deterministic 'last week' range start.",
    },
    VariableDef {
        name: "last_week_until",
        description: "Monday of the current week (YYYY-MM-DD, end-exclusive) — deterministic 'last week' range end.",
    },
];

const TRANSLATE_DETECT_VARS: &[VariableDef] = &[VariableDef {
    name: "sample",
    description: "First ~400 characters of the email's plain text.",
}];

const TRANSLATE_EMAIL_VARS: &[VariableDef] = &[
    VariableDef {
        name: "target_language",
        description: "English name of the language to translate into (e.g. 'Spanish').",
    },
    VariableDef {
        name: "text",
        description: "Plain text of the email or draft being translated (truncated to fit the context window).",
    },
];

// ── Registry table ──────────────────────────────────────────────────────────

pub const PROMPTS: &[PromptDef] = &[
    PromptDef {
        id: "classify.email",
        label: "Email classification",
        description: "Used to assign intent / topic / urgency to each new email.",
        category: PromptCategory::Classification,
        advanced: false,
        default_template: defaults::CLASSIFY_EMAIL,
        variables: CLASSIFY_EMAIL_VARS,
    },
    PromptDef {
        id: "memory.extract_tasks",
        label: "Legacy — task extraction",
        description: "Legacy task prompt id kept for existing overrides.",
        category: PromptCategory::Memory,
        advanced: true,
        default_template: defaults::MEMORY_EXTRACT_TASKS,
        variables: MEMORY_TASKS_VARS,
    },
    PromptDef {
        id: "tasks.extract",
        label: "Tasks — extraction",
        description: "Pulls action items, commitments, and deadlines from each email.",
        category: PromptCategory::Tasks,
        advanced: false,
        default_template: defaults::MEMORY_EXTRACT_TASKS,
        variables: MEMORY_TASKS_VARS,
    },
    PromptDef {
        id: "memory.extract_facts",
        label: "Memory — fact extraction",
        description: "Pulls durable facts and preferences from each email.",
        category: PromptCategory::Memory,
        advanced: false,
        default_template: defaults::MEMORY_EXTRACT_FACTS,
        variables: MEMORY_FACTS_VARS,
    },
    PromptDef {
        id: "chat.system",
        label: "Chat — system prompt",
        description: "Top-level instructions and tool descriptions for the chat assistant.",
        category: PromptCategory::Chat,
        advanced: false,
        default_template: defaults::CHAT_SYSTEM,
        variables: CHAT_SYSTEM_VARS,
    },
    PromptDef {
        id: "chat.query_rewrite",
        label: "Chat — query rewrite (HyDE)",
        description: "Internal reformulation step that expands the user's question before retrieval.",
        category: PromptCategory::Chat,
        advanced: true,
        default_template: defaults::CHAT_QUERY_REWRITE,
        variables: CHAT_QUERY_REWRITE_VARS,
    },
    PromptDef {
        id: "chat.rerank",
        label: "Chat — result reranker",
        description: "Internal step that re-scores retrieval candidates by relevance.",
        category: PromptCategory::Chat,
        advanced: true,
        default_template: defaults::CHAT_RERANK,
        variables: CHAT_RERANK_VARS,
    },
    PromptDef {
        id: "translate.detect_language",
        label: "Translation — language detection",
        description: "Internal step that identifies an email's language to decide whether to offer translation.",
        category: PromptCategory::Translation,
        advanced: true,
        default_template: defaults::TRANSLATE_DETECT_LANGUAGE,
        variables: TRANSLATE_DETECT_VARS,
    },
    PromptDef {
        id: "translate.email",
        label: "Translation — email / draft",
        description: "Translates an email body or compose draft into the requested language.",
        category: PromptCategory::Translation,
        advanced: false,
        default_template: defaults::TRANSLATE_EMAIL,
        variables: TRANSLATE_EMAIL_VARS,
    },
    PromptDef {
        id: "chat.query_plan",
        label: "Chat — query planner",
        description: "Internal fast-path that turns a tools-first question into a single search_emails filter (or defers) before the model round.",
        category: PromptCategory::Chat,
        advanced: true,
        default_template: defaults::CHAT_QUERY_PLAN,
        variables: CHAT_QUERY_PLAN_VARS,
    },
];

pub fn lookup(id: &str) -> Option<&'static PromptDef> {
    PROMPTS.iter().find(|p| p.id == id)
}
