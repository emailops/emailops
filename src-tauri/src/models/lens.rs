//! Type definitions for the Lenses feature. See `docs/lenses-prd.md`.

use serde::{Deserialize, Serialize};

// ── Schema ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensSchema {
    pub columns: Vec<LensColumn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensColumn {
    /// snake_case stable identifier used as the key in extracted_json.
    pub key: String,
    /// Human-friendly column header.
    pub label: String,
    #[serde(rename = "type")]
    pub column_type: LensColumnType,
    /// Description fed to the extraction prompt to guide the LLM.
    #[serde(default)]
    pub description: String,
    /// Allowed values when `column_type == Enum`. Ignored otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    /// When false, the model may return `null` for this column.
    #[serde(default)]
    pub required: bool,
    /// When true, rows are deduplicated by this column's value in the
    /// spreadsheet view — only the most recent email per unique value is shown.
    /// At most one column per schema should have this set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_unique_key: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensColumnType {
    String,
    /// Long string — rendered as multi-line in UI.
    Text,
    Number,
    /// `{ amount: number, currency: string }`.
    Currency,
    /// ISO 8601 date.
    Date,
    Boolean,
    /// Requires `enum_values`.
    Enum,
    Email,
    Url,
}

// ── Scope ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensScope {
    /// `None` = all accounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ids: Option<Vec<String>>,
    /// e.g. `["inbox", "sent"]`. `None` = all mailboxes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailboxes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<TagFilter>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_domains: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_emails: Option<Vec<String>>,
    /// FTS5 query (applied to `emails_fts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// When `true`, the keyword query searches subject + sender + body.
    /// When `false` (the default), it searches subject only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub query_search_body: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_range: Option<DateRange>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
    Either,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagFilter {
    pub tag_type: String,
    pub tag_value: String,
}

/// Either a relative window expressed in days, or an absolute unix-seconds range.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DateRange {
    /// "Last N days" — when set, takes precedence over `from`/`to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<i64>,
}

// ── Lens record ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lens {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub template_key: Option<String>,
    /// `None` = applies to all accounts.
    pub account_id: Option<String>,
    pub scope: LensScope,
    pub schema: LensSchema,
    pub prompt_text: String,
    pub prompt_version: i64,
    pub model_provider: Option<String>,
    pub model_name: Option<String>,
    pub is_enabled: bool,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Compact list-view summary returned by `list_lenses`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensSummary {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub template_key: Option<String>,
    pub account_id: Option<String>,
    pub is_enabled: bool,
    pub sort_order: i64,
    pub row_count: i64,
    pub prompt_version: i64,
}

// ── Rows ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensRow {
    pub lens_id: String,
    pub email_id: String,
    pub account_id: String,
    /// Extracted JSON merged with `overrides_json` (overrides win).
    pub data: serde_json::Value,
    /// True when at least one cell in `data` was user-edited.
    pub has_overrides: bool,
    pub prompt_version: i64,
    pub email_timestamp: i64,
    pub extracted_at: i64,
    /// `'ok' | 'failed' | 'excluded'`.
    pub status: String,
    pub error_message: Option<String>,
    /// Email metadata denormalized for the leading "Email" column in the UI.
    pub email_subject: String,
    pub email_sender: String,
    pub email_sender_email: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensRowsPage {
    pub rows: Vec<LensRow>,
    /// Total number of rows for this Lens (across pages). `-1` if not computed.
    pub total: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortSpec {
    /// Column key. Special values: `"emailTimestamp"` (default).
    pub key: String,
    /// `true` = descending.
    #[serde(default)]
    pub desc: bool,
}

// ── Run kinds / status ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensRunKind {
    Backfill,
    Incremental,
    Reextract,
    Single,
}

impl LensRunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LensRunKind::Backfill => "backfill",
            LensRunKind::Incremental => "incremental",
            LensRunKind::Reextract => "reextract",
            LensRunKind::Single => "single",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensStatus {
    pub lens_id: String,
    /// `'idle' | 'running' | 'error'`.
    pub state: String,
    pub current_run_id: Option<String>,
    pub current_run_kind: Option<String>,
    pub processed: i64,
    pub total: i64,
    pub succeeded: i64,
    pub failed: i64,
    /// Number of rows where `prompt_version < lens.prompt_version`.
    pub pending_reextract: i64,
    pub last_error: Option<String>,
}

/// Returned by `run_lens` — the frontend tracks progress via `app-log` events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensRunHandle {
    pub run_id: String,
    pub lens_id: String,
}

/// One row from `lens_runs`, surfaced in the run-history dropdown.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensRunHistoryEntry {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub processed: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub error_message: Option<String>,
}

// ── Inputs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLensInput {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub template_key: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    pub scope: LensScope,
    pub schema: LensSchema,
    pub prompt_text: String,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLensInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub account_id: Option<Option<String>>,
    #[serde(default)]
    pub scope: Option<LensScope>,
    #[serde(default)]
    pub schema: Option<LensSchema>,
    #[serde(default)]
    pub prompt_text: Option<String>,
    #[serde(default)]
    pub model_provider: Option<Option<String>>,
    #[serde(default)]
    pub model_name: Option<Option<String>>,
    #[serde(default)]
    pub is_enabled: Option<bool>,
    #[serde(default)]
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRow {
    pub email_id: String,
    pub email_subject: String,
    pub email_sender: String,
    pub data: serde_json::Value,
    pub status: String,
    pub error_message: Option<String>,
}
